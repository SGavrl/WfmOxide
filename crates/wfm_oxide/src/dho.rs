use anyhow::{anyhow, Result};
use flate2::read::ZlibDecoder;
use std::io::Read;

use crate::sample::Affine;

const FILE_HEADER_SIZE: usize = 24;
const BLOCK_HEADER_SIZE: usize = 12;
const ADC_MIDPOINT: f32 = 32768.0;

const DHO1000_TICK_S: f64 = 1e-8;
const DHO800_TICK_S: f64 = 8e-10;

const BLOCK_TYPE_DHO800_PARAMS: u16 = 5;
const BLOCK_TYPE_SETTINGS: u16 = 6;
const BLOCK_TYPE_CHANNEL_PARAMS: u16 = 9;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DhoFamily {
    Dho800,
    Dho1000,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct DhoHeader {
    pub family: DhoFamily,
    pub model: String,
    pub channel_cals: [Option<Affine>; 4],
    pub n_pts_per_ch: usize,
    pub n_ch: usize,
    pub data_start: usize,
    pub x_increment: f64,
    pub x_origin: f64,
}

impl DhoHeader {
    pub fn is_ch_enabled(&self, ch: usize) -> bool {
        ch < 4 && self.channel_cals[ch].is_some()
    }
}

struct ParsedBlock {
    block_id: u16,
    block_type: u16,
    decompressed: Vec<u8>,
}

fn try_decompress(data: &[u8], decomp_size: usize) -> Vec<u8> {
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::with_capacity(decomp_size);
    if decoder.read_to_end(&mut out).is_ok() {
        out
    } else {
        data.to_vec()
    }
}

fn parse_blocks(data: &[u8]) -> Result<(Vec<ParsedBlock>, usize)> {
    let mut blocks = Vec::new();
    let mut offset = FILE_HEADER_SIZE;

    loop {
        if offset + BLOCK_HEADER_SIZE > data.len() {
            return Err(anyhow!("Unexpected EOF inside DHO block region"));
        }
        let block_id = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let block_type = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
        let decomp_size = u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as usize;
        let comp_size = u16::from_le_bytes([data[offset + 6], data[offset + 7]]) as usize;
        let len_content_raw = u16::from_le_bytes([data[offset + 8], data[offset + 9]]) as usize;
        // bytes 10..12 reserved

        if len_content_raw == 0 && comp_size == 0 {
            offset += BLOCK_HEADER_SIZE;
            return Ok((blocks, offset));
        }

        let content_start = offset + BLOCK_HEADER_SIZE;
        let content_end = content_start + len_content_raw;
        if content_end > data.len() {
            return Err(anyhow!("DHO block content overruns file"));
        }
        let comp_end = content_start + comp_size.min(len_content_raw);
        let raw_content = &data[content_start..comp_end];

        let decompressed = if comp_size != decomp_size {
            try_decompress(raw_content, decomp_size)
        } else {
            raw_content.to_vec()
        };

        blocks.push(ParsedBlock { block_id, block_type, decompressed });
        offset = content_end;
    }
}

fn read_i64_le(buf: &[u8], at: usize) -> Option<i64> {
    if at + 8 > buf.len() { return None; }
    let mut a = [0u8; 8];
    a.copy_from_slice(&buf[at..at + 8]);
    Some(i64::from_le_bytes(a))
}

fn read_i32_le(buf: &[u8], at: usize) -> Option<i32> {
    if at + 4 > buf.len() { return None; }
    let mut a = [0u8; 4];
    a.copy_from_slice(&buf[at..at + 4]);
    Some(i32::from_le_bytes(a))
}

fn read_u32_le(buf: &[u8], at: usize) -> Option<u32> {
    if at + 4 > buf.len() { return None; }
    let mut a = [0u8; 4];
    a.copy_from_slice(&buf[at..at + 4]);
    Some(u32::from_le_bytes(a))
}

fn read_u64_le(buf: &[u8], at: usize) -> Option<u64> {
    if at + 8 > buf.len() { return None; }
    let mut a = [0u8; 8];
    a.copy_from_slice(&buf[at..at + 8]);
    Some(u64::from_le_bytes(a))
}

fn extract_calibration(blocks: &[ParsedBlock]) -> (DhoFamily, [Option<Affine>; 4]) {
    let is_dho800 = blocks.iter().any(|b| {
        b.block_type == BLOCK_TYPE_DHO800_PARAMS && b.block_id >= 1 && b.block_id <= 4
    });

    let mut cals: [Option<Affine>; 4] = [None, None, None, None];

    if is_dho800 {
        for b in blocks {
            if b.block_type == BLOCK_TYPE_DHO800_PARAMS && b.block_id >= 1 && b.block_id <= 4 {
                let scale_num = match read_i64_le(&b.decompressed, 1) {
                    Some(v) => v,
                    None => continue,
                };
                let v_center_raw = match read_i32_le(&b.decompressed, 38) {
                    Some(v) => v,
                    None => continue,
                };
                let scale = scale_num as f64 / 7_500_000_000_000.0;
                let v_center = -(v_center_raw as f64) / 1.0e9;
                let offset = v_center - scale * (ADC_MIDPOINT as f64);
                cals[(b.block_id - 1) as usize] = Some(Affine {
                    scale: scale as f32,
                    offset: offset as f32,
                });
            }
        }
    } else {
        for b in blocks {
            if b.block_type == BLOCK_TYPE_CHANNEL_PARAMS && b.block_id >= 1 && b.block_id <= 4 {
                let scale_num = match read_i64_le(&b.decompressed, 1) {
                    Some(v) => v,
                    None => continue,
                };
                let v_center_raw = match read_i64_le(&b.decompressed, 38) {
                    Some(v) => v,
                    None => continue,
                };
                let scale = scale_num as f64 / 750_000_000_000.0;
                let v_center = v_center_raw as f64 / 1.0e8;
                let offset = -v_center - scale * (ADC_MIDPOINT as f64);
                cals[(b.block_id - 1) as usize] = Some(Affine {
                    scale: scale as f32,
                    offset: offset as f32,
                });
            }
        }

        // Legacy single-channel fallback.
        if cals.iter().all(|c| c.is_none()) {
            let mut scale_opt: Option<f64> = None;
            let mut v_center_opt: Option<f64> = None;
            for b in blocks {
                if b.block_id == 1 && b.block_type == BLOCK_TYPE_CHANNEL_PARAMS {
                    if let Some(num) = read_i64_le(&b.decompressed, 1) {
                        scale_opt = Some(num as f64 / 750_000_000_000.0);
                    }
                } else if b.block_type == BLOCK_TYPE_SETTINGS {
                    if let Some(raw) = read_i32_le(&b.decompressed, 36) {
                        v_center_opt = Some(raw as f64 / 1.0e8);
                    }
                }
            }
            if let (Some(scale), Some(v_center)) = (scale_opt, v_center_opt) {
                let offset = -v_center - scale * (ADC_MIDPOINT as f64);
                cals[0] = Some(Affine {
                    scale: scale as f32,
                    offset: offset as f32,
                });
            }
        }
    }

    let family = if is_dho800 { DhoFamily::Dho800 } else { DhoFamily::Dho1000 };
    (family, cals)
}

fn parse_model(blocks: &[ParsedBlock]) -> String {
    for b in blocks {
        let text = String::from_utf8_lossy(&b.decompressed);
        for prefix in ["DHO", "MSO"] {
            if let Some(idx) = text.find(prefix) {
                let mut model = String::new();
                for c in text[idx..].chars().take(20) {
                    if c.is_ascii_graphic() && c != '\0' {
                        model.push(c);
                    } else {
                        break;
                    }
                }
                if model.len() >= 3 {
                    return model;
                }
            }
        }
    }
    String::new()
}

fn find_data_section(
    data: &[u8],
    blocks_end: usize,
    is_dho800: bool,
) -> Result<(usize, usize, usize, f64, f64)> {
    let mut offset = blocks_end;
    while offset < data.len() && data[offset] == 0 {
        offset += 1;
    }
    if offset + 40 >= data.len() {
        return Err(anyhow!("DHO data section header truncated"));
    }

    let n_pts_u64 = read_u64_le(data, offset).ok_or_else(|| anyhow!("EOF reading n_pts_u64"))?;
    if n_pts_u64 == 0 || n_pts_u64 > 2_000_000_000 {
        return Err(anyhow!("DHO n_pts_u64 out of range: {}", n_pts_u64));
    }

    let n_pts_hint = read_u32_le(data, offset + 24).ok_or_else(|| anyhow!("EOF reading n_pts_hint"))?;
    let (n_pts_per_ch, n_ch) = if n_pts_hint > 0 {
        let n_pts_per_ch = n_pts_hint as usize;
        let n_ch = ((n_pts_u64 as f64) / (n_pts_per_ch as f64)).round() as usize;
        (n_pts_per_ch, n_ch)
    } else {
        (n_pts_u64 as usize, 1)
    };
    if n_pts_per_ch == 0 || n_ch == 0 || n_ch > 4 {
        return Err(anyhow!("DHO data shape invalid: n_pts={}, n_ch={}", n_pts_per_ch, n_ch));
    }

    let mut x_increment_raw = read_u32_le(data, offset + 16).unwrap_or(0);
    if x_increment_raw == 0 || x_increment_raw > 1_000_000_000 {
        x_increment_raw = 1;
    }
    let tick_s = if is_dho800 { DHO800_TICK_S } else { DHO1000_TICK_S };
    let x_increment = x_increment_raw as f64 * tick_s;
    let x_origin = -(n_pts_per_ch as f64 / 2.0) * x_increment;

    Ok((n_pts_per_ch, n_ch, offset + 40, x_increment, x_origin))
}

pub fn looks_like_dho_wfm(data: &[u8]) -> bool {
    if data.len() < 24 { return false; }
    if data[0..4] != [0x02, 0x00, 0x00, 0x00] { return false; }
    // Bytes 10-15 are always zero in observed DHO captures (padding around the
    // model code at 8-9). Combined with the magic, this is distinct from every
    // other supported family.
    data[10..16].iter().all(|&b| b == 0)
}

pub fn parse(data: &[u8]) -> Result<DhoHeader> {
    if data.len() < FILE_HEADER_SIZE + BLOCK_HEADER_SIZE {
        return Err(anyhow!("DHO file too small"));
    }

    let (blocks, blocks_end) = parse_blocks(data)?;
    if blocks.is_empty() {
        return Err(anyhow!("No DHO metadata blocks found"));
    }

    let (family, cals) = extract_calibration(&blocks);
    if cals.iter().all(|c| c.is_none()) {
        return Err(anyhow!("Could not extract DHO voltage calibration"));
    }

    let is_dho800 = matches!(family, DhoFamily::Dho800);
    let (n_pts_per_ch, n_ch, data_start, x_increment, x_origin) =
        find_data_section(data, blocks_end, is_dho800)?;

    if data_start + n_pts_per_ch * n_ch * 2 > data.len() {
        return Err(anyhow!("DHO data section overruns file"));
    }

    let mut header = DhoHeader {
        family,
        model: parse_model(&blocks),
        channel_cals: cals,
        n_pts_per_ch,
        n_ch,
        data_start,
        x_increment,
        x_origin,
    };
    if header.model.is_empty() {
        header.model = match family {
            DhoFamily::Dho800 => "DHO800".to_string(),
            DhoFamily::Dho1000 => "DHO1000".to_string(),
        };
    }

    // The format only stores n_ch interleaved channels with no slot identity, so
    // active channels are exposed as CH1..CH(n_ch). Fall back to the first
    // available calibration for any active slot that lacks its own.
    let fallback = header.channel_cals.iter().find_map(|c| c.as_ref()).copied();
    for ch in 0..4 {
        if ch < n_ch {
            if header.channel_cals[ch].is_none() {
                header.channel_cals[ch] = fallback;
            }
        } else {
            header.channel_cals[ch] = None;
        }
    }

    Ok(header)
}
