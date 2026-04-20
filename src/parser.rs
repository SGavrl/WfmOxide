use crate::mmap::{WfmFile};
use crate::structs::{WfmHeader1000Z, WfmHeader1000E};
use numpy::{PyArray1, PyArrayMethods};
use pyo3::prelude::*;

pub struct Parser;

impl Parser {
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
        
        let ch1_enabled = (header.active_channel >> 0) & 1 != 0;
        let ch2_enabled = (header.active_channel >> 1) & 1 != 0;
        
        let is_enabled = if channel_idx == 0 { ch1_enabled } else { ch2_enabled };
        if !is_enabled {
            return Err(pyo3::exceptions::PyValueError::new_err(format!("Channel {} is not enabled", channel_idx + 1)));
        }

        let channel = &header.channels[channel_idx];
        let points = if channel_idx == 0 { header.ch1_points() } else { header.ch2_points() };
        
        let data_start = 272; // Adjusted from 276
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
                array_slice[i] = y_scale * (raw_byte - midpoint) - y_offset;
            }
        }
        Ok(array)
    }
}
