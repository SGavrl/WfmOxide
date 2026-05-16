use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use wfm_oxide::{TimeAxis, WfmFile};

#[derive(Parser, Debug)]
#[command(name = "wfm-oxide")]
#[command(about = "Fast reader and converter for proprietary oscilloscope waveform files")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Print model, firmware, enabled channels, and sample count for a waveform file.
    Info {
        /// Path to a .wfm or .isf capture.
        path: PathBuf,
        /// Print machine-readable JSON instead of the human-friendly summary.
        #[arg(long)]
        json: bool,
    },
    /// Convert one or more waveform files into CSV or NPY.
    Convert {
        /// Paths to one or more .wfm or .isf captures.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Output file. Required when converting a single capture; rejected when --out-dir is given.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Directory to write outputs into when converting multiple captures. Each input
        /// becomes <out-dir>/<stem>.<ext> with extension inferred from --format (or --output for single inputs).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Extract only the named 1-based channel. Omit to extract every enabled channel.
        #[arg(short, long)]
        channel: Option<usize>,
        /// Skip the first N samples per channel before writing.
        #[arg(long)]
        start: Option<usize>,
        /// Write at most N samples per channel (applied after --start).
        #[arg(long)]
        length: Option<usize>,
        /// Override the output format. Defaults to the --output extension.
        #[arg(short, long)]
        format: Option<Format>,
        /// Suppress the leading time column. Time is included by default when the
        /// format exposes a time axis.
        #[arg(long)]
        no_time: bool,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Format {
    Csv,
    Npy,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Info { path, json } => cmd_info(&path, json),
        Cmd::Convert { paths, output, out_dir, channel, start, length, format, no_time } => {
            cmd_convert(paths, output, out_dir, channel, start, length, format, no_time)
        }
    }
}

fn cmd_info(path: &Path, json: bool) -> Result<()> {
    let wfm = open_wfm(path)?;
    let channels = wfm.enabled_channels();
    let time = wfm.time_axis();
    let n_pts_first = channels
        .first()
        .and_then(|ch| wfm.channel_sample_count(*ch).ok())
        .unwrap_or(0);

    if json {
        #[derive(Serialize)]
        struct TimeAxisOut { x_origin: f64, x_increment: f64, sample_rate: f64 }
        #[derive(Serialize)]
        struct ChannelOut {
            channel: usize,
            samples: usize,
            vertical_scale: Option<f32>,
            vertical_offset: Option<f32>,
            inverted: Option<bool>,
            coupling: Option<&'static str>,
            probe_ratio: Option<f32>,
        }
        #[derive(Serialize)]
        struct InfoOut<'a> {
            file: String,
            model: &'a str,
            firmware: &'a str,
            enabled_channels: &'a [usize],
            time_axis: Option<TimeAxisOut>,
            channels: Vec<ChannelOut>,
        }

        let channels_out: Vec<ChannelOut> = channels.iter().map(|&ch| {
            let n = wfm.channel_sample_count(ch).unwrap_or(0);
            let meta = wfm.channel_metadata(ch);
            ChannelOut {
                channel: ch,
                samples: n,
                vertical_scale: meta.as_ref().map(|m| m.vertical_scale),
                vertical_offset: meta.as_ref().map(|m| m.vertical_offset),
                inverted: meta.as_ref().map(|m| m.inverted),
                coupling: meta.as_ref().and_then(|m| m.coupling),
                probe_ratio: meta.as_ref().and_then(|m| m.probe_ratio),
            }
        }).collect();
        let out = InfoOut {
            file: path.display().to_string(),
            model: &wfm.model_number,
            firmware: &wfm.firmware_version,
            enabled_channels: &channels,
            time_axis: time.map(|t| TimeAxisOut {
                x_origin: t.x_origin,
                x_increment: t.x_increment,
                sample_rate: t.sample_rate(),
            }),
            channels: channels_out,
        };
        let json_text = serde_json::to_string_pretty(&out)?;
        println!("{}", json_text);
        return Ok(());
    }

    println!("File:     {}", path.display());
    println!("Model:    {}", wfm.model_number);
    println!("Firmware: {}", wfm.firmware_version);
    println!("Channels: {} enabled ({})", channels.len(), format_channel_list(&channels));

    if let Some(t) = time {
        println!("Sample rate: {}", format_si(t.sample_rate(), "Sa/s"));
        println!("Sample step: {}", format_si(t.x_increment, "s"));
        if n_pts_first > 0 {
            println!("Capture:     {} ({} samples)", format_si(n_pts_first as f64 * t.x_increment, "s"), n_pts_first);
        }
        println!("Time origin: {}", format_si(t.x_origin, "s"));
    } else {
        println!("Time axis:   <not available for this format>");
    }

    for ch in &channels {
        let n = wfm.channel_sample_count(*ch).unwrap_or(0);
        match wfm.channel_metadata(*ch) {
            Some(m) => {
                let coupling = m.coupling.unwrap_or("?");
                let probe = m
                    .probe_ratio
                    .map(|r| format!("{}x", r))
                    .unwrap_or_else(|| "?".to_string());
                let inv = if m.inverted { ", inverted" } else { "" };
                println!(
                    "  CH{}: {} samples, {}/div, offset {}, coupling {}, probe {}{}",
                    ch,
                    n,
                    format_si(m.vertical_scale as f64, "V"),
                    format_si(m.vertical_offset as f64, "V"),
                    coupling,
                    probe,
                    inv,
                );
            }
            None => println!("  CH{}: {} samples", ch, n),
        }
    }
    Ok(())
}

