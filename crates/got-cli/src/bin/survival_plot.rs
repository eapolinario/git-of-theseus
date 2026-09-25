//! `git-of-theseus-survival-plot` — Rust port of
//! `git_of_theseus.survival_plot.survival_plot_cmdline`.

use std::path::PathBuf;

use anyhow::Result;
use clap::{error::ErrorKind, CommandFactory, Parser};
use got_plot::{survival_plot, SurvivalPlotOptions};

#[derive(Debug, Parser)]
#[command(
    name = "git-of-theseus-survival-plot",
    version,
    about = "Plot survival plot"
)]
struct Cli {
    /// Plot exponential fit (red overlay).
    #[arg(long = "exp-fit", default_value_t = false)]
    exp_fit: bool,

    /// Display plot (currently a no-op; the file is always written).
    #[arg(long, default_value_t = false)]
    display: bool,

    /// Output file to store results.
    #[arg(long, default_value = "survival_plot.png")]
    outfile: PathBuf,

    /// Number of years on the x axis.
    #[arg(long, default_value_t = 5.0)]
    years: f64,

    /// Not supported: survival plots use elapsed age, not calendar time.
    #[arg(long)]
    events: Option<PathBuf>,

    /// One or more `survival.json` input files.
    input_fns: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.events.is_some() {
        Cli::command()
            .error(
                ErrorKind::ArgumentConflict,
                "--events is not supported: survival plots use elapsed age, not calendar time",
            )
            .exit();
    }
    let opts = SurvivalPlotOptions {
        inputs: cli.input_fns,
        output: cli.outfile,
        exp_fit: cli.exp_fit,
        years: cli.years,
    };
    let path = survival_plot(&opts)?;
    println!("Writing output to {}", path.display());
    if cli.display {
        eprintln!("note: --display is not yet implemented in the Rust port");
    }
    Ok(())
}
