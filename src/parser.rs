use crate::mmap::{WfmFile, WfmHeader};
use crate::structs::{WfmHeader1000Z, WfmHeader1000E, WfmHeader2000, TektronixHeader, IsfHeader};
use rayon::prelude::*;

pub struct Parser;

impl Parser {
    pub fn get_all_channels(wfm: &WfmFile) -> anyhow::Result<Vec<Option<Vec<f32>>>> {
        match &wfm.wfm_header {
            WfmHeader::Ds1000z(header) => {
                let results: Vec<_> = (0..4).into_par_iter().map(|ch_idx| {
                    Self::get_channel_data_1000z(wfm, header, ch_idx).ok()
                }).collect();
                Ok(results)
            },
            WfmHeader::Ds1000e(header) => {
                let results: Vec<_> = (0..2).into_par_iter().map(|ch_idx| {
                    Self::get_channel_data_1000e(wfm, header, ch_idx).ok()
                }).collect();
                Ok(results)
            },
            WfmHeader::Ds2000(header) => {
                let results: Vec<_> = (0..4).into_par_iter().map(|ch_idx| {
                    Self::get_channel_data_2000(wfm, header, ch_idx).ok()
                }).collect();
                Ok(results)
            },
            WfmHeader::Tektronix(header) => {
                Ok(vec![Self::get_channel_data_tektronix(wfm, header, 0).ok()])
            },
            WfmHeader::Isf(header) => {
                Ok(vec![Self::get_channel_data_isf(wfm, header, 0).ok()])
            }
        }
    }

    pub fn get_channel_data_isf(
        wfm: &WfmFile,
        header: &IsfHeader,
        channel_idx: usize,
    ) -> anyhow::Result<Vec<f32>> {
        if channel_idx > 0 {
            return Err(anyhow::anyhow!("ISF files typically contain only 1 channel"));
        }

        let raw_data = &wfm.mmap[header.data_offset..];
        let points = header.nr_pt as usize;
        let bpp = header.byt_nr as usize;

        if points * bpp > raw_data.len() {
            // Some ISF files report larger NR_PT than actual data length
            // We should use the actual available length if it's smaller, but here we just bound it
        }
        
        let actual_points = std::cmp::min(points, raw_data.len() / bpp);

        let is_le = header.byt_or == "LSB";
        let y_scale = header.ymult;
        let y_offset = header.yzero;
        let y_adc_offset = header.yoff;

        let voltages: Vec<f32> = (0..actual_points).into_par_iter().map(|i| {
            let raw_val = if bpp == 1 {
                raw_data[i] as i8 as f32
            } else {
                if is_le {
                    i16::from_le_bytes([raw_data[i * 2], raw_data[i * 2 + 1]]) as f32
                } else {
                    i16::from_be_bytes([raw_data[i * 2], raw_data[i * 2 + 1]]) as f32
                }
            };
            y_offset + y_scale * (raw_val - y_adc_offset)
        }).collect();

        Ok(voltages)
    }

    pub fn get_channel_data_2000(
        wfm: &WfmFile,
        header: &WfmHeader2000,
        channel_idx: usize,
    ) -> anyhow::Result<Vec<f32>> {
        if channel_idx > 3 {
            return Err(anyhow::anyhow!("Channel must be between 1 and 4"));
        }
        
        if !header.is_ch_enabled(channel_idx) {
            return Err(anyhow::anyhow!("Channel {} is not enabled", channel_idx + 1));
        }

        let channel = &header.channels[channel_idx];
        let points = header.wfm_len as usize;
        let y_scale = channel.volt_scale();
        let y_offset = channel.volt_offset;
        let midpoint = 127.0f32;
        
        if header.interwoven() {
            let half_points = header.raw_depth();
            let offset_a = (header.channel_offsets[0] + header.z_pt_offset) as usize;
            let offset_b = (header.channel_offsets[1] + header.z_pt_offset) as usize;
            
            if offset_a + half_points > wfm.mmap.len() || offset_b + half_points > wfm.mmap.len() {
                return Err(anyhow::anyhow!("Invalid channel data offset (interwoven)"));
            }
            
            let raw_a = &wfm.mmap[offset_a..offset_a + half_points];
            let raw_b = &wfm.mmap[offset_b..offset_b + half_points];
            
            // Parallelize the interwoven reconstruction
            let voltages: Vec<f32> = (0..points).into_par_iter().map(|i| {
                let raw_byte = if i % 2 == 0 {
                    raw_a[i / 2]
                } else {
                    raw_b[i / 2]
                };
                y_scale * (raw_byte as f32 - midpoint) - y_offset
            }).collect();
            
            return Ok(voltages);
        }

        let data_start = (header.channel_offsets[channel_idx] + header.z_pt_offset) as usize;
        if data_start + points > wfm.mmap.len() {
            return Err(anyhow::anyhow!("Invalid channel data offset"));
        }
        
        let raw_data = &wfm.mmap[data_start..data_start + points];

        // Parallel map for contiguous data
        let voltages: Vec<f32> = raw_data.par_iter().map(|&raw_byte| {
            y_scale * (raw_byte as f32 - midpoint) - y_offset
        }).collect();

        Ok(voltages)
    }