fn format_si(value: f64, unit: &str) -> String {
    if value == 0.0 || !value.is_finite() {
        return format!("{} {}", value, unit);
    }
    let abs = value.abs();
    let (scale, prefix) = if abs >= 1e9       { (1e-9,  "G")
                          } else if abs >= 1e6  { (1e-6,  "M")
                          } else if abs >= 1e3  { (1e-3,  "k")
                          } else if abs >= 1.0  { (1.0,   "")
                          } else if abs >= 1e-3 { (1e3,   "m")
                          } else if abs >= 1e-6 { (1e6,   "µ")
                          } else if abs >= 1e-9 { (1e9,   "n")
                          } else                { (1e12,  "p") };
    format!("{:.4} {}{}", value * scale, prefix, unit)
}

#[allow(clippy::too_many_arguments)]
fn cmd_convert(
    paths: Vec<PathBuf>,
    output: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    channel: Option<usize>,
    start: Option<usize>,
    length: Option<usize>,
    format: Option<Format>,
    no_time: bool,
) -> Result<()> {
    if paths.len() > 1 && output.is_some() {
        bail!("--output is only valid with a single input; use --out-dir for batches");
    }
    if paths.len() == 1 && out_dir.is_none() && output.is_none() {
        bail!("missing --output (single input) or --out-dir (batch)");
    }
    if let Some(dir) = out_dir.as_ref() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    let multi = paths.len() > 1;
    let mut n_failed = 0usize;
    for path in &paths {
        let result = (|| -> Result<()> {
            let resolved_format = match format {
                Some(f) => f,
                None => match &output {
                    Some(o) => infer_format(o)?,
                    None => bail!("--format is required when batch-converting without an inferable extension"),
                },
            };
            let resolved_output = match (&output, &out_dir) {
                (Some(o), _) => o.clone(),
                (None, Some(dir)) => {
                    let stem = path.file_stem()
                        .ok_or_else(|| anyhow!("cannot derive stem from {}", path.display()))?;
                    dir.join(stem).with_extension(format_extension(resolved_format))
                }
                (None, None) => unreachable!("validated above"),
            };
            if let Some(parent) = resolved_output.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
            }
            convert_one(path, &resolved_output, channel, start, length, resolved_format, no_time)
        })();
        if let Err(e) = result {
            if !multi {
                return Err(e);
            }
            eprintln!("Error: {}: {:#}", path.display(), e);
            n_failed += 1;
        }
    }
    if n_failed > 0 {
        bail!("{} of {} file(s) failed", n_failed, paths.len());
    }
    Ok(())
}

fn format_extension(f: Format) -> &'static str {
    match f { Format::Csv => "csv", Format::Npy => "npy" }
}

fn convert_one(
    path: &Path,
    output: &Path,
    channel: Option<usize>,
    start: Option<usize>,
    length: Option<usize>,
    format: Format,
    no_time: bool,
) -> Result<()> {
    let wfm = open_wfm(path)?;
    let enabled = wfm.enabled_channels();
    if enabled.is_empty() {
        bail!("No enabled channels in {}", path.display());
    }

    let channels: Vec<usize> = match channel {
        Some(ch) => {
            if !enabled.contains(&ch) {
                bail!("Channel {} is not enabled in {} (enabled: {})", ch, path.display(), format_channel_list(&enabled));
            }
            vec![ch]
        }
        None => enabled,
    };

    let mut data: Vec<Vec<f32>> = Vec::with_capacity(channels.len());
    for ch in &channels {
        let v = wfm.extract_channel(*ch, start, length)
            .with_context(|| format!("extracting CH{}", ch))?;
        data.push(v);
    }

    let n_samples = data[0].len();
    if data.iter().any(|c| c.len() != n_samples) {
        bail!("Channel sample counts differ; this format mixes channel lengths and is not yet supported by `convert`.");
    }

    let time_axis = if no_time { None } else { wfm.time_axis() };
    let start_pt = start.unwrap_or(0);

    match format {
        Format::Csv => write_csv(output, &channels, &data, n_samples, time_axis, start_pt)?,
        Format::Npy => write_npy(output, &channels, &data, n_samples, time_axis, start_pt)?,
    }
    let time_note = if time_axis.is_some() { " + time" } else { "" };
    eprintln!(
        "Wrote {} samples × {} channel(s){} to {}",
        n_samples,
        channels.len(),
        time_note,
        output.display()
    );
    Ok(())
}

