use std::fs::File;
use std::io::{Cursor, Seek, SeekFrom};
use memmap2::Mmap;
use binrw::{BinRead, Endian};
use crate::dho::{self, DhoHeader};
use crate::parser::Parser;
use crate::structs::{FileHeader, WfmHeader1000Z, WfmHeader1000E, WfmHeader2000, FileHeader2000, WfmHeader4000, TektronixStaticFileInfo, TektronixHeader, IsfHeader};

/// Time-axis metadata for a captured waveform. All fields are in seconds.
#[derive(Copy, Clone, Debug)]
pub struct TimeAxis {
    /// Time of sample 0, relative to the trigger.
    pub x_origin: f64,
    /// Seconds between consecutive samples.
    pub x_increment: f64,
}

impl TimeAxis {
    /// Sampling frequency in Hz.
    pub fn sample_rate(&self) -> f64 {
        if self.x_increment > 0.0 { 1.0 / self.x_increment } else { 0.0 }
    }
}

pub enum WfmHeader {
    Ds1000z(WfmHeader1000Z),
    Ds1000e(WfmHeader1000E),
    Ds2000(WfmHeader2000),
    Ds4000(WfmHeader4000),
    Tektronix(TektronixHeader),
    Isf(IsfHeader),
    Dho(DhoHeader),
}

pub struct WfmFile {
    pub mmap: Mmap,
    pub model_number: String,
    pub firmware_version: String,
    pub wfm_header: WfmHeader,
}

impl WfmFile {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        
        let mut is_isf = false;
        let limit = std::cmp::min(mmap.len(), 512);
        for i in 0..limit {
            if mmap[i..limit].starts_with(b":CURV") || mmap[i..limit].starts_with(b"BYT_N") {
                is_isf = true;
                break;
            }
        }
        
        if is_isf {
            let mut header_end = 0;
            for i in 0..mmap.len() {
                if mmap[i] == b'#' {
                    header_end = i;
                    break;
                }
            }
            if header_end == 0 {
                return Err(anyhow::anyhow!("Invalid ISF file: '#' not found"));
            }
            let header_text = String::from_utf8_lossy(&mmap[0..header_end]);
            
            let mut byt_nr = 2;
            let mut byt_or = "MSB".to_string();
            let mut nr_pt = 0;
            let mut ymult = 1.0;
            let mut yoff = 0.0;
            let mut yzero = 0.0;
            let mut xincr: f64 = 0.0;
            let mut xzero: f64 = 0.0;

            let parts = header_text.split(';');
            for part in parts {
                let part = part.trim();
                let part = part.strip_prefix(":WFMP:").unwrap_or(part);
                let part = part.strip_prefix(":CURVE:").unwrap_or(part);
                let part = part.strip_prefix(":CURV:").unwrap_or(part);
                let part = part.strip_prefix(":").unwrap_or(part);

                let mut kv = part.splitn(2, char::is_whitespace);
                if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                    let k = k.trim().to_uppercase();
                    let v = v.trim().trim_matches('"');
                    match k.as_str() {
                        "BYT_NR" | "BYT_N" => byt_nr = v.parse().unwrap_or(2),
                        "BYT_OR" | "BYT_O" => byt_or = v.to_string(),
                        "NR_PT" | "NR_P" => nr_pt = v.parse().unwrap_or(0),
                        "YMULT" | "YMU" => ymult = v.parse().unwrap_or(1.0),
                        "YOFF" | "YOF" => yoff = v.parse().unwrap_or(0.0),
                        "YZERO" | "YZE" => yzero = v.parse().unwrap_or(0.0),
                        "XINCR" | "XIN" => xincr = v.parse().unwrap_or(0.0),
                        "XZERO" | "XZE" => xzero = v.parse().unwrap_or(0.0),
                        _ => {}
                    }
                }
            }
            
            let n_digits_char = mmap[header_end + 1];
            let n_digits = (n_digits_char - b'0') as usize;
            let data_offset = header_end + 2 + n_digits;
            
            let isf_header = IsfHeader {
                byt_nr,
                byt_or,
                nr_pt,
                ymult,
                yoff,
                yzero,
                xincr,
                xzero,
                data_offset,
            };
            
