//! LeCroy / Teledyne LeCroy waveform capture format (`.trc`).
//!
//! The file is a SCPI definite-length block transfer of a `WAVEDESC` block.
//! Most captures begin with an 11-byte ASCII prefix of the form
//! `#9000001350` (the `#` is the SCPI block-data flag, `9` is the
//! digit-count, the remaining nine digits are the payload length in
//! bytes), but some tools strip the prefix and dump the WAVEDESC at offset
//! zero. We locate the descriptor by searching for the literal magic
//! string `"WAVEDESC"` in the first 64 bytes of the file.
//!
//! The 346-byte `LECROY_2_3` descriptor is the long-standing public
//! template — the layout below is portable across DDA, LC, LSA, LT/LX,
//! Wave* (WaveSurfer/WaveRunner/WavePro/WaveAce) and HDO families because
//! every Teledyne LeCroy scope emits the same `LECROY_2_3` template.
//! Offsets are relative to the WAVEDESC magic byte.
//!
//! ```text
//! +16   char[16]   TEMPLATE_NAME    "LECROY_2_3"
//! +32   i16        COMM_TYPE         0 = byte, 1 = word
//! +34   i16        COMM_ORDER        0 = big-endian, 1 = little-endian
//! +36   i32        WAVE_DESCRIPTOR   length of this descriptor (== 346)
//! +40   i32        USER_TEXT         length of optional user text block
//! +48   i32        TRIGTIME_ARRAY    length of trigtime block (segmented captures)
//! +60   i32        WAVE_ARRAY_1      length of sample payload in BYTES
//! +116  i32        WAVE_ARRAY_COUNT  number of samples
//! +156  f32        VERTICAL_GAIN     V per raw count
//! +160  f32        VERTICAL_OFFSET   V (subtracted, see formula)
//! +172  i16        NOMINAL_BITS      ADC bit depth (8..14)
//! +176  f32        HORIZ_INTERVAL    seconds between samples
//! +180  f64        HORIZ_OFFSET      seconds; time of sample 0
//! +344  i16        WAVE_SOURCE       channel index (0..3 → CH1..CH4)
//! ```
//!
//! Sample payload starts at `WAVEDESC + WAVE_DESCRIPTOR + USER_TEXT +
//! TRIGTIME_ARRAY` and is `WAVE_ARRAY_1` bytes long. The voltage
//! conversion is `V = VERTICAL_GAIN × raw − VERTICAL_OFFSET`.
//! `COMM_ORDER` is stored as a single LSB byte (its high byte is always
//! zero in both endians for the values 0 and 1), so we can read it before
//! knowing the file's byte order.

use anyhow::{bail, Result};

pub const MAGIC: &[u8] = b"WAVEDESC";
const SEARCH_LIMIT: usize = 64;

const OFF_TEMPLATE_NAME: usize = 16;
const OFF_COMM_TYPE: usize = 32;
const OFF_COMM_ORDER: usize = 34;
const OFF_WAVE_DESCRIPTOR: usize = 36;
const OFF_USER_TEXT: usize = 40;
const OFF_TRIGTIME_ARRAY: usize = 48;
const OFF_WAVE_ARRAY_1: usize = 60;
const OFF_WAVE_ARRAY_COUNT: usize = 116;
const OFF_VGAIN: usize = 156;
const OFF_VOFF: usize = 160;
const OFF_NOMINAL_BITS: usize = 172;
const OFF_HORIZ_INTERVAL: usize = 176;
const OFF_HORIZ_OFFSET: usize = 180;
const OFF_WAVE_SOURCE: usize = 344;

/// Minimum bytes from WAVEDESC start needed to read every field above.
/// `OFF_WAVE_SOURCE + 2` is the largest fixed offset we touch.
const DESC_MIN_LEN: usize = OFF_WAVE_SOURCE + 2;

#[derive(Copy, Clone, Debug)]
pub enum LecroyByteOrder {
    Le,
    Be,
}

#[derive(Copy, Clone, Debug)]
pub enum LecroySampleWidth {
    Byte,
    Word,
}

#[derive(Clone, Debug)]
pub struct LecroyHeader {
    /// "LECROY_2_3" or vendor variant — trimmed, NUL-stripped ASCII.
    pub template_name: String,
    /// 1-based channel index inferred from WAVE_SOURCE (`+1`).
    pub channel: usize,
    pub byte_order: LecroyByteOrder,
    pub sample_width: LecroySampleWidth,
    pub n_points: usize,
    pub vertical_gain: f32,
    pub vertical_offset: f32,
    pub nominal_bits: i16,
    pub horiz_interval: f64,
    pub horiz_offset: f64,
    /// Byte offset of the sample payload in the mmap.
    pub data_offset: usize,
    /// Length of the sample payload in bytes.
    pub data_len: usize,
}

pub fn find_wavedesc(data: &[u8]) -> Option<usize> {
    let limit = SEARCH_LIMIT.min(data.len().saturating_sub(MAGIC.len()));
    (0..=limit).find(|&i| data[i..].starts_with(MAGIC))
}

pub fn looks_like_lecroy(data: &[u8]) -> bool {
    let Some(pos) = find_wavedesc(data) else {
        return false;
    };
    // Need enough trailing bytes for the descriptor itself.
    pos.checked_add(DESC_MIN_LEN).is_some_and(|end| end <= data.len())
}

