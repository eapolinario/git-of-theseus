//! CLI entry point for the Rust port of `git-of-theseus-analyze`.
//!
//! Flags mirror `git_of_theseus.analyze.analyze_cmdline` so this binary is
//! a near drop-in replacement; it writes the same JSON files which the
//! existing Python plot CLIs (`git-of-theseus-stack-plot`,
//! `-line-plot`, `-survival-plot`) consume unchanged.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use got_core::{analyze_many, AnalyzeOptions, DEFAULT_INTERVAL_SECS};

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

    /// When multiple repositories are given, combine them into a single
    /// set of output files instead of one subdirectory per repository.
    /// Series for labels shared across repositories (e.g. the same
    /// author or file extension) are summed after being aligned onto a
    /// shared timeline. Ignored with a single repository.
    #[arg(long, default_value_t = false)]
    merge: bool,

    /// Enable detailed timing measurements (prints timing statistics after analysis).
    #[arg(long, default_value_t = false)]
    measure_time: bool,

    /// Path(s) to the git repository/repositories to analyze. When more
    /// than one is given, each is analyzed independently and its JSON
    /// output is written to a subdirectory of `--outdir` named after the
    /// repository's directory name (unless `--merge` is set).
    #[arg(required = true, num_args = 1..)]
    repo_dir: Vec<PathBuf>,
}

fn default_procs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let options = AnalyzeOptions {
        repo_dir: PathBuf::new(), // overridden per-repository by analyze_many
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
        merge: cli.merge,
        measure_time: cli.measure_time,
        timing: Default::default(),
    };
    analyze_many(&cli.repo_dir, &options)?;
    
    if cli.measure_time {
        print_timing_stats(&options.timing);
    }
    Ok(())
}

fn print_timing_stats(timing: &got_core::analyze::TimingStats) {
    let blame_us = timing.blame_time_us.load(std::sync::atomic::Ordering::Relaxed);
    let post_blame_us = timing.post_blame_time_us.load(std::sync::atomic::Ordering::Relaxed);
    let fastdiff_us = timing.fastdiff_time_us.load(std::sync::atomic::Ordering::Relaxed);
    let tree_discovery_us = timing.tree_discovery_time_us.load(std::sync::atomic::Ordering::Relaxed);
    let commit_walk_us = timing.commit_walk_time_us.load(std::sync::atomic::Ordering::Relaxed);
    let files_blamed = timing.files_blamed.load(std::sync::atomic::Ordering::Relaxed);
    
    let blame_ms = blame_us as f64 / 1000.0;
    let post_blame_ms = post_blame_us as f64 / 1000.0;
    let fastdiff_ms = fastdiff_us as f64 / 1000.0;
    let tree_discovery_ms = tree_discovery_us as f64 / 1000.0;
    let commit_walk_ms = commit_walk_us as f64 / 1000.0;
    let total_ms = blame_ms + post_blame_ms + fastdiff_ms + tree_discovery_ms + commit_walk_ms;
    
    eprintln!("\n=== Timing Statistics ===");
    eprintln!("Blame (I/O):              {:8.1}ms ({:5.1}%)", blame_ms, (blame_ms / total_ms) * 100.0);
    eprintln!("Post-blame (compute):    {:8.1}ms ({:5.1}%)", post_blame_ms, (post_blame_ms / total_ms) * 100.0);
    eprintln!("Fast-diff:               {:8.1}ms ({:5.1}%)", fastdiff_ms, (fastdiff_ms / total_ms) * 100.0);
    eprintln!("Tree discovery (I/O):    {:8.1}ms ({:5.1}%)", tree_discovery_ms, (tree_discovery_ms / total_ms) * 100.0);
    eprintln!("Commit walk (I/O):       {:8.1}ms ({:5.1}%)", commit_walk_ms, (commit_walk_ms / total_ms) * 100.0);
    eprintln!("---");
    eprintln!("Total:                   {:8.1}ms", total_ms);
    eprintln!("Files blamed:            {}", files_blamed);
    eprintln!("\nI/O operations: {:.1}ms ({:.1}%)", 
        blame_ms + tree_discovery_ms + commit_walk_ms,
        ((blame_ms + tree_discovery_ms + commit_walk_ms) / total_ms) * 100.0);
    eprintln!("Computation:    {:.1}ms ({:.1}%)", 
        post_blame_ms + fastdiff_ms,
        ((post_blame_ms + fastdiff_ms) / total_ms) * 100.0);
}
