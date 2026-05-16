//! Keysight / Agilent InfiniiVision binary capture format (`.bin`).
//!
//! Layout (all little-endian):
//!
//! ```text
//! FileHeader (12 bytes)
//!   [u8;2]  cookie     = "AG" (Agilent/Keysight) or "RG" (Rigol-rebrand)
//!   [u8;2]  version    = "10"  (ASCII)
//!   u32     file_size
//!   i32     n_waveforms
//!
//! Then, for each of n_waveforms:
//!   WaveformHeader
//!     i32     hdr_size           // self-describing; total bytes for this block
//!     i32     wf_type            // 1=normal, 2=peak max, 3=peak min, 6=logic, ...
//!     i32     n_buffers          // number of data blocks that follow
//!     i32     n_points
//!     i32     count              // averaging count
//!     f32     x_display_range
//!     f64     x_display_origin
//!     f64     x_increment
//!     f64     x_origin
//!     i32     x_units            // 1=volts 2=seconds 3=constant 4=amps 5=dB 6=Hz
//!     i32     y_units
//!     [u8;16] date
//!     [u8;16] time
//!     [u8;24] frame_model
//!     [u8;16] channel_name       // "1", "2", "Math", ...
//!     u8      acq_mode
//!     u8      completion
//!     u16     x_units_subtype
//!     u16     frame_flags
//!     u32     segment_index
//!     u32     segment_count
//!     f64     trigger_time
//!     f64     segment_time_tag
//!   (remaining bytes through hdr_size are skipped — different firmwares
//!    extend the header.)
//!
//!   For each of n_buffers data blocks:
//!     DataHeader (hdr_size bytes; we only read the documented 12)
//!       i32 hdr_size
//!       i16 buffer_type   // 1=normal f32 voltage, 2=max float, 3=min float,
//!                         // 4=time f32, 5=count u32, 6=logic u8
//!       i16 bytes_per_point
//!       i32 buffer_size  // payload size in bytes
//!     ... buffer_size bytes of payload
//! ```
//!
//! We surface every waveform record as a 1-based channel and decode the
//! first `buffer_type == 1` block (already in volts, IEEE 754 f32 LE).

use anyhow::{anyhow, bail, Result};
use std::io::{Cursor, Read, Seek, SeekFrom};

pub const COOKIE_AG: [u8; 2] = *b"AG";
pub const COOKIE_RG: [u8; 2] = *b"RG";
const VERSION: [u8; 2] = *b"10";

pub const BUFFER_TYPE_FLOAT: i16 = 1;

#[derive(Clone, Debug)]
pub struct KeysightWaveform {
    /// Display label from the file (`"1"`, `"2"`, `"Math"`, ...).
    pub channel_name: String,
    pub n_points: usize,
    pub x_increment: f64,
    pub x_origin: f64,
    /// Byte offset of the f32 voltage block in the mmap.
    pub data_offset: usize,
    /// Length of the voltage block in bytes (== n_points * 4 for type 1).
    pub data_len: usize,
    pub bytes_per_point: usize,
    pub buffer_type: i16,
}

#[derive(Clone, Debug)]
pub struct KeysightHeader {
    /// Vendor string from the cookie: `"Agilent/Keysight"` or `"Rigol"`.
    pub vendor: String,
    /// Scope model from the first waveform record, if any.
    pub model: String,
    pub waveforms: Vec<KeysightWaveform>,
}

pub fn looks_like_keysight(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    let cookie = &data[0..2];
    let version = &data[2..4];
    (cookie == COOKIE_AG || cookie == COOKIE_RG) && version == VERSION
}

fn read_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

fn read_i32(c: &mut Cursor<&[u8]>) -> Result<i32> {
    let mut b = [0u8; 4];
    c.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}
