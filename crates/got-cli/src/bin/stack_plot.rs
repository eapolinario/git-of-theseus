//! `git-of-theseus-stack-plot` — Rust port of
//! `git_of_theseus.stack_plot.stack_plot_cmdline`.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use got_plot::{stack_plot, StackPlotOptions};

#[derive(Debug, Parser)]
#[command(name = "git-of-theseus-stack-plot", version, about = "Plot stack plot")]
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

    /// YAML manifest of external calendar-time events.
    #[arg(long)]
    events: Option<PathBuf>,

    /// Input JSON file(s) (e.g. cohorts.json, exts.json). Pass more than
    /// one to overlay/align series from multiple repositories.
    #[arg(required = true, num_args = 1..)]
    input_fn: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let opts = StackPlotOptions {
        inputs: cli.input_fn,
        output: cli.outfile,
        max_n: cli.max_n,
        normalize: cli.normalize,
        events: cli.events,
    };
    let path = stack_plot(&opts)?;
    println!("Writing output to {}", path.display());
    if cli.display {
        eprintln!("note: --display is not yet implemented in the Rust port");
    }
    Ok(())
}
