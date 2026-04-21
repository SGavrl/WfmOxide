use std::fs::File;
use std::io::{Cursor, Seek, SeekFrom};
use memmap2::Mmap;
use binrw::{BinRead, Endian};
use crate::structs::{FileHeader, WfmHeader1000Z, WfmHeader1000E, WfmHeader2000, FileHeader2000, TektronixStaticFileInfo, TektronixHeader, IsfHeader};

pub enum WfmHeader {
    Ds1000z(WfmHeader1000Z),
    Ds1000e(WfmHeader1000E),
    Ds2000(WfmHeader2000),
    Tektronix(TektronixHeader),
    Isf(IsfHeader),
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
                data_offset,
            };
            
            return Ok(WfmFile {
                mmap,
                model_number: "Tektronix ISF".to_string(),
                firmware_version: "ISF".to_string(),
                wfm_header: WfmHeader::Isf(isf_header),
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
            // DS2000 family
            let (file_header, wfm_header) = {
                let mut cursor = Cursor::new(&mmap);
                let file_header = FileHeader2000::read(&mut cursor)?;
                cursor.set_position(56);
                let wfm_header = WfmHeader2000::read(&mut cursor)?;
                (file_header, wfm_header)
            };
            return Ok(WfmFile {
                mmap,
                model_number: file_header.model_number,
                firmware_version: file_header.firmware_version,
                wfm_header: WfmHeader::Ds2000(wfm_header),
            });
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
}
