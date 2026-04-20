use std::fs::File;
use std::io::{Cursor, Seek, SeekFrom};
use memmap2::Mmap;
use binrw::{BinRead, Endian};
use crate::structs::{FileHeader, WfmHeader1000Z, WfmHeader1000E, WfmHeader2000, FileHeader2000, TektronixStaticFileInfo, TektronixHeader};

pub enum WfmHeader {
    Ds1000z(WfmHeader1000Z),
    Ds1000e(WfmHeader1000E),
    Ds2000(WfmHeader2000),
    Tektronix(TektronixHeader),
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
