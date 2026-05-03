//! Core analysis library for git-of-theseus, a Rust port of the original
//! Python implementation in `git_of_theseus/analyze.py`.
//!
//! High-level entry points:
//! - [`analyze::analyze`] runs the full analysis and writes JSON output
//!   files compatible with the Python `git-of-theseus-stack-plot`,
//!   `git-of-theseus-line-plot`, and `git-of-theseus-survival-plot` CLIs.
//! - [`analyze::analyze_in_memory`] returns the analysis result without
//!   writing any files (useful for tests and library embedding).
//!
//! The Python and Rust implementations live side-by-side in this
//! repository during the migration. Behavioural differences and deferred
//! features are documented at the top of `analyze.rs`.

pub mod analyze;
pub mod cohort;
pub mod filetypes;
pub mod output;
pub mod path_filter;

pub use analyze::{
    analyze, analyze_in_memory, write_outputs, AnalyzeOptions, AnalyzeResult, SurvivalSeries,
    DEFAULT_INTERVAL_SECS,
};
