use crate::mmap::{WfmFile};
use crate::structs::{WfmHeader1000Z, WfmHeader1000E, TektronixHeader};
use numpy::{PyArray1, PyArrayMethods};
use pyo3::prelude::*;

pub struct Parser;

impl Parser {
    pub fn get_channel_data_tektronix<'py>(
        py: Python<'py>,
        wfm: &WfmFile,
        header: &TektronixHeader,
        channel_idx: usize,
    ) -> PyResult<Bound<'py, PyArray1<f32>>> {
        if channel_idx > 0 {
            // Tektronix WFM typically contains only one waveform per file
            return Err(pyo3::exceptions::PyValueError::new_err("Tektronix WFM typically contains only 1 channel"));
        }

        let base_start = header.static_info.byte_offset_to_curve_buffer as usize;
        let data_start = base_start + header.data_start_offset as usize;
        let data_end = base_start + header.postcharge_start_offset as usize;
        let bpp = header.static_info.num_bytes_per_point as usize;

        if data_end > wfm.mmap.len() || data_start >= data_end {
            return Err(pyo3::exceptions::PyValueError::new_err("Invalid curve buffer offsets"));
        }

        let raw_data = &wfm.mmap[data_start..data_end];
        let points = raw_data.len() / bpp;

        let y_scale = header.y_scale as f32;
        let y_offset = header.y_offset as f32;
        let is_le = header.static_info.byte_order == 0x0f0f;

        let array = unsafe { PyArray1::new(py, [points], false) };
        {
            let array_slice = unsafe { array.as_slice_mut()? };
            if bpp == 1 {
                for i in 0..points {
                    let raw_val = raw_data[i] as i8 as f32; // Assuming signed 8-bit for Tek
                    array_slice[i] = raw_val * y_scale + y_offset;
                }
            } else if bpp == 2 {
                for i in 0..points {
                    let raw_val = if is_le {
                        i16::from_le_bytes([raw_data[i * 2], raw_data[i * 2 + 1]]) as f32
                    } else {
                        i16::from_be_bytes([raw_data[i * 2], raw_data[i * 2 + 1]]) as f32
                    };
                    array_slice[i] = raw_val * y_scale + y_offset;
                }
            } else if bpp == 4 {
                // Often FP32 or INT32, we assume FP32 given typical '002'/'003' files if scale is 1.0, but let's stick to safe fallback.
                for i in 0..points {
                    let raw_val = if is_le {
                        i32::from_le_bytes([raw_data[i * 4], raw_data[i * 4 + 1], raw_data[i * 4 + 2], raw_data[i * 4 + 3]]) as f32
                    } else {
                        i32::from_be_bytes([raw_data[i * 4], raw_data[i * 4 + 1], raw_data[i * 4 + 2], raw_data[i * 4 + 3]]) as f32
                    };
                    array_slice[i] = raw_val * y_scale + y_offset;
                }
            } else {
                return Err(pyo3::exceptions::PyValueError::new_err(format!("Unsupported bytes per point: {}", bpp)));
            }
        }
        Ok(array)
    }

    pub fn get_channel_data_1000z<'py>(
        py: Python<'py>,
        wfm: &WfmFile,
        header: &WfmHeader1000Z,
        channel_idx: usize,
    ) -> PyResult<Bound<'py, PyArray1<f32>>> {
        let channel = &header.channels[channel_idx];
        if channel.enabled_val == 0 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!("Channel {} is not enabled", channel_idx + 1)));
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

        let array = unsafe { PyArray1::new(py, [points], false) };
        {
            let array_slice = unsafe { array.as_slice_mut()? };
            for i in 0..points {
                let raw_byte = raw_data[i * stride + offset] as f32;
                array_slice[i] = y_scale * (midpoint - raw_byte) - y_offset;
            }
        }
        Ok(array)
    }

    pub fn get_channel_data_1000e<'py>(
        py: Python<'py>,
        wfm: &WfmFile,
        header: &WfmHeader1000E,
        channel_idx: usize,
    ) -> PyResult<Bound<'py, PyArray1<f32>>> {
        if channel_idx > 1 {
            return Err(pyo3::exceptions::PyValueError::new_err("DS1000E only has 2 channels"));
        }
        
        let ch1_enabled = header.channels[0].enabled_val != 0;
        let ch2_enabled = header.channels[1].enabled_val != 0;
        
        let is_enabled = if channel_idx == 0 { ch1_enabled } else { ch2_enabled };
        if !is_enabled {
            return Err(pyo3::exceptions::PyValueError::new_err(format!("Channel {} is not enabled", channel_idx + 1)));
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

        let array = unsafe { PyArray1::new(py, [points], false) };
        {
            let array_slice = unsafe { array.as_slice_mut()? };
            for i in 0..points {
                let raw_byte = raw_data[i] as f32;
                array_slice[i] = y_scale * (midpoint - raw_byte) - y_offset;
            }
        }
        Ok(array)
    }
}