fn open_wfm(path: &Path) -> Result<WfmFile> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 path: {:?}", path))?;
    WfmFile::open(path_str).with_context(|| format!("opening {}", path.display()))
}

fn infer_format(output: &Path) -> Result<Format> {
    let ext = output
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("csv") => Ok(Format::Csv),
        Some("npy") => Ok(Format::Npy),
        Some(other) => Err(anyhow!(
            "cannot infer format from .{} — pass --format csv|npy",
            other
        )),
        None => Err(anyhow!(
            "no extension on {} — pass --format csv|npy",
            output.display()
        )),
    }
}

fn format_channel_list(channels: &[usize]) -> String {
    channels
        .iter()
        .map(|c| format!("CH{}", c))
        .collect::<Vec<_>>()
        .join(", ")
}

fn write_csv(
    output: &Path,
    channels: &[usize],
    data: &[Vec<f32>],
    n_samples: usize,
    time_axis: Option<TimeAxis>,
    start_pt: usize,
) -> Result<()> {
    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut w = BufWriter::new(file);

    let mut header_cols: Vec<String> = Vec::with_capacity(channels.len() + 1);
    if time_axis.is_some() {
        header_cols.push("time".to_string());
    }
    header_cols.extend(channels.iter().map(|c| format!("CH{}", c)));
    writeln!(w, "{}", header_cols.join(","))?;

    for i in 0..n_samples {
        let mut col_idx = 0;
        if let Some(t) = time_axis {
            write!(w, "{}", t.x_origin + (start_pt + i) as f64 * t.x_increment)?;
            col_idx = 1;
        }
        for col in data.iter() {
            if col_idx > 0 {
                w.write_all(b",")?;
            }
            write!(w, "{}", col[i])?;
            col_idx += 1;
        }
        w.write_all(b"\n")?;
    }
    w.flush()?;
    Ok(())
}

fn write_npy(
    output: &Path,
    channels: &[usize],
    data: &[Vec<f32>],
    n_samples: usize,
    time_axis: Option<TimeAxis>,
    start_pt: usize,
) -> Result<()> {
    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut w = BufWriter::new(file);

    // dtype + shape:
    //   time on:   structured 1D record [('time','<f8'), ('CH1','<f4'), ...]
    //   time off, 1 channel:    plain '<f4', shape (n,)
    //   time off, k channels:   plain '<f4', shape (n, k)
    let (descr, shape) = match (time_axis.is_some(), channels.len()) {
        (true, _) => {
            let mut fields = Vec::with_capacity(channels.len() + 1);
            fields.push("('time', '<f8')".to_string());
            for c in channels {
                fields.push(format!("('CH{}', '<f4')", c));
            }
            (format!("[{}]", fields.join(", ")), format!("({},)", n_samples))
        }
        (false, 1) => ("'<f4'".to_string(), format!("({},)", n_samples)),
        (false, k) => ("'<f4'".to_string(), format!("({}, {})", n_samples, k)),
    };

    let header_dict = format!(
        "{{'descr': {}, 'fortran_order': False, 'shape': {}, }}",
        descr, shape
    );

    let prefix_len = 10 + header_dict.len() + 1;
    let pad_len = (64 - (prefix_len % 64)) % 64;
    let header_padded = format!("{}{}\n", header_dict, " ".repeat(pad_len));
    let header_bytes = header_padded.as_bytes();
    if header_bytes.len() > u16::MAX as usize {
        bail!("NPY header too long for v1.0 format");
    }

    w.write_all(b"\x93NUMPY")?;
    w.write_all(&[0x01, 0x00])?;
    w.write_all(&(header_bytes.len() as u16).to_le_bytes())?;
    w.write_all(header_bytes)?;

    match (time_axis, channels.len()) {
        // Structured: 8 bytes time + 4 bytes per channel, per record.
        (Some(t), _) => {
            for i in 0..n_samples {
                let ts = t.x_origin + (start_pt + i) as f64 * t.x_increment;
                w.write_all(&ts.to_le_bytes())?;
                for col in data {
                    w.write_all(&col[i].to_le_bytes())?;
                }
            }
        }
        // No time, single channel — 1D f32.
        (None, 1) => {
            for &v in &data[0] {
                w.write_all(&v.to_le_bytes())?;
            }
        }
        // No time, multi channel — 2D f32 (n_samples, n_channels).
        (None, _) => {
            for i in 0..n_samples {
                for col in data {
                    w.write_all(&col[i].to_le_bytes())?;
                }
            }
        }
    }
    w.flush()?;
    Ok(())
}
