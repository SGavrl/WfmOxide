//! Rohde & Schwarz RTP / RTO / RTE oscilloscope waveform format.
//!
//! Captures are saved as a **pair** of files that share a basename:
//!
//!   - `Trace.bin`     — XML metadata describing the capture
//!   - `Trace.Wfm.bin` — binary sample payload (8-byte header + raw samples)
//!
//! The user passes the `.bin` XML file; this module opens its `.Wfm.bin`
//! sibling and uses it as the working mmap. XML is parsed by a small
//! attribute scanner — every interesting tag in the R&S schema follows
//! the pattern `<… Name="X" Value="Y" …/>` (and for indexed groups,
//! `… I_0="…" I_1="…" …`), so a full XML parser is not needed and saves a
//! dependency.
//!
//! ## `.Wfm.bin` header (8 bytes, little-endian)
//!
//! ```text
//! u32  format_code        // 0 = i8, 1 = i16, 4 = f32, 6 = XY (f64 time + f32 V)
//! u32  hw_record_length   // total samples per channel including leading + trailing settling
//! ```
//!
//! Multi-channel captures interleave samples one row at a time:
//! `[ch_0[0], ch_1[0], … ch_0[1], ch_1[1], …]`. For format 6 each row
//! is prefixed by an `f64` timestamp.
//!
//! ## Voltage formula (integer formats)
//!
//! ```text
//! conv = (step_factor × vertical_scale) / quantisation_levels
//! pos  = position_div × vertical_scale
//! off  = vertical_offset − pos
//! V    = raw × conv + off
//! ```
//!
//! For `FLOAT` and `XYDOUBLEFLOAT` the payload is already in volts and no
//! affine transform is needed.

use anyhow::{anyhow, bail, Result};
use memmap2::Mmap;
use std::fs::File;
use std::path::{Path, PathBuf};

const FORMAT_INT8: u32 = 0;
const FORMAT_INT16: u32 = 1;
const FORMAT_FLOAT32: u32 = 4;
const FORMAT_XYDOUBLEFLOAT: u32 = 6;

/// Open the user-passed `.bin` XML metadata file and locate / mmap its
/// `.Wfm.bin` sibling. Returns both the parsed header (drawn from the XML)
/// and the payload mmap (becomes `WfmFile.mmap` upstream).
pub fn open(xml_path: &str) -> Result<(RsHeader, Mmap)> {
    let xml_bytes = std::fs::read(xml_path)
        .map_err(|e| anyhow!("R&S: failed to read XML metadata file {}: {}", xml_path, e))?;
    let xml = std::str::from_utf8(&xml_bytes)
        .map_err(|_| anyhow!("R&S: metadata file {} is not valid UTF-8", xml_path))?;
    if !is_rs_xml(xml) {
        bail!("R&S: {} does not look like an R&S XML waveform metadata file", xml_path);
    }
    let meta = parse_xml(xml)?;

    let payload_path = sibling_wfm_bin_path(Path::new(xml_path)).ok_or_else(|| {
        anyhow!(
            "R&S: cannot derive sibling .Wfm.bin path from {} (expected '<base>.bin' + '<base>.Wfm.bin')",
            xml_path
        )
    })?;
    let payload_file = File::open(&payload_path).map_err(|e| {
        anyhow!(
            "R&S: could not open payload file {}: {} (the XML at {} expects a sibling .Wfm.bin)",
            payload_path.display(),
            e,
            xml_path,
        )
    })?;
    let mmap = unsafe { Mmap::map(&payload_file)? };

    if mmap.len() < 8 {
        bail!(
            "R&S: payload file {} is only {} bytes (need ≥8 for header)",
            payload_path.display(),
            mmap.len()
        );
    }
    let format_code = u32::from_le_bytes([mmap[0], mmap[1], mmap[2], mmap[3]]);
    let hw_record_length = u32::from_le_bytes([mmap[4], mmap[5], mmap[6], mmap[7]]) as usize;
    if hw_record_length == 0 {
        bail!("R&S: payload header reports zero record length");
    }

    let (bytes_per_sample, time_prefix_bytes) = match format_code {
        FORMAT_INT8 => (1usize, 0usize),
        FORMAT_INT16 => (2, 0),
        FORMAT_FLOAT32 => (4, 0),
        FORMAT_XYDOUBLEFLOAT => (4, 8),
        n => bail!("R&S: unsupported format code {} (header bytes 0..4 = {:?})", n, &mmap[..4]),
    };
    let row_stride = time_prefix_bytes + meta.active_channels.len() * bytes_per_sample;
    let need = row_stride.checked_mul(hw_record_length).ok_or_else(|| {
        anyhow!("R&S: payload size overflow ({}×{})", row_stride, hw_record_length)
    })?;
    if 8 + need > mmap.len() {
        bail!(
            "R&S: payload too short — header says {} rows × {} bytes = {} bytes but file holds {} after header",
            hw_record_length,
            row_stride,
            need,
            mmap.len() - 8
        );
    }

    let header = RsHeader {
        format_code,
        bytes_per_sample,
        time_prefix_bytes,
        row_stride,
        hw_record_length,
        record_length: meta.record_length.min(hw_record_length),
        leading_settling: meta.leading_settling.min(hw_record_length),
        x_start: meta.x_start,
        x_stop: meta.x_stop,
        channels: meta.active_channels,
        source_xml_path: xml_path.to_string(),
        payload_path: payload_path.display().to_string(),
    };

    Ok((header, mmap))
}