#[inline]
fn read_i16(buf: &[u8], off: usize, order: LecroyByteOrder) -> i16 {
    let b = [buf[off], buf[off + 1]];
    match order {
        LecroyByteOrder::Le => i16::from_le_bytes(b),
        LecroyByteOrder::Be => i16::from_be_bytes(b),
    }
}
#[inline]
fn read_i32(buf: &[u8], off: usize, order: LecroyByteOrder) -> i32 {
    let b = [buf[off], buf[off + 1], buf[off + 2], buf[off + 3]];
    match order {
        LecroyByteOrder::Le => i32::from_le_bytes(b),
        LecroyByteOrder::Be => i32::from_be_bytes(b),
    }
}
#[inline]
fn read_f32(buf: &[u8], off: usize, order: LecroyByteOrder) -> f32 {
    let b = [buf[off], buf[off + 1], buf[off + 2], buf[off + 3]];
    match order {
        LecroyByteOrder::Le => f32::from_le_bytes(b),
        LecroyByteOrder::Be => f32::from_be_bytes(b),
    }
}
#[inline]
fn read_f64(buf: &[u8], off: usize, order: LecroyByteOrder) -> f64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[off..off + 8]);
    match order {
        LecroyByteOrder::Le => f64::from_le_bytes(b),
        LecroyByteOrder::Be => f64::from_be_bytes(b),
    }
}

pub fn parse(data: &[u8]) -> Result<LecroyHeader> {
    let wd_pos = find_wavedesc(data).ok_or_else(|| {
        anyhow::anyhow!("LeCroy: WAVEDESC magic not found in first {} bytes", SEARCH_LIMIT)
    })?;
    if wd_pos.checked_add(DESC_MIN_LEN).is_none_or(|end| end > data.len()) {
        bail!(
            "LeCroy: WAVEDESC at offset {} but file is only {} bytes (need ≥{})",
            wd_pos,
            data.len(),
            wd_pos + DESC_MIN_LEN
        );
    }
    let desc = &data[wd_pos..];

    // COMM_ORDER is a 16-bit int but only ever holds 0 or 1, so the LSB
    // byte alone tells us the endianness regardless of how we read it.
    let byte_order = if desc[OFF_COMM_ORDER] == 0 {
        LecroyByteOrder::Be
    } else {
        LecroyByteOrder::Le
    };

    let comm_type_raw = read_i16(desc, OFF_COMM_TYPE, byte_order);
    let sample_width = match comm_type_raw {
        0 => LecroySampleWidth::Byte,
        1 => LecroySampleWidth::Word,
        n => bail!("LeCroy: unknown COMM_TYPE={}", n),
    };

    let wave_descriptor = read_i32(desc, OFF_WAVE_DESCRIPTOR, byte_order);
    let user_text = read_i32(desc, OFF_USER_TEXT, byte_order);
    let trigtime_array = read_i32(desc, OFF_TRIGTIME_ARRAY, byte_order);
    let wave_array_1 = read_i32(desc, OFF_WAVE_ARRAY_1, byte_order);
    let wave_array_count = read_i32(desc, OFF_WAVE_ARRAY_COUNT, byte_order);

    for (name, v) in &[
        ("WAVE_DESCRIPTOR", wave_descriptor),
        ("USER_TEXT", user_text),
        ("TRIGTIME_ARRAY", trigtime_array),
        ("WAVE_ARRAY_1", wave_array_1),
        ("WAVE_ARRAY_COUNT", wave_array_count),
    ] {
        if *v < 0 {
            bail!("LeCroy: negative {} length ({})", name, v);
        }
    }

    let data_start = wd_pos
        .checked_add(wave_descriptor as usize)
        .and_then(|x| x.checked_add(user_text as usize))
        .and_then(|x| x.checked_add(trigtime_array as usize))
        .ok_or_else(|| anyhow::anyhow!("LeCroy: data offset overflow"))?;
    let data_len = wave_array_1 as usize;
    if data_start
        .checked_add(data_len)
        .is_none_or(|end| end > data.len())
    {
        bail!(
            "LeCroy: sample payload (offset {} + {} bytes) overruns file ({} bytes)",
            data_start,
            data_len,
            data.len()
        );
    }

    let bytes_per_sample = match sample_width {
        LecroySampleWidth::Byte => 1,
        LecroySampleWidth::Word => 2,
    };
    let n_points = (wave_array_count as usize).min(data_len / bytes_per_sample);

    let vgain = read_f32(desc, OFF_VGAIN, byte_order);
    let voff = read_f32(desc, OFF_VOFF, byte_order);
    let nominal_bits = read_i16(desc, OFF_NOMINAL_BITS, byte_order);
    let horiz_interval = read_f32(desc, OFF_HORIZ_INTERVAL, byte_order) as f64;
    let horiz_offset = read_f64(desc, OFF_HORIZ_OFFSET, byte_order);
    let wave_source = read_i16(desc, OFF_WAVE_SOURCE, byte_order);
    let channel = if (0..=3).contains(&wave_source) {
        (wave_source + 1) as usize
    } else {
        1
    };

    let template_name = {
        let raw = &desc[OFF_TEMPLATE_NAME..OFF_TEMPLATE_NAME + 16];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..end]).trim().to_string()
    };

    Ok(LecroyHeader {
        template_name,
        channel,
        byte_order,
        sample_width,
        n_points,
        vertical_gain: vgain,
        vertical_offset: voff,
        nominal_bits,
        horiz_interval,
        horiz_offset,
        data_offset: data_start,
        data_len,
    })
}
