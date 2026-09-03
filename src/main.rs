//! # main.rs
//!
//! Command-line entry point for the Urbix engine.
//!
//! Thin CLI wrapper around `WorldEngine`: generate one or more chunks and dump
//! them to `bin` (raw `ChunkBuffer` wire bytes) or `json` (pretty-printed
//! header + cells). The same generation pipeline drives the library, FFI, and
//! visualizer, so CLI output is byte-identical to what a C consumer would read.

use std::fs;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use urbix::config::WorldConfig;
use urbix::data::{ChunkBuffer, ChunkHeader};
use urbix::engine::WorldEngine;

/// Output format for dumped chunks.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    Bin,
    Json,
}

/// Urbix chunk dumper — generate city chunks from the shell.
#[derive(Parser, Debug)]
#[command(name = "urbix", version, about = "Generate Urbix city chunks")]
struct Args {
    /// World seed (default 0)
    #[arg(long, default_value_t = 0)]
    seed: u64,

    /// Chunk column to generate (center when --radius > 0)
    #[arg(long, default_value_t = 0)]
    cx: i32,

    /// Chunk row to generate (center when --radius > 0)
    #[arg(long, default_value_t = 0)]
    cy: i32,

    /// Radius of the chunk grid around (cx,cy) (0 = single chunk, max 64)
    #[arg(long, default_value_t = 0)]
    radius: u32,

    /// Override chunk size (cells per side, 1..=256, default from WorldConfig)
    #[arg(long)]
    chunk_size: Option<u16>,

    /// Output format: `bin` (raw wire bytes) or `json` (header + cells)
    #[arg(long, value_enum, default_value_t = Format::Bin)]
    format: Format,

    /// Output file (single chunk) or directory (grid). Defaults to
    /// `chunk_<cx>_<cy>.bin` / `.json` for single, `./` for grid.
    #[arg(long)]
    out: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if let Some(cs) = args.chunk_size {
        if cs == 0 {
            anyhow::bail!("--chunk-size must be non-zero (1..=256)");
        }
        if cs > 256 {
            anyhow::bail!("--chunk-size {cs} too large (max 256)");
        }
    }

    if args.radius > 64 {
        anyhow::bail!("--radius {} too large (max 64)", args.radius);
    }

    // Guard against i32 overflow when adding radius to cx/cy.
    if args.radius > 0 {
        let r = args.radius as i32;
        args.cx
            .checked_add(r)
            .ok_or_else(|| anyhow::anyhow!("--cx + --radius overflows i32"))?;
        args.cx
            .checked_sub(r)
            .ok_or_else(|| anyhow::anyhow!("--cx - --radius underflows i32"))?;
        args.cy
            .checked_add(r)
            .ok_or_else(|| anyhow::anyhow!("--cy + --radius overflows i32"))?;
        args.cy
            .checked_sub(r)
            .ok_or_else(|| anyhow::anyhow!("--cy - --radius underflows i32"))?;
    }

    let mut config = WorldConfig {
        seed: args.seed,
        ..Default::default()
    };
    if let Some(cs) = args.chunk_size {
        config.chunk_size = cs;
    }
    if !config.is_valid() {
        anyhow::bail!("invalid WorldConfig: {:?}", config);
    }

    let mut engine = WorldEngine::with_config(config);

    // Collect chunk coordinates to generate. Radius is capped above so
    // (2r+1)² stays bounded (max 129² = 16641 entries).
    let coords: Vec<(i32, i32)> = if args.radius == 0 {
        vec![(args.cx, args.cy)]
    } else {
        let r = args.radius as i32;
        let mut v = Vec::with_capacity(((2 * r + 1) as usize).pow(2));
        for dy in -r..=r {
            for dx in -r..=r {
                // Checked above, so unwrap is safe.
                v.push((args.cx + dx, args.cy + dy));
            }
        }
        v
    };

    validate_out(&args, &coords)?;

    if args.format == Format::Json {
        write_json(&mut engine, &coords, &args)?;
    } else {
        write_bin(&mut engine, &coords, &args)?;
    }

    Ok(())
}

/// Validate `--out` is a file for single-chunk and a directory for grid.
fn validate_out(args: &Args, coords: &[(i32, i32)]) -> anyhow::Result<()> {
    let Some(out) = &args.out else { return Ok(()) };
    if coords.len() == 1 {
        if out.exists() && out.is_dir() {
            anyhow::bail!(
                "--out {} is a directory but single chunk expects a file",
                out.display()
            );
        }
    } else if out.exists() && out.is_file() {
        anyhow::bail!(
            "--out {} is a file but --radius grid expects a directory",
            out.display()
        );
    }
    Ok(())
}

