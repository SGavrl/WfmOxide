//! Siglent SDS1xx4X-E (and compatible SDS1000X-E series) binary capture format
//! (`.bin`).
//!
//! Unlike Keysight/Agilent the file has no magic bytes — detection is
//! heuristic: the first 16 bytes must be four well-formed u32 booleans
//! (channel-enabled flags), at least one channel must be enabled, the
//! sample count must be plausible, and the file must be large enough to
//! contain `enabled_channels × wave_length` bytes of u8 sample data after
//! the 0x800-byte header. The layout below is documented by:
//!
//!   - geekman/siglent-bin2sr (010 Editor template),
//!   - danielpclin/siglent-sds1000x-e-binary-decode (Python port),
//!   - featherfeet/siglent2csv (C reference, source of the offsets).
//!
//! ```text
//! 0x000   u32   ch1_on, ch2_on, ch3_on, ch4_on   (booleans: 0 or 1)
//! 0x010   {f64 vdiv,  u32 mag, u32 units} × 4    (16-byte stride per channel)
//! 0x050   {f64 voff,  u32 mag, u32 units} × 4    (16-byte stride per channel)
//! 0x090   u32   digital_on, then 16× u32 d0..d15  (covers 0x090..0x0D3)
//! 0x0D4   {f64 tdiv,  u32 mag, u32 units}
//! 0x0E4   {f64 tdly,  u32 mag, u32 units}
//! 0x0F4   u32   wave_length    (samples per enabled analog channel)
//! 0x0F8   {f64 srate, u32 mag, u32 units}
//! 0x800+  raw u8 samples, one wave_length block per enabled analog channel
//! ```
//!
//! The voltage-conversion formula is `V = (byte - 128) × vdiv / 25 − v_off`,
//! with both `vdiv` and `v_off` scaled by `10^((magnitude - 8) × 3)` to
//! convert from the stored mantissa+SI-magnitude representation to plain SI
//! base units. `CODE_PER_DIV = 25` matches the C reference.

use anyhow::{bail, Result};

pub const HEADER_SIZE: usize = 0x800;
pub const CODE_PER_DIV: f64 = 25.0;
const MAGNITUDE_BASE: i32 = 8;

const OFF_CH_ON: [usize; 4] = [0x00, 0x04, 0x08, 0x0C];
const OFF_VDIV: [usize; 4] = [0x10, 0x20, 0x30, 0x40];
const OFF_VDIV_MAG: [usize; 4] = [0x18, 0x28, 0x38, 0x48];
const OFF_VOFF: [usize; 4] = [0x50, 0x60, 0x70, 0x80];
const OFF_VOFF_MAG: [usize; 4] = [0x58, 0x68, 0x78, 0x88];
const OFF_TDIV: usize = 0xD4;
const OFF_TDIV_MAG: usize = 0xDC;
const OFF_TDLY: usize = 0xE4;
const OFF_TDLY_MAG: usize = 0xEC;
const OFF_WAVE_LEN: usize = 0xF4;
const OFF_SRATE: usize = 0xF8;
const OFF_SRATE_MAG: usize = 0x100;

/// Convert the stored (mantissa, magnitude) pair to a plain SI value.
/// `magnitude == 8` is the base unit; each step is `10^3`.
pub fn apply_magnitude(value: f64, magnitude: u32) -> f64 {
    let exp = (magnitude as i32 - MAGNITUDE_BASE) * 3;
    value * 10f64.powi(exp)
}

fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn read_f64(buf: &[u8], off: usize) -> f64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&buf[off..off + 8]);
    f64::from_le_bytes(b)
}

#[derive(Clone, Debug)]
pub struct SiglentChannel {
    /// 1-based channel number.
    pub channel: usize,
    /// `vdiv` already converted to volts (volts per division on the front panel).
    pub volt_per_div: f64,
    /// Vertical offset already converted to volts.
    pub volt_offset: f64,
    /// Byte offset in the mmap where this channel's `wave_length`-byte
    /// block starts.
    pub data_offset: usize,
}

#[derive(Clone, Debug)]
pub struct SiglentHeader {
    pub wave_length: usize,
    /// Sample rate in Sa/s.
    pub sample_rate_hz: f64,
    /// Trigger delay in seconds (positive = trigger before screen midpoint).
    pub trigger_delay_s: f64,
    /// Horizontal scale in seconds/division.
    pub time_per_div_s: f64,
    pub channels: Vec<SiglentChannel>,
}

