//! CLI entry point for the Rust port of `git-of-theseus-analyze`.
//!
//! Flags mirror `git_of_theseus.analyze.analyze_cmdline` so this binary is
//! a near drop-in replacement; it writes the same JSON files which the
//! existing Python plot CLIs (`git-of-theseus-stack-plot`,
//! `-line-plot`, `-survival-plot`) consume unchanged.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use got_core::{analyze, AnalyzeOptions, DEFAULT_INTERVAL_SECS};

/// Analyze a git repository's history and emit JSON time-series files.
#[derive(Debug, Parser)]
#[command(name = "git-of-theseus-analyze-rs", version, about)]
struct Cli {
    /// A chrono/strftime format string (e.g. "%Y") for cohort labels.
    #[arg(long, default_value = "%Y")]
    cohortfm: String,

    /// Minimum number of seconds between sampled commits.
    #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
    interval: i64,

    /// File patterns to ignore (glob; can be repeated).
    #[arg(long, action = clap::ArgAction::Append)]
    ignore: Vec<String>,

    /// File patterns that must match (glob; can be repeated).
    #[arg(long, action = clap::ArgAction::Append)]
    only: Vec<String>,

    /// Output directory for JSON files.
    #[arg(long, default_value = ".")]
    outdir: PathBuf,

    /// Branch to analyze.
    #[arg(long, default_value = "master")]
    branch: String,

    /// Ignore whitespace changes when running blame.
    #[arg(long, default_value_t = false)]
    ignore_whitespace: bool,

    /// Include all filetypes (otherwise only known code filetypes are analyzed).
    #[arg(long, default_value_t = false)]
    all_filetypes: bool,

    /// Suppress progress output.
    #[arg(long, default_value_t = false)]
    quiet: bool,

    /// Number of worker threads to use for blaming.
    #[arg(long, default_value_t = default_procs())]
    procs: usize,

    /// Path to the git repository to analyze.
    repo_dir: PathBuf,
}

fn default_procs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let options = AnalyzeOptions {
        repo_dir: cli.repo_dir,
        branch: cli.branch,
        cohort_format: cli.cohortfm,
        interval_secs: cli.interval,
        only: cli.only,
        ignore: cli.ignore,
        all_filetypes: cli.all_filetypes,
        ignore_whitespace: cli.ignore_whitespace,
        procs: cli.procs,
        quiet: cli.quiet,
        outdir: cli.outdir,
    };
    analyze(&options)?;
    Ok(())
}