#[derive(Clone, Debug)]
pub struct RsChannel {
    /// 1-based channel number derived from the source name (CH1..CH4).
    pub channel: usize,
    /// Slot in the interleaved payload (0..active_channels.len()).
    pub slot: usize,
    /// Volts per division.
    pub vertical_scale: f32,
    /// Vertical offset (volts).
    pub vertical_offset: f32,
    /// Vertical position in divisions (subtracted as `position × scale`).
    pub vertical_position: f32,
    /// Step factor (multiplier applied to scale during the integer conversion).
    pub step_factor: f32,
    /// Quantisation levels — used to normalise integer raw to ±0.5 range.
    pub quantisation_levels: f32,
}

#[derive(Clone, Debug)]
pub struct RsHeader {
    pub format_code: u32,
    pub bytes_per_sample: usize,
    pub time_prefix_bytes: usize,
    pub row_stride: usize,
    pub hw_record_length: usize,
    pub record_length: usize,
    pub leading_settling: usize,
    pub x_start: f64,
    pub x_stop: f64,
    pub channels: Vec<RsChannel>,
    pub source_xml_path: String,
    pub payload_path: String,
}

impl RsHeader {
    pub fn x_increment(&self) -> f64 {
        if self.record_length == 0 || self.x_stop <= self.x_start {
            0.0
        } else {
            (self.x_stop - self.x_start) / self.record_length as f64
        }
    }
    pub fn x_origin(&self) -> f64 {
        self.x_start
    }
}

/// Cheap content sniff: detect that a candidate file is the R&S metadata
/// XML. Looks for an XML declaration and the unique R&S `eRS_` prefix
/// that appears in every meaningful enum value.
pub fn is_rs_xml(text: &str) -> bool {
    let head = text.get(..text.len().min(4096)).unwrap_or("");
    let has_xml_decl = head.starts_with("<?xml") || head.starts_with("\u{feff}<?xml");
    has_xml_decl && head.contains("eRS_")
}

/// Inspect the first kilobyte of an arbitrary file and decide whether it
/// is plausibly an R&S `.bin` XML metadata file. Used by `WfmFile::open`
/// before doing the more expensive full XML parse.
pub fn looks_like_rs(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    is_rs_xml(text)
}

fn sibling_wfm_bin_path(xml: &Path) -> Option<PathBuf> {
    // Replace the trailing `.bin` with `.Wfm.bin`. Use the literal string
    // form so we preserve any non-UTF-8 path components on platforms that
    // allow them (Path::with_extension would also collapse multi-dot stems).
    let s = xml.as_os_str().to_str()?;
    let stem = s.strip_suffix(".bin")?;
    Some(PathBuf::from(format!("{stem}.Wfm.bin")))
}

#[derive(Default)]
struct XmlMeta {
    record_length: usize,
    leading_settling: usize,
    quantisation_levels: f32,
    x_start: f64,
    x_stop: f64,
    signal_format: String,
    multi_channel_export: bool,
    /// Single-channel path uses these.
    source: String,
    vertical_scale: f32,
    vertical_position: f32,
    vertical_offset: f32,
    vertical_scale_step_factor: f32,
    /// Multi-channel path uses these (indexed 0..4 for I_0..I_3).
    mc_source: [String; 4],
    mc_state: [String; 4],
    mc_scale: [f32; 4],
    mc_position: [f32; 4],
    mc_offset: [f32; 4],
    mc_scale_step_factor: f32,
    active_channels: Vec<RsChannel>,
}

