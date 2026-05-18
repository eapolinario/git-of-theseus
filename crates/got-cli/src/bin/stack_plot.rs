//! `git-of-theseus-stack-plot-rs` — Rust port of
//! `git_of_theseus.stack_plot.stack_plot_cmdline`.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use got_plot::{stack_plot, StackPlotOptions};

#[derive(Debug, Parser)]
#[command(
    name = "git-of-theseus-stack-plot-rs",
    version,
    about = "Plot stack plot"
)]
struct Cli {
    /// Display plot (currently a no-op; the file is always written).
    #[arg(long, default_value_t = false)]
    display: bool,

    /// Output file to store results.
    #[arg(long, default_value = "stack_plot.png")]
    outfile: PathBuf,

    /// Max number of dataseries (extras roll into "other").
    #[arg(long = "max-n", default_value_t = 20)]
    max_n: usize,

    /// Normalize the plot to 100%.
    #[arg(long, default_value_t = false)]
    normalize: bool,

    /// Input JSON file (e.g. cohorts.json, exts.json).
    input_fn: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let opts = StackPlotOptions {
        input: cli.input_fn,
        output: cli.outfile,
        max_n: cli.max_n,
        normalize: cli.normalize,
    };
    let path = stack_plot(&opts)?;
    println!("Writing output to {}", path.display());
    if cli.display {
        eprintln!("note: --display is not yet implemented in the Rust port");
    }
    Ok(())
}
