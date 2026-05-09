use crate::dho::DhoHeader;
use crate::mmap::{WfmFile, WfmHeader};
use crate::sample::{decode_with, Affine, SampleType};
use crate::structs::{IsfHeader, TektronixHeader, WfmHeader1000E, WfmHeader1000Z, WfmHeader2000, WfmHeader4000};
use rayon::prelude::*;

pub struct Parser;

impl Parser {

    pub fn apply_slice(total_points: usize, start_idx: Option<usize>, length: Option<usize>) -> (usize, usize) {
        let start = start_idx.unwrap_or(0).min(total_points);
        let rem = total_points - start;
        let len = length.unwrap_or(rem).min(rem);
        (start, len)
    }

    pub fn get_all_channels(wfm: &WfmFile, start_idx: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<Option<Vec<f32>>>> {
        match &wfm.wfm_header {
            WfmHeader::Ds1000z(header) => {
                let results: Vec<_> = (0..4).into_par_iter().map(|ch_idx| {
                    Self::get_channel_data_1000z(wfm, header, ch_idx, start_idx, length).ok()
                }).collect();
                Ok(results)
            },
            WfmHeader::Ds1000e(header) => {
                let results: Vec<_> = (0..2).into_par_iter().map(|ch_idx| {
                    Self::get_channel_data_1000e(wfm, header, ch_idx, start_idx, length).ok()
                }).collect();
                Ok(results)
            },
            WfmHeader::Ds2000(header) => {
                let results: Vec<_> = (0..4).into_par_iter().map(|ch_idx| {
                    Self::get_channel_data_2000(wfm, header, ch_idx, start_idx, length).ok()
                }).collect();
                Ok(results)
            },
            WfmHeader::Ds4000(header) => {
                let results: Vec<_> = (0..4).into_par_iter().map(|ch_idx| {
                    Self::get_channel_data_4000(wfm, header, ch_idx, start_idx, length).ok()
                }).collect();
                Ok(results)
            },
            WfmHeader::Tektronix(header) => {
                Ok(vec![Self::get_channel_data_tektronix(wfm, header, 0, start_idx, length).ok()])
            },
            WfmHeader::Isf(header) => {
                Ok(vec![Self::get_channel_data_isf(wfm, header, 0, start_idx, length).ok()])
            },
            WfmHeader::Dho(header) => {
                let results: Vec<_> = (0..4).into_par_iter().map(|ch_idx| {
                    Self::get_channel_data_dho(wfm, header, ch_idx, start_idx, length).ok()
                }).collect();
                Ok(results)
            }
        }
    }

    pub fn get_channel_data_dho(wfm: &WfmFile, header: &DhoHeader, channel_idx: usize, start_idx: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<f32>> {
        if channel_idx > 3 {
            return Err(anyhow::anyhow!("Channel must be between 1 and 4"));
        }
        let transform = header.channel_cals[channel_idx]
            .ok_or_else(|| anyhow::anyhow!("Channel {} is not enabled", channel_idx + 1))?;

        let n_ch = header.n_ch;
        let n_pts = header.n_pts_per_ch;
        let total_bytes = n_pts * n_ch * 2;
        if header.data_start + total_bytes > wfm.mmap.len() {
            return Err(anyhow::anyhow!("DHO data section overruns mmap"));
        }
        let raw_data = &wfm.mmap[header.data_start..header.data_start + total_bytes];

        let (start_pt, slice_len) = Self::apply_slice(n_pts, start_idx, length);
        Ok(decode_with(
            raw_data,
            slice_len,
            SampleType::U16Le,
            transform,
            move |i| (start_pt + i) * n_ch + channel_idx,
        ))
    }


    pub fn get_channel_data_4000(wfm: &WfmFile, header: &WfmHeader4000, channel_idx: usize, start_idx: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<f32>> {
        if channel_idx > 3 {
            return Err(anyhow::anyhow!("Channel must be between 1 and 4"));
        }
        if !header.is_ch_enabled(channel_idx) {
            return Err(anyhow::anyhow!("Channel {} is not enabled", channel_idx + 1));
        }

        let channel = &header.channels[channel_idx];
        let points = header.mem_depth as usize;
        let data_start = header.channel_offsets[channel_idx] as usize;
        if data_start + points > wfm.mmap.len() {
            return Err(anyhow::anyhow!("Invalid channel data offset"));
        }

        let volt_div = if wfm.model_number.chars().nth(2) == Some('2') { 25.0 } else { 32.0 };
        let y_scale = channel.volt_signed() / volt_div;
        let y_offset = channel.volt_offset;
        let midpoint = 127.0f32;

        let (start_pt, slice_len) = Self::apply_slice(points, start_idx, length);
        let transform = Affine {
            scale: y_scale,
            offset: -y_scale * midpoint - y_offset,
        };

        Ok(decode_with(
            &wfm.mmap[data_start..data_start + points],
            slice_len,
            SampleType::U8,
            transform,
            |i| start_pt + i,
        ))
    }

    pub fn get_channel_data_isf(wfm: &WfmFile, header: &IsfHeader, channel_idx: usize, start_idx: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<f32>> {
        if channel_idx > 0 {
            return Err(anyhow::anyhow!("ISF files typically contain only 1 channel"));
        }

        let raw_data = &wfm.mmap[header.data_offset..];
        let points = header.nr_pt as usize;
        let bpp = header.byt_nr as usize;
        let actual_points = std::cmp::min(points, raw_data.len() / bpp);
        let (start_pt, slice_len) = Self::apply_slice(actual_points, start_idx, length);

        let is_le = header.byt_or == "LSB";
        let ty = match (bpp, is_le) {
            (1, _) => SampleType::I8,
            (2, true) => SampleType::I16Le,
            (2, false) => SampleType::I16Be,
            other => return Err(anyhow::anyhow!("Unsupported ISF byte width/order: {:?}", other)),
        };

        let y_scale = header.ymult;
        let transform = Affine {
            scale: y_scale,
            offset: header.yzero - y_scale * header.yoff,
        };

        Ok(decode_with(raw_data, slice_len, ty, transform, |i| start_pt + i))
    }

    pub fn get_channel_data_2000(wfm: &WfmFile, header: &WfmHeader2000, channel_idx: usize, start_idx: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<f32>> {
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
        let transform = Affine {
            scale: y_scale,
            offset: -y_scale * midpoint - y_offset,
        };
        let (start_pt, slice_len) = Self::apply_slice(points, start_idx, length);

        if header.interwoven() {
            let half_points = header.raw_depth();
            let offset_a = (header.channel_offsets[0] + header.z_pt_offset) as usize;
            let offset_b = (header.channel_offsets[1] + header.z_pt_offset) as usize;
            if offset_a + half_points > wfm.mmap.len() || offset_b + half_points > wfm.mmap.len() {
                return Err(anyhow::anyhow!("Invalid channel data offset (interwoven)"));
            }

            let raw_a = &wfm.mmap[offset_a..offset_a + half_points];
            let raw_b = &wfm.mmap[offset_b..offset_b + half_points];

            let voltages: Vec<f32> = (0..slice_len).into_par_iter().map(|idx| {
                let i = start_pt + idx;
                let raw_byte = if i % 2 == 0 { raw_a[i / 2] } else { raw_b[i / 2] };
                transform.apply(raw_byte as f32)
            }).collect();
            return Ok(voltages);
        }

        let data_start = (header.channel_offsets[channel_idx] + header.z_pt_offset) as usize;
        if data_start + points > wfm.mmap.len() {
            return Err(anyhow::anyhow!("Invalid channel data offset"));
        }

        Ok(decode_with(
            &wfm.mmap[data_start..data_start + points],
            slice_len,
            SampleType::U8,
            transform,
            |i| start_pt + i,
        ))
    }

    pub fn get_channel_data_tektronix(wfm: &WfmFile, header: &TektronixHeader, channel_idx: usize, start_idx: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<f32>> {
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
        let is_le = header.static_info.byte_order == 0x0f0f;
        let ty = match (bpp, is_le) {
            (1, _) => SampleType::I8,
            (2, true) => SampleType::I16Le,
            (2, false) => SampleType::I16Be,
            (4, true) => SampleType::I32Le,
            (4, false) => SampleType::I32Be,
            other => return Err(anyhow::anyhow!("Unsupported Tektronix byte width/order: {:?}", other)),
        };

        let (start_pt, slice_len) = Self::apply_slice(points, start_idx, length);
        let transform = Affine {
            scale: header.y_scale as f32,
            offset: header.y_offset as f32,
        };

        Ok(decode_with(raw_data, slice_len, ty, transform, |i| start_pt + i))
    }

    pub fn get_channel_data_1000z(wfm: &WfmFile, header: &WfmHeader1000Z, channel_idx: usize, start_idx: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<f32>> {
        if channel_idx > 3 {
            return Err(anyhow::anyhow!("Channel must be between 1 and 4"));
        }
        let channel = &header.channels[channel_idx];
        if channel.enabled_val == 0 {
            return Err(anyhow::anyhow!("Channel {} is not enabled", channel_idx + 1));
        }

        let stride = header.stride();
        let points = header.points() as usize;

        let chan_offset_in_stride = if stride == 1 {
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
        let transform = Affine {
            scale: -y_scale,
            offset: y_scale * midpoint - y_offset,
        };

        let (start_pt, slice_len) = Self::apply_slice(points, start_idx, length);
        Ok(decode_with(
            raw_data,
            slice_len,
            SampleType::U8,
            transform,
            move |i| (start_pt + i) * stride + chan_offset_in_stride,
        ))
    }

    pub fn get_channel_data_1000e(wfm: &WfmFile, header: &WfmHeader1000E, channel_idx: usize, start_idx: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<f32>> {
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
        let chan_offset_bytes = if channel_idx == 0 { 0 } else { ch1_total };

        let volt_per_div = (channel.scale_measured as f32 / 1_000_000.0) * channel.probe_value;
        let volt_per_div = if channel.inverted_m_val != 0 { -volt_per_div } else { volt_per_div };

        let y_scale = volt_per_div / 25.0;
        let y_offset = (channel.shift_measured as f32) * (volt_per_div / 25.0);
        let midpoint = 125.0f32;
        let transform = Affine {
            scale: -y_scale,
            offset: y_scale * midpoint - y_offset,
        };

        let (start_pt, slice_len) = Self::apply_slice(points, start_idx, length);
        Ok(decode_with(
            &wfm.mmap[data_start + chan_offset_bytes..],
            slice_len,
            SampleType::U8,
            transform,
            move |i| start_pt + i,
        ))
    }
}
