use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use wfm_oxide::WfmFile;

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
    },
    /// Convert a waveform file into CSV or NPY.
    Convert {
        /// Path to a .wfm or .isf capture.
        path: PathBuf,
        /// Output file. Format is inferred from the extension when --format is omitted.
        #[arg(short, long)]
        output: PathBuf,
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
        Cmd::Info { path } => cmd_info(&path),
        Cmd::Convert { path, output, channel, start, length, format } => {
            let format = match format {
                Some(f) => f,
                None => infer_format(&output)?,
            };
            cmd_convert(&path, &output, channel, start, length, format)
        }
    }
}

fn cmd_info(path: &Path) -> Result<()> {
    let wfm = open_wfm(path)?;
    println!("File:     {}", path.display());
    println!("Model:    {}", wfm.model_number);
    println!("Firmware: {}", wfm.firmware_version);
    let channels = wfm.enabled_channels();
    println!("Channels: {} enabled ({})", channels.len(), format_channel_list(&channels));
    for ch in &channels {
        match wfm.extract_channel(*ch, None, None) {
            Ok(v) => println!("  CH{}: {} samples", ch, v.len()),
            Err(e) => println!("  CH{}: <error: {}>", ch, e),
        }
    }
    Ok(())
}

fn cmd_convert(
    path: &Path,
    output: &Path,
    channel: Option<usize>,
    start: Option<usize>,
    length: Option<usize>,
    format: Format,
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

    match format {
        Format::Csv => write_csv(output, &channels, &data, n_samples)?,
        Format::Npy => write_npy(output, &channels, &data, n_samples)?,
    }
    eprintln!(
        "Wrote {} samples × {} channel(s) to {}",
        n_samples,
        channels.len(),
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

fn write_csv(output: &Path, channels: &[usize], data: &[Vec<f32>], n_samples: usize) -> Result<()> {
    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut w = BufWriter::new(file);

    let header: Vec<String> = channels.iter().map(|c| format!("CH{}", c)).collect();
    writeln!(w, "{}", header.join(","))?;

    for i in 0..n_samples {
        for (j, col) in data.iter().enumerate() {
            if j > 0 {
                w.write_all(b",")?;
            }
            write!(w, "{}", col[i])?;
        }
        w.write_all(b"\n")?;
    }
    w.flush()?;
    Ok(())
}

fn write_npy(output: &Path, channels: &[usize], data: &[Vec<f32>], n_samples: usize) -> Result<()> {
    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut w = BufWriter::new(file);

    let shape = if channels.len() == 1 {
        format!("({},)", n_samples)
    } else {
        // (n_samples, n_channels) — row-major, one sample per row, columns by channel.
        format!("({}, {})", n_samples, channels.len())
    };
    let header_dict = format!(
        "{{'descr': '<f4', 'fortran_order': False, 'shape': {}, }}",
        shape
    );

    // NPY 1.0: 6-byte magic + 2-byte version + 2-byte u16 header length + header text.
    // Total preamble length (10 bytes + header_dict bytes + trailing newline) must be a
    // multiple of 64; pad with spaces before the newline.
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

    if channels.len() == 1 {
        let bytes: &[u8] = f32_slice_as_bytes(&data[0]);
        w.write_all(bytes)?;
    } else {
        // Interleave row-major: row i is [ch0[i], ch1[i], ...].
        for i in 0..n_samples {
            for col in data {
                w.write_all(&col[i].to_le_bytes())?;
            }
        }
    }
    w.flush()?;
    Ok(())
}

fn f32_slice_as_bytes(slice: &[f32]) -> &[u8] {
    let len = std::mem::size_of_val(slice);
    unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const u8, len) }
}