/// Heuristic content-based detector — the Siglent format carries no magic
/// bytes, so we require the first 16 bytes to look like 4 boolean enable
/// flags, at least one channel enabled, a plausible sample count, and
/// enough trailing bytes for the declared payload.
pub fn looks_like_siglent(data: &[u8]) -> bool {
    if data.len() < HEADER_SIZE {
        return false;
    }
    let flags = [
        read_u32(data, OFF_CH_ON[0]),
        read_u32(data, OFF_CH_ON[1]),
        read_u32(data, OFF_CH_ON[2]),
        read_u32(data, OFF_CH_ON[3]),
    ];
    if !flags.iter().all(|&v| v <= 1) {
        return false;
    }
    let n_enabled = flags.iter().filter(|&&v| v == 1).count();
    if n_enabled == 0 {
        return false;
    }
    let wave_len = read_u32(data, OFF_WAVE_LEN) as usize;
    if !(1..=1_000_000_000).contains(&wave_len) {
        return false;
    }
    let payload = match wave_len.checked_mul(n_enabled) {
        Some(p) => p,
        None => return false,
    };
    if HEADER_SIZE.checked_add(payload).is_none_or(|end| end > data.len()) {
        return false;
    }
    // Sample rate sanity (after SI-magnitude scaling).
    let srate_raw = read_f64(data, OFF_SRATE);
    let srate_mag = read_u32(data, OFF_SRATE_MAG);
    if !srate_raw.is_finite() {
        return false;
    }
    let srate = apply_magnitude(srate_raw, srate_mag);
    if !(1.0..=1e13).contains(&srate) {
        return false;
    }
    true
}

pub fn parse(data: &[u8]) -> Result<SiglentHeader> {
    if !looks_like_siglent(data) {
        bail!("not a Siglent SDS .bin capture (header heuristics failed)");
    }

    let wave_length = read_u32(data, OFF_WAVE_LEN) as usize;
    let sample_rate_hz = apply_magnitude(read_f64(data, OFF_SRATE), read_u32(data, OFF_SRATE_MAG));
    let trigger_delay_s = apply_magnitude(read_f64(data, OFF_TDLY), read_u32(data, OFF_TDLY_MAG));
    let time_per_div_s = apply_magnitude(read_f64(data, OFF_TDIV), read_u32(data, OFF_TDIV_MAG));

    let mut channels = Vec::with_capacity(4);
    let mut cursor = HEADER_SIZE;
    for ch in 0..4 {
        if read_u32(data, OFF_CH_ON[ch]) != 1 {
            continue;
        }
        let vdiv = apply_magnitude(read_f64(data, OFF_VDIV[ch]), read_u32(data, OFF_VDIV_MAG[ch]));
        let voff = apply_magnitude(read_f64(data, OFF_VOFF[ch]), read_u32(data, OFF_VOFF_MAG[ch]));
        let end = cursor
            .checked_add(wave_length)
            .ok_or_else(|| anyhow::anyhow!("Siglent: cursor overflow"))?;
        if end > data.len() {
            bail!(
                "Siglent: channel {} payload (offset {} + {} bytes) overruns file ({} bytes)",
                ch + 1,
                cursor,
                wave_length,
                data.len()
            );
        }
        channels.push(SiglentChannel {
            channel: ch + 1,
            volt_per_div: vdiv,
            volt_offset: voff,
            data_offset: cursor,
        });
        cursor = end;
    }

    Ok(SiglentHeader {
        wave_length,
        sample_rate_hz,
        trigger_delay_s,
        time_per_div_s,
        channels,
    })
}

impl SiglentHeader {
    /// Time-axis origin in seconds. Siglent positions the trigger at screen
    /// division 7 (0-indexed from the left): `t0 = trigger_delay − tdiv × 7`.
    pub fn x_origin(&self) -> f64 {
        self.trigger_delay_s - self.time_per_div_s * 7.0
    }

    pub fn x_increment(&self) -> f64 {
        if self.sample_rate_hz > 0.0 {
            1.0 / self.sample_rate_hz
        } else {
            0.0
        }
    }

    pub fn channel_index(&self, channel_1based: usize) -> Option<usize> {
        self.channels.iter().position(|c| c.channel == channel_1based)
    }
}