            return Ok(WfmFile {
                mmap,
                model_number: "Tektronix ISF".to_string(),
                firmware_version: "ISF".to_string(),
                wfm_header: WfmHeader::Isf(isf_header),
            });
        }
        
        if dho::looks_like_dho_wfm(&mmap) {
            let dho_header = dho::parse(&mmap)?;
            let model = dho_header.model.clone();
            return Ok(WfmFile {
                mmap,
                model_number: model,
                firmware_version: "Unknown".to_string(),
                wfm_header: WfmHeader::Dho(dho_header),
            });
        }

        // Peek at first 4 bytes for magic
        let magic = &mmap[0..4];
        
        // Tektronix byte order check (0x0F0F little endian, 0xF0F0 big endian)
        if magic[0..2] == [0x0f, 0x0f] || magic[0..2] == [0xf0, 0xf0] {
            let mut cursor = Cursor::new(&mmap);
            let is_le = magic[0..2] == [0x0f, 0x0f];
            
            let endian = if is_le { Endian::Little } else { Endian::Big };
            
            let static_info = TektronixStaticFileInfo::read_options(&mut cursor, endian, ())?;
            
            let version = static_info.version_number.clone();
            
            let (exp_dim_offset, curve_offset) = if version.starts_with("WFM#001") {
                (166, 790)
            } else if version.starts_with("WFM#002") {
                (168, 792)
            } else if version.starts_with("WFM#003") {
                (168, 808)
            } else {
                return Err(anyhow::anyhow!("Unsupported Tektronix WFM version: {}", version));
            };

            cursor.seek(SeekFrom::Start(exp_dim_offset))?;
            let y_scale = f64::read_options(&mut cursor, endian, ())?;
            let y_offset = f64::read_options(&mut cursor, endian, ())?;

            cursor.seek(SeekFrom::Start(curve_offset + 14))?;
            let data_start_offset = u32::read_options(&mut cursor, endian, ())?;
            let postcharge_start_offset = u32::read_options(&mut cursor, endian, ())?;

            let tek_header = TektronixHeader {
                static_info,
                y_scale,
                y_offset,
                data_start_offset,
                postcharge_start_offset,
            };

            return Ok(WfmFile {
                mmap,
                model_number: "Tektronix".to_string(),
                firmware_version: version,
                wfm_header: WfmHeader::Tektronix(tek_header),
            });
        }
        
        if magic == [0xa5, 0xa5, 0x00, 0x00] {
            // DS1000E family
            let header = {
                let mut cursor = Cursor::new(&mmap);
                WfmHeader1000E::read(&mut cursor)?
            };
            return Ok(WfmFile {
                mmap,
                model_number: "DS1000E".to_string(),
                firmware_version: "Unknown".to_string(),
                wfm_header: WfmHeader::Ds1000e(header),
            });
        }
        
        if magic == [0xa5, 0xa5, 0x38, 0x00] {
            // DS2000 and DS4000 families share this magic
            let mut cursor = Cursor::new(&mmap);
            let file_header = FileHeader2000::read(&mut cursor)?;
            
            if file_header.model_number.contains("4000") || file_header.model_number.contains("DS4") || file_header.model_number.contains("MSO4") {
                cursor.set_position(44);
                let wfm_header = WfmHeader4000::read(&mut cursor)?;
                return Ok(WfmFile {
                    mmap,
                    model_number: file_header.model_number,
                    firmware_version: file_header.firmware_version,
                    wfm_header: WfmHeader::Ds4000(wfm_header),
                });
            } else {
                cursor.set_position(56);
                let wfm_header = WfmHeader2000::read(&mut cursor)?;
                return Ok(WfmFile {
                    mmap,
                    model_number: file_header.model_number,
                    firmware_version: file_header.firmware_version,
                    wfm_header: WfmHeader::Ds2000(wfm_header),
                });
            }
        }
        
        // Standard FileHeader based models (Z and newer)
        let (file_header, wfm_header) = {
            let mut cursor = Cursor::new(&mmap);
            let file_header = FileHeader::read(&mut cursor)?;
            cursor.set_position(64);
            let wfm_header = if file_header.model_number.contains('Z') && 
                               (file_header.model_number.starts_with("DS1") || file_header.model_number.starts_with("MSO1")) {
                WfmHeader::Ds1000z(WfmHeader1000Z::read(&mut cursor)?)
            } else {
                return Err(anyhow::anyhow!("Unsupported model: {}", file_header.model_number));
            };
            (file_header, wfm_header)
        };
        
        Ok(WfmFile {
            mmap,
            model_number: file_header.model_number,
            firmware_version: file_header.firmware_version,
            wfm_header,
        })
    }

    /// 1-based channel numbers that contain data in this capture.
    pub fn enabled_channels(&self) -> Vec<usize> {
        let mut enabled = Vec::new();
        match &self.wfm_header {
            WfmHeader::Ds1000z(header) => {
                for i in 0..4 { if header.is_ch_enabled(i) { enabled.push(i + 1); } }
            },
            WfmHeader::Ds1000e(header) => {
                if header.channels[0].enabled_val != 0 { enabled.push(1); }
                if header.channels[1].enabled_val != 0 { enabled.push(2); }
            },
            WfmHeader::Ds2000(header) => {
                for i in 0..4 { if header.is_ch_enabled(i) { enabled.push(i + 1); } }
            },
            WfmHeader::Ds4000(header) => {
                for i in 0..4 { if header.is_ch_enabled(i) { enabled.push(i + 1); } }
            },
            WfmHeader::Tektronix(_) | WfmHeader::Isf(_) => {
                enabled.push(1);
            },
            WfmHeader::Dho(header) => {
                for i in 0..4 { if header.is_ch_enabled(i) { enabled.push(i + 1); } }
            }
        }
        enabled
    }

    /// Decode a single channel into volts. `channel` is 1-based.
    pub fn extract_channel(&self, channel: usize, start: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<f32>> {
        if channel < 1 {
            return Err(anyhow::anyhow!("Channel index must be >= 1"));
        }
        let ch_idx = channel - 1;
        match &self.wfm_header {
            WfmHeader::Ds1000z(h)   => Parser::get_channel_data_1000z(self, h, ch_idx, start, length),
            WfmHeader::Ds1000e(h)   => Parser::get_channel_data_1000e(self, h, ch_idx, start, length),
            WfmHeader::Ds2000(h)    => Parser::get_channel_data_2000(self, h, ch_idx, start, length),
            WfmHeader::Ds4000(h)    => Parser::get_channel_data_4000(self, h, ch_idx, start, length),
            WfmHeader::Tektronix(h) => Parser::get_channel_data_tektronix(self, h, ch_idx, start, length),
            WfmHeader::Isf(h)       => Parser::get_channel_data_isf(self, h, ch_idx, start, length),
            WfmHeader::Dho(h)       => Parser::get_channel_data_dho(self, h, ch_idx, start, length),
        }
    }

    /// Decode every channel slot. Position i in the returned vec is `None` when
    /// channel i+1 is not enabled.
    pub fn extract_all_channels(&self, start: Option<usize>, length: Option<usize>) -> anyhow::Result<Vec<Option<Vec<f32>>>> {
        Parser::get_all_channels(self, start, length)
    }

    /// Time axis (x_origin, x_increment) when the format and parser support it.
    /// Returns None for DS1000E and Tektronix WFM where the relevant header
    /// fields are not yet parsed.
    pub fn time_axis(&self) -> Option<TimeAxis> {
        match &self.wfm_header {
            WfmHeader::Ds1000z(h) => {
                if h.sample_rate_ghz <= 0.0 { return None; }
                let dt = 1.0 / (h.sample_rate_ghz as f64 * 1e9);
                let trigger = h.picoseconds_offset as f64 * 1e-12;
                let n = h.points() as f64;
                Some(TimeAxis { x_increment: dt, x_origin: trigger - n * dt / 2.0 })
            }
            WfmHeader::Ds2000(h) => {
                if h.sample_rate_hz <= 0.0 { return None; }
                let dt = 1.0 / h.sample_rate_hz as f64;
                // Older DS2A captures with firmware 00.03.00.01.03 store a non-zero
                // time_offset even when the saved screenshot shows a centered trigger;
                // RigolWFM treats that as a firmware quirk and zeroes the offset.
                let trigger_ps = if self.model_number.starts_with("DS2A") && self.firmware_version == "00.03.00.01.03" {
                    0.0
                } else {
                    h.time_offset_ps as f64 * 1e-12
                };
                let trigger = trigger_ps + h.z_pt_offset as f64 * dt;
                let n = h.wfm_len as f64;
                Some(TimeAxis { x_increment: dt, x_origin: trigger - n * dt / 2.0 })
            }
            WfmHeader::Ds4000(h) => {
                if h.sample_rate_hz <= 0.0 { return None; }
                let dt = 1.0 / h.sample_rate_hz as f64;
                // No trigger-relative offset is parsed for DS4000; assume centered trigger.
                let n = h.mem_depth as f64;
                Some(TimeAxis { x_increment: dt, x_origin: -n * dt / 2.0 })
            }
            WfmHeader::Isf(h) => {
                if h.xincr <= 0.0 { return None; }
                Some(TimeAxis { x_increment: h.xincr, x_origin: h.xzero })
            }
            WfmHeader::Dho(h) => {
                if h.x_increment <= 0.0 { return None; }
                Some(TimeAxis { x_increment: h.x_increment, x_origin: h.x_origin })
            }
            WfmHeader::Ds1000e(_) | WfmHeader::Tektronix(_) => None,
        }
    }

    /// Sample count for the given channel, derived from the header without decoding.
    pub fn channel_sample_count(&self, channel: usize) -> anyhow::Result<usize> {
        if channel < 1 {
            return Err(anyhow::anyhow!("Channel index must be >= 1"));
        }
        let ch_idx = channel - 1;
        let count = match &self.wfm_header {
            WfmHeader::Ds1000z(h) => {
                if ch_idx > 3 || !h.is_ch_enabled(ch_idx) {
                    return Err(anyhow::anyhow!("Channel {} is not enabled", channel));
                }
                h.points() as usize
            }
            WfmHeader::Ds1000e(h) => {
                if ch_idx > 1 {
                    return Err(anyhow::anyhow!("DS1000E only has 2 channels"));
                }
                if h.channels[ch_idx].enabled_val == 0 {
                    return Err(anyhow::anyhow!("Channel {} is not enabled", channel));
                }
                if ch_idx == 0 { h.ch1_points() } else { h.ch2_points() }
            }
            WfmHeader::Ds2000(h) => {
                if ch_idx > 3 {
                    return Err(anyhow::anyhow!("Channel must be between 1 and 4"));
                }
                if !h.is_ch_enabled(ch_idx) {
                    return Err(anyhow::anyhow!("Channel {} is not enabled", channel));
                }
                h.wfm_len as usize
            }
            WfmHeader::Ds4000(h) => {
                if ch_idx > 3 {
                    return Err(anyhow::anyhow!("Channel must be between 1 and 4"));
                }
                if !h.is_ch_enabled(ch_idx) {
                    return Err(anyhow::anyhow!("Channel {} is not enabled", channel));
                }
                h.mem_depth as usize
            }
            WfmHeader::Tektronix(h) => {
                if ch_idx > 0 {
                    return Err(anyhow::anyhow!("Tektronix WFM has only 1 channel"));
                }
                let base = h.static_info.byte_offset_to_curve_buffer as usize;
                let data_start = base + h.data_start_offset as usize;
                let data_end = base + h.postcharge_start_offset as usize;
                let bpp = h.static_info.num_bytes_per_point as usize;
                if bpp == 0 || data_end <= data_start {
                    return Err(anyhow::anyhow!("Invalid curve buffer offsets"));
                }
                (data_end - data_start) / bpp
            }
            WfmHeader::Isf(h) => {
                if ch_idx > 0 {
                    return Err(anyhow::anyhow!("ISF has only 1 channel"));
                }
                let bpp = (h.byt_nr as usize).max(1);
                let raw_len = self.mmap.len().saturating_sub(h.data_offset);
                std::cmp::min(h.nr_pt as usize, raw_len / bpp)
            }
            WfmHeader::Dho(h) => {
                if ch_idx > 3 {
                    return Err(anyhow::anyhow!("Channel must be between 1 and 4"));
                }
                if !h.is_ch_enabled(ch_idx) {
                    return Err(anyhow::anyhow!("Channel {} is not enabled", channel));
                }
                h.n_pts_per_ch
            }
        };
        Ok(count)
    }
}