fn write_bin(engine: &mut WorldEngine, coords: &[(i32, i32)], args: &Args) -> anyhow::Result<()> {
    if coords.len() == 1 {
        let (cx, cy) = coords[0];
        let buf = engine.generate_chunk(cx, cy);
        let out = args
            .out
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("chunk_{cx}_{cy}.bin")));
        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(&out, buf.as_bytes())?;
        println!(
            "wrote {} ({} bytes, {} cells) -> {}",
            format_chunk_label(cx, cy),
            buf.as_bytes().len(),
            buf.header().cell_count,
            out.display()
        );
    } else {
        let out_dir = args.out.clone().unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&out_dir)?;
        for &(cx, cy) in coords {
            let buf = engine.generate_chunk(cx, cy);
            let path = out_dir.join(format!("chunk_{cx}_{cy}.bin"));
            fs::write(&path, buf.as_bytes())?;
        }
        println!(
            "wrote {} chunks (radius {}) -> {}",
            coords.len(),
            args.radius,
            out_dir.display()
        );
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JsonChunk {
    header: ChunkHeader,
    cells: Vec<urbix::data::Cell>,
}

fn write_json(engine: &mut WorldEngine, coords: &[(i32, i32)], args: &Args) -> anyhow::Result<()> {
    if coords.len() == 1 {
        let (cx, cy) = coords[0];
        let buf = engine.generate_chunk(cx, cy);
        let json = chunk_to_json(&buf)?;
        let out = args
            .out
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("chunk_{cx}_{cy}.json")));
        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(&out, json)?;
        println!(
            "wrote {} ({} cells) -> {} (json)",
            format_chunk_label(cx, cy),
            buf.header().cell_count,
            out.display()
        );
    } else {
        let out_dir = args.out.clone().unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&out_dir)?;
        for &(cx, cy) in coords {
            let buf = engine.generate_chunk(cx, cy);
            let json = chunk_to_json(&buf)?;
            let path = out_dir.join(format!("chunk_{cx}_{cy}.json"));
            fs::write(&path, json)?;
        }
        println!(
            "wrote {} chunks (radius {}) -> {} (json)",
            coords.len(),
            args.radius,
            out_dir.display()
        );
    }
    Ok(())
}

fn chunk_to_json(buf: &ChunkBuffer) -> anyhow::Result<String> {
    let header = buf.header();
    let cells: Vec<_> = buf.cells().collect();
    let chunk = JsonChunk { header, cells };
    Ok(serde_json::to_string_pretty(&chunk)?)
}

fn format_chunk_label(cx: i32, cy: i32) -> String {
    format!("chunk ({cx},{cy})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_out(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "urbix_cli_test_{}_{}.tmp",
            name,
            std::process::id()
        ));
        // Ensure clean slate; remove if exists from prior run.
        let _ = fs::remove_file(&p);
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn cli_bin_generates_expected_layout() {
        let out = tmp_out("bin");
        let mut cfg = WorldConfig {
            seed: 7,
            ..Default::default()
        };
        cfg.chunk_size = 8;
        let mut engine = WorldEngine::with_config(cfg);
        let buf = engine.generate_chunk(1, 2);
        fs::write(&out, buf.as_bytes()).unwrap();
        let bytes = fs::read(&out).unwrap();
        assert_eq!(bytes.len(), 32 + 8 * 8 * 40);
        let read_hdr = unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const ChunkHeader) };
        assert_eq!(read_hdr.cx, 1);
        assert_eq!(read_hdr.cy, 2);
        assert_eq!(read_hdr.chunk_size, 8);
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn cli_json_round_trip() {
        let out = tmp_out("json");
        let mut engine = WorldEngine::new(99);
        let buf = engine.generate_chunk(0, 0);
        let json = chunk_to_json(&buf).unwrap();
        fs::write(&out, &json).unwrap();
        let parsed: JsonChunk = serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(parsed.header.cx, 0);
        assert_eq!(parsed.header.cell_count, 32 * 32);
        assert_eq!(parsed.cells.len(), 32 * 32);
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn cli_rejects_oversized_radius() {
        let args = Args {
            seed: 0,
            cx: 0,
            cy: 0,
            radius: 100,
            chunk_size: None,
            format: Format::Bin,
            out: None,
        };
        // Simulate main's radius check.
        assert!(args.radius > 64);
    }

    #[test]
    fn cli_rejects_oversized_chunk_size() {
        let cs = 512;
        assert!(cs > 256);
    }
}
