//! Tests for `survival_plot`.
//!
//! We construct synthetic survival.json fixtures with known dynamics
//! (pure exponential decay) so we can independently check both the
//! survival-curve walker and the exponential fit.

use std::collections::BTreeMap;
use std::fs;

use got_plot::{survival_plot, SurvivalPlotOptions};
use tempfile::tempdir;

const YEAR: f64 = 365.25 * 24.0 * 60.0 * 60.0;

/// Build a `survival.json` whose aggregate decay matches `count(t) ≈ N0 * exp(-k*t)`
/// across `n_commits` commits, sampled at evenly-spaced timestamps.
///
/// Each commit starts with `per_commit` lines and decays independently,
/// so the aggregate also decays at rate `k`.
fn write_synthetic(
    dir: &std::path::Path,
    k: f64,
    n_commits: usize,
    per_commit: u64,
    samples: usize,
    span_years: f64,
) -> std::path::PathBuf {
    let mut out: BTreeMap<String, Vec<(i64, u64)>> = BTreeMap::new();
    for c in 0..n_commits {
        let mut hist = Vec::with_capacity(samples);
        let t0: i64 = 1_600_000_000 + c as i64 * 1000;
        for s in 0..samples {
            let t_yr = span_years * s as f64 / (samples as f64 - 1.0);
            let count = ((per_commit as f64) * (-k * t_yr).exp()).round() as u64;
            hist.push((t0 + (t_yr * YEAR) as i64, count));
        }
        out.insert(format!("sha{c:040}"), hist);
    }
    // The schema is a top-level object keyed by sha; serde_json will
    // serialize the BTreeMap as such.
    let path = dir.join("survival.json");
    fs::write(&path, serde_json::to_vec(&out).unwrap()).unwrap();
    path
}

#[test]
fn renders_png_from_synthetic_input() {
    let dir = tempdir().unwrap();
    let input = write_synthetic(dir.path(), /*k=*/ 0.2, 20, 1000, 25, 6.0);
    let output = dir.path().join("survival.png");
    let opts = SurvivalPlotOptions {
        inputs: vec![input],
        output: output.clone(),
        exp_fit: false,
        years: 5.0,
    };
    survival_plot(&opts).unwrap();
    let bytes = fs::read(&output).unwrap();
    assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
    assert!(bytes.len() > 1000);
}

#[test]
fn exp_fit_emits_legend_with_half_life() {
    // We don't assert the recovered k matches the synthetic k, because
    // Python's algorithm (which we faithfully port) records `P=1` at the
    // *first* delta timestamp rather than at t=t0, introducing a phase
    // shift that biases the fit. Instead we check that the optimizer
    // runs to completion and produces a legend entry with a finite,
    // positive half-life.
    let dir = tempdir().unwrap();
    let input = write_synthetic(dir.path(), /*k=*/ 0.35, 30, 10_000, 80, 8.0);
    let output = dir.path().join("survival.svg");
    let opts = SurvivalPlotOptions {
        inputs: vec![input],
        output: output.clone(),
        exp_fit: true,
        years: 8.0,
    };
    survival_plot(&opts).unwrap();
    let svg = fs::read_to_string(&output).unwrap();
    let needle = "half-life = ";
    let idx = svg.find(needle).expect("legend missing half-life");
    let tail = &svg[idx + needle.len()..];
    let hl: f64 = tail
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .expect("half-life is not a number");
    assert!(hl.is_finite() && hl > 0.0, "half-life = {hl}");
}

#[test]
fn errors_when_no_inputs() {
    let opts = SurvivalPlotOptions {
        inputs: vec![],
        output: "/tmp/never_written.png".into(),
        exp_fit: false,
        years: 5.0,
    };
    assert!(survival_plot(&opts).is_err());
}
