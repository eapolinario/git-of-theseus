//! Plot generation for git-of-theseus.
//!
//! This crate is a Rust port of the Python plot scripts in
//! `git_of_theseus/line_plot.py`, `stack_plot.py`, and `survival_plot.py`.
//! It reads the JSON files produced by the analyzer (see
//! `got_core::output`) and renders PNG/SVG charts.
//!
//! Calendar line and stack plots also accept external event manifests.

pub mod colors;
pub mod curve;
pub mod events;
pub mod line;
pub mod stack;
pub mod survival;

pub use line::{line_plot, LinePlotOptions};
pub use stack::{stack_plot, StackPlotOptions};
pub use survival::{survival_plot, SurvivalPlotOptions};
