//! JSON output writer matching the schema produced by
//! `git_of_theseus/analyze.py`. The Python plot scripts read these files
//! verbatim, so we reproduce the field names and shapes exactly:
//!
//! Curve files (`cohorts.json`, `exts.json`, `authors.json`, `dirs.json`,
//! `domains.json`):
//!     {"y": [[int, ...], ...], "ts": ["YYYY-MM-DDTHH:MM:SS", ...], "labels": [str, ...]}
//!
//! Survival file (`survival.json`):
//!     {"<commit-sha>": [[unix_ts, surviving_lines], ...], ...}

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;

use crate::analyze::SurvivalSeries;

/// Writes a curve JSON file (e.g. `cohorts.json`).
///
/// `series` is a map from the curve label to its y-axis time series. The
/// labels are emitted in their natural `BTreeMap` order, matching Python's
/// `sorted(...)` behaviour.
pub fn write_curve_json<P: AsRef<Path>>(
    path: P,
    series: &BTreeMap<String, Vec<u64>>,
    timestamps: &[DateTime<Utc>],
    label_format: impl Fn(&str) -> String,
) -> Result<()> {
    let labels: Vec<String> = series.keys().map(|k| label_format(k)).collect();
    let y: Vec<&Vec<u64>> = series.values().collect();
    // Python emits naive ISO timestamps via datetime.utcfromtimestamp(...).isoformat(),
    // which has no timezone suffix. chrono's NaiveDateTime::format("%Y-%m-%dT%H:%M:%S")
    // reproduces that exactly.
    let ts: Vec<String> = timestamps
        .iter()
        .map(|t| t.naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string())
        .collect();

    let payload = json!({
        "y": y,
        "ts": ts,
        "labels": labels,
    });
    write_json(path, &payload)
}

/// Writes the `survival.json` file.
pub fn write_survival_json<P: AsRef<Path>>(
    path: P,
    survival: &BTreeMap<String, SurvivalSeries>,
) -> Result<()> {
    write_json(path, survival)
}

fn write_json<P: AsRef<Path>, T: Serialize>(path: P, value: &T) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating output directory {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec(value)?;
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
