//! `git-of-theseus-line-plot-rs` — Rust port of
//! `git_of_theseus.line_plot.line_plot_cmdline`.
//!
//! Flags mirror the Python CLI so this is a near drop-in replacement.
//! Note: `--display` is currently a no-op (matplotlib's interactive
//! window has no direct equivalent in headless Rust); the file is still
//! written.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use got_plot::{line_plot, LinePlotOptions};

#[derive(Debug, Parser)]
#[command(
    name = "git-of-theseus-line-plot-rs",
    version,
    about = "Plot line plot"
)]
struct Cli {
    /// Display plot (currently a no-op; the file is always written).
    #[arg(long, default_value_t = false)]
    display: bool,

    /// Output file to store results.
    #[arg(long, default_value = "line_plot.png")]
    outfile: PathBuf,

    /// Max number of dataseries.
    #[arg(long = "max-n", default_value_t = 20)]
    max_n: usize,

    /// Plot the share of each, so it adds up to 100%.
    #[arg(long, default_value_t = false)]
    normalize: bool,

    /// Input JSON file (e.g. cohorts.json, exts.json).
    input_fn: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let opts = LinePlotOptions {
        input: cli.input_fn,
        output: cli.outfile,
        max_n: cli.max_n,
        normalize: cli.normalize,
    };
    let path = line_plot(&opts)?;
    println!("Writing output to {}", path.display());
    if cli.display {
        eprintln!("note: --display is not yet implemented in the Rust port");
    }
    Ok(())
}