    pub fn get_channel_data_tektronix(
        wfm: &WfmFile,
        header: &TektronixHeader,
        channel_idx: usize,
    ) -> anyhow::Result<Vec<f32>> {
        if channel_idx > 0 {
            return Err(anyhow::anyhow!("Tektronix WFM typically contains only 1 channel"));
        }

        let base_start = header.static_info.byte_offset_to_curve_buffer as usize;
        let data_start = base_start + header.data_start_offset as usize;
        let data_end = base_start + header.postcharge_start_offset as usize;
        let bpp = header.static_info.num_bytes_per_point as usize;

        if data_end > wfm.mmap.len() || data_start >= data_end {
            return Err(anyhow::anyhow!("Invalid curve buffer offsets"));
        }

        let raw_data = &wfm.mmap[data_start..data_end];
        let points = raw_data.len() / bpp;

        let y_scale = header.y_scale as f32;
        let y_offset = header.y_offset as f32;
        let is_le = header.static_info.byte_order == 0x0f0f;

        let voltages: Vec<f32> = (0..points).into_par_iter().map(|i| {
            let raw_val = if bpp == 1 {
                raw_data[i] as i8 as f32
            } else if bpp == 2 {
                if is_le {
                    i16::from_le_bytes([raw_data[i * 2], raw_data[i * 2 + 1]]) as f32
                } else {
                    i16::from_be_bytes([raw_data[i * 2], raw_data[i * 2 + 1]]) as f32
                }
            } else {
                if is_le {
                    i32::from_le_bytes([raw_data[i * 4], raw_data[i * 4 + 1], raw_data[i * 4 + 2], raw_data[i * 4 + 3]]) as f32
                } else {
                    i32::from_be_bytes([raw_data[i * 4], raw_data[i * 4 + 1], raw_data[i * 4 + 2], raw_data[i * 4 + 3]]) as f32
                }
            };
            raw_val * y_scale + y_offset
        }).collect();

        Ok(voltages)
    }

    pub fn get_channel_data_1000z(
        wfm: &WfmFile,
        header: &WfmHeader1000Z,
        channel_idx: usize,
    ) -> anyhow::Result<Vec<f32>> {
        let channel = &header.channels[channel_idx];
        if channel.enabled_val == 0 {
            return Err(anyhow::anyhow!("Channel {} is not enabled", channel_idx + 1));
        }

        let stride = header.stride();
        let points = header.points() as usize;
        
        let offset = if stride == 1 {
            0
        } else if stride == 2 {
            let enabled_before = (0..channel_idx).filter(|&i| header.is_ch_enabled(i)).count();
            if enabled_before == 0 { 1 } else { 0 }
        } else if stride == 4 {
            4 - (channel_idx + 1)
        } else {
            0
        };

        let data_start = (header.horizontal_offset + header.horizontal_size) as usize;
        let raw_data = &wfm.mmap[data_start..];
        
        let volt_per_div = if channel.inverted_val != 0 { -channel.scale } else { channel.scale };
        let vertical_bias = if wfm.firmware_version == "00.04.04.SP3" && header.enabled_channels_count() == 2 {
            if channel.shift < 0.0 { volt_per_div / 5.0 } else { 0.0 }
        } else {
            volt_per_div
        };
        
        let y_scale = -volt_per_div / 20.0;
        let y_offset = channel.shift - vertical_bias;
        let midpoint = 127.0f32;

        let voltages: Vec<f32> = (0..points).into_par_iter().map(|i| {
            let raw_byte = raw_data[i * stride + offset] as f32;
            y_scale * (midpoint - raw_byte) - y_offset
        }).collect();

        Ok(voltages)
    }

    pub fn get_channel_data_1000e(
        wfm: &WfmFile,
        header: &WfmHeader1000E,
        channel_idx: usize,
    ) -> anyhow::Result<Vec<f32>> {
        if channel_idx > 1 {
            return Err(anyhow::anyhow!("DS1000E only has 2 channels"));
        }
        
        let ch1_enabled = header.channels[0].enabled_val != 0;
        let ch2_enabled = header.channels[1].enabled_val != 0;
        
        let is_enabled = if channel_idx == 0 { ch1_enabled } else { ch2_enabled };
        if !is_enabled {
            return Err(anyhow::anyhow!("Channel {} is not enabled", channel_idx + 1));
        }

        let channel = &header.channels[channel_idx];
        let points = if channel_idx == 0 { header.ch1_points() } else { header.ch2_points() };
        
        let data_start = 276;
        let ch1_total = if ch1_enabled { header.ch1_points() + header.ch1_skip() } else { 0 };
        let offset = if channel_idx == 0 { 0 } else { ch1_total };

        let raw_data = &wfm.mmap[data_start + offset..];

        let volt_per_div = (channel.scale_measured as f32 / 1_000_000.0) * channel.probe_value;
        let volt_per_div = if channel.inverted_m_val != 0 { -volt_per_div } else { volt_per_div };
        
        let y_scale = volt_per_div / 25.0;
        let y_offset = (channel.shift_measured as f32) * (volt_per_div / 25.0);
        let midpoint = 125.0f32;

        let voltages: Vec<f32> = raw_data[..points].par_iter().map(|&raw_byte| {
            y_scale * (midpoint - raw_byte as f32) - y_offset
        }).collect();

        Ok(voltages)
    }
}
