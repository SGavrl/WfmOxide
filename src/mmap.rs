use std::fs::File;
use std::io::Cursor;
use memmap2::Mmap;
use binrw::BinRead;
use crate::structs::{FileHeader, WfmHeader1000Z, WfmHeader1000E};

pub enum WfmHeader {
    Ds1000z(WfmHeader1000Z),
    Ds1000e(WfmHeader1000E),
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