fn parse_xml(text: &str) -> Result<XmlMeta> {
    let mut m = XmlMeta::default();
    // Walk every <Prop Name="..."/> element. The scanner is intentionally
    // lenient: it does not validate the XML structure, only extracts the
    // attribute values we care about by their Name= key.
    for tag in text.split('<') {
        // Find `Name="…"` to identify which tag this is.
        let Some(name) = extract_attr(tag, "Name") else { continue };
        match name {
            "RecordLength" => {
                if let Some(v) = extract_attr(tag, "Value") { m.record_length = v.parse().unwrap_or(0); }
            }
            "LeadingSettlingSamples" => {
                if let Some(v) = extract_attr(tag, "Value") { m.leading_settling = v.parse().unwrap_or(0); }
            }
            "NofQuantisationLevels" => {
                if let Some(v) = extract_attr(tag, "Value") { m.quantisation_levels = v.parse().unwrap_or(0.0); }
            }
            "XStart" => {
                if let Some(v) = extract_attr(tag, "Value") { m.x_start = v.parse().unwrap_or(0.0); }
            }
            "XStop" => {
                if let Some(v) = extract_attr(tag, "Value") { m.x_stop = v.parse().unwrap_or(0.0); }
            }
            "SignalFormat" => {
                if let Some(v) = extract_attr(tag, "Value") { m.signal_format = v.to_string(); }
            }
            "MultiChannelExport" => {
                if let Some(v) = extract_attr(tag, "Value") {
                    m.multi_channel_export = v == "eRS_ONOFF_ON";
                }
            }
            "Source" => {
                if let Some(v) = extract_attr(tag, "Value") { m.source = v.to_string(); }
            }
            "VerticalScale" => {
                if let Some(v) = extract_attr(tag, "Value") { m.vertical_scale = v.parse().unwrap_or(0.0); }
                if let Some(v) = extract_attr(tag, "StepFactor") { m.vertical_scale_step_factor = v.parse().unwrap_or(1.0); }
            }
            "VerticalPosition" => {
                if let Some(v) = extract_attr(tag, "Value") { m.vertical_position = v.parse().unwrap_or(0.0); }
            }
            "VerticalOffset" => {
                if let Some(v) = extract_attr(tag, "Value") { m.vertical_offset = v.parse().unwrap_or(0.0); }
            }
            "MultiChannelSource" => {
                for (i, slot) in m.mc_source.iter_mut().enumerate() {
                    if let Some(v) = extract_attr(tag, &format!("I_{i}")) { *slot = v.to_string(); }
                }
            }
            "MultiChannelExportState" => {
                for (i, slot) in m.mc_state.iter_mut().enumerate() {
                    if let Some(v) = extract_attr(tag, &format!("I_{i}")) { *slot = v.to_string(); }
                }
            }
            "MultiChannelVerticalScale" => {
                if let Some(v) = extract_attr(tag, "StepFactor") { m.mc_scale_step_factor = v.parse().unwrap_or(1.0); }
                for (i, slot) in m.mc_scale.iter_mut().enumerate() {
                    if let Some(v) = extract_attr(tag, &format!("I_{i}")) { *slot = v.parse().unwrap_or(0.0); }
                }
            }
            "MultiChannelVerticalPosition" => {
                for (i, slot) in m.mc_position.iter_mut().enumerate() {
                    if let Some(v) = extract_attr(tag, &format!("I_{i}")) { *slot = v.parse().unwrap_or(0.0); }
                }
            }
            "MultiChannelVerticalOffset" => {
                for (i, slot) in m.mc_offset.iter_mut().enumerate() {
                    if let Some(v) = extract_attr(tag, &format!("I_{i}")) { *slot = v.parse().unwrap_or(0.0); }
                }
            }
            _ => {}
        }
    }

    if m.multi_channel_export {
        let mut slot = 0usize;
        for i in 0..4 {
            if m.mc_state[i] == "eRS_ONOFF_ON" {
                let ch = source_to_channel(&m.mc_source[i]).unwrap_or(i + 1);
                m.active_channels.push(RsChannel {
                    channel: ch,
                    slot,
                    vertical_scale: m.mc_scale[i],
                    vertical_offset: m.mc_offset[i],
                    vertical_position: m.mc_position[i],
                    step_factor: m.mc_scale_step_factor,
                    quantisation_levels: m.quantisation_levels.max(1.0),
                });
                slot += 1;
            }
        }
    } else {
        let ch = source_to_channel(&m.source).unwrap_or(1);
        m.active_channels.push(RsChannel {
            channel: ch,
            slot: 0,
            vertical_scale: m.vertical_scale,
            vertical_offset: m.vertical_offset,
            vertical_position: m.vertical_position,
            step_factor: m.vertical_scale_step_factor,
            quantisation_levels: m.quantisation_levels.max(1.0),
        });
    }

    if m.active_channels.is_empty() {
        bail!("R&S: XML metadata declares no active channels");
    }
    Ok(m)
}

fn source_to_channel(src: &str) -> Option<usize> {
    // Sources look like "eRS_SIGNAL_SOURCE_CH3_TR1"; pull the digit after CH.
    let after_ch = src.find("CH")?;
    let tail = &src[after_ch + 2..];
    let mut digits = String::new();
    for c in tail.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            break;
        }
    }
    digits.parse().ok()
}

/// Find `attr="value"` inside a tag fragment (the slice between two `<`
/// markers). Returns the unescaped value, or `None` if not present.
fn extract_attr<'a>(tag: &'a str, attr: &str) -> Option<&'a str> {
    // Look for ` attr="` then everything up to the next `"`.
    let needle_with_space = format!(" {attr}=\"");
    let pos = tag.find(&needle_with_space).map(|p| p + needle_with_space.len())?;
    let rest = &tag[pos..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Used externally to identify the integer vs. float path. Returns true
/// when the integer voltage-conversion formula must be applied.
pub fn format_is_integer(format_code: u32) -> bool {
    matches!(format_code, FORMAT_INT8 | FORMAT_INT16)
}