fn read_u32(c: &mut Cursor<&[u8]>) -> Result<u32> {
    let mut b = [0u8; 4];
    c.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_i16(c: &mut Cursor<&[u8]>) -> Result<i16> {
    let mut b = [0u8; 2];
    c.read_exact(&mut b)?;
    Ok(i16::from_le_bytes(b))
}
fn read_f64(c: &mut Cursor<&[u8]>) -> Result<f64> {
    let mut b = [0u8; 8];
    c.read_exact(&mut b)?;
    Ok(f64::from_le_bytes(b))
}

pub fn parse(data: &[u8]) -> Result<KeysightHeader> {
    if !looks_like_keysight(data) {
        bail!("not a Keysight/Agilent .bin file");
    }
    let mut c = Cursor::new(data);

    let mut cookie = [0u8; 2];
    c.read_exact(&mut cookie)?;
    let mut version = [0u8; 2];
    c.read_exact(&mut version)?;
    let _file_size = read_u32(&mut c)?;
    let n_waveforms = read_i32(&mut c)?;
    if !(0..=64).contains(&n_waveforms) {
        bail!("Keysight: implausible n_waveforms={}", n_waveforms);
    }

    let vendor = if cookie == COOKIE_AG {
        "Agilent/Keysight".to_string()
    } else {
        "Rigol".to_string()
    };

    let mut waveforms = Vec::with_capacity(n_waveforms as usize);
    let mut first_model = String::new();

    for wf_idx in 0..n_waveforms {
        let wf_start = c.position();
        let wf_hdr_size = read_i32(&mut c)?;
        if wf_hdr_size < 140 || (wf_start as i64 + wf_hdr_size as i64) > data.len() as i64 {
            bail!(
                "Keysight: waveform {} header size {} overruns file",
                wf_idx,
                wf_hdr_size
            );
        }

        let _wf_type = read_i32(&mut c)?;
        let n_buffers = read_i32(&mut c)?;
        let n_points = read_i32(&mut c)?;
        let _count = read_i32(&mut c)?;
        // x_disp_range (f32) — skip
        c.seek(SeekFrom::Current(4))?;
        let _x_disp_origin = read_f64(&mut c)?;
        let x_increment = read_f64(&mut c)?;
        let x_origin = read_f64(&mut c)?;
        let _x_units = read_i32(&mut c)?;
        let _y_units = read_i32(&mut c)?;
        // date, time, frame_model, channel_name
        let mut date = [0u8; 16];
        c.read_exact(&mut date)?;
        let mut time = [0u8; 16];
        c.read_exact(&mut time)?;
        let mut frame_model = [0u8; 24];
        c.read_exact(&mut frame_model)?;
        let mut chan_name = [0u8; 16];
        c.read_exact(&mut chan_name)?;
        let channel_name = read_string(&chan_name);
        let model = read_string(&frame_model);
        if first_model.is_empty() {
            first_model = model;
        }

        // Skip the rest of the waveform header — different firmwares add fields
        // (acq_mode, completion, segment info, etc.) and we don't need them
        // for decoding.
        c.seek(SeekFrom::Start(wf_start + wf_hdr_size as u64))?;

        if n_buffers <= 0 || n_buffers > 16 {
            bail!(
                "Keysight: waveform {} has implausible n_buffers={}",
                wf_idx,
                n_buffers
            );
        }
        if n_points <= 0 {
            bail!(
                "Keysight: waveform {} has implausible n_points={}",
                wf_idx,
                n_points
            );
        }

        let mut chosen: Option<(usize, usize, usize, i16)> = None; // (offset, len, bpp, type)
        for buf_idx in 0..n_buffers {
            let bh_start = c.position();
            let bh_size = read_i32(&mut c)?;
            if bh_size < 12 || (bh_start as i64 + bh_size as i64) > data.len() as i64 {
                bail!(
                    "Keysight: waveform {} buffer {} header size {} overruns file",
                    wf_idx,
                    buf_idx,
                    bh_size
                );
            }
            let buffer_type = read_i16(&mut c)?;
            let bytes_per_point = read_i16(&mut c)? as usize;
            let buffer_size = read_i32(&mut c)? as usize;
            c.seek(SeekFrom::Start(bh_start + bh_size as u64))?;
            let data_start = c.position() as usize;
            if data_start
                .checked_add(buffer_size)
                .is_none_or(|end| end > data.len())
            {
                bail!(
                    "Keysight: waveform {} buffer {} payload (offset {} + {} bytes) overruns file ({} bytes)",
                    wf_idx,
                    buf_idx,
                    data_start,
                    buffer_size,
                    data.len(),
                );
            }
            // Keep the first float buffer; if none exists, fall back to the
            // first buffer of any kind so we can at least record presence.
            if chosen.is_none() || (chosen.as_ref().unwrap().3 != BUFFER_TYPE_FLOAT
                && buffer_type == BUFFER_TYPE_FLOAT)
            {
                chosen = Some((data_start, buffer_size, bytes_per_point, buffer_type));
            }
            c.seek(SeekFrom::Current(buffer_size as i64))?;
        }
        let (data_offset, data_len, bpp, btype) = chosen
            .ok_or_else(|| anyhow!("Keysight: waveform {} has no buffers", wf_idx))?;
        if btype != BUFFER_TYPE_FLOAT {
            bail!(
                "Keysight: waveform {} buffer_type {} not yet supported (only normal float)",
                wf_idx,
                btype
            );
        }
        if bpp != 4 {
            bail!(
                "Keysight: waveform {} bytes_per_point {} not yet supported (float buffers are 4)",
                wf_idx,
                bpp
            );
        }
        waveforms.push(KeysightWaveform {
            channel_name,
            n_points: n_points as usize,
            x_increment,
            x_origin,
            data_offset,
            data_len,
            bytes_per_point: bpp,
            buffer_type: btype,
        });
    }

    Ok(KeysightHeader {
        vendor,
        model: first_model,
        waveforms,
    })
}
