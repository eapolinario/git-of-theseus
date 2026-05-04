//! Loading and pre-processing of curve JSON files (the output of
//! `got-cli` for cohorts/exts/authors/dirs/domains).
//!
//! Schema (matching `got_core::output::write_curve_json`):
//!
//! ```json
//! {
//!   "y": [[u64, ...], ...],
//!   "ts": ["YYYY-MM-DDTHH:MM:SS", ...],
//!   "labels": [str, ...]
//! }
//! ```

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawCurve {
    y: Vec<Vec<u64>>,
    ts: Vec<String>,
    labels: Vec<String>,
}

/// In-memory representation of a curve JSON file, with timestamps parsed.
#[derive(Debug, Clone)]
pub struct Curve {
    /// One row per label, one column per timestamp.
    pub y: Vec<Vec<u64>>,
    pub ts: Vec<NaiveDateTime>,
    pub labels: Vec<String>,
}

impl Curve {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let bytes =
            fs::read(path).with_context(|| format!("reading curve file {}", path.display()))?;
        let raw: RawCurve = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing curve file {}", path.display()))?;
        let ts = raw
            .ts
            .iter()
            .map(|s| {
                NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                    .with_context(|| format!("parsing timestamp {s:?}"))
            })
            .collect::<Result<Vec<_>>>()?;

        anyhow::ensure!(
            raw.y.len() == raw.labels.len(),
            "curve file has {} y-rows but {} labels",
            raw.y.len(),
            raw.labels.len()
        );
        for (i, row) in raw.y.iter().enumerate() {
            anyhow::ensure!(
                row.len() == ts.len(),
                "y[{}] has {} points but ts has {}",
                i,
                row.len(),
                ts.len()
            );
        }

        Ok(Curve {
            y: raw.y,
            ts,
            labels: raw.labels,
        })
    }

    /// Trim to the top-N series by per-series maximum, sorted alphabetically
    /// by label. If `aggregate_other` is true, the dropped series are summed
    /// into a final "other" row (used by stack_plot); if false, they are
    /// discarded (used by line_plot).
    ///
    /// Mirrors the trimming logic in `git_of_theseus/{line,stack}_plot.py`.
    pub fn top_n(mut self, max_n: usize, aggregate_other: bool) -> Self {
        if self.y.len() <= max_n {
            return self;
        }
        // Sort all indices by max(series) descending. Python uses a stable
        // sort, so ties resolve by original index — `sort_by` in Rust is
        // also stable.
        let mut idx: Vec<usize> = (0..self.y.len()).collect();
        idx.sort_by(|&a, &b| {
            let ma = *self.y[a].iter().max().unwrap_or(&0);
            let mb = *self.y[b].iter().max().unwrap_or(&0);
            mb.cmp(&ma)
        });
        let (top, rest) = idx.split_at(max_n);
        let mut top = top.to_vec();
        // Re-sort the kept indices alphabetically by label (matches Python).
        top.sort_by(|&a, &b| self.labels[a].cmp(&self.labels[b]));

        let other_row: Option<Vec<u64>> = if aggregate_other {
            let n = self.ts.len();
            let mut acc = vec![0u64; n];
            for &j in rest {
                for (i, v) in self.y[j].iter().enumerate() {
                    acc[i] += v;
                }
            }
            Some(acc)
        } else {
            None
        };

        let mut new_y: Vec<Vec<u64>> = top
            .iter()
            .map(|&j| std::mem::take(&mut self.y[j]))
            .collect();
        let mut new_labels: Vec<String> = top
            .iter()
            .map(|&j| std::mem::take(&mut self.labels[j]))
            .collect();
        if let Some(row) = other_row {
            new_y.push(row);
            new_labels.push("other".to_string());
        }
        Curve {
            y: new_y,
            ts: self.ts,
            labels: new_labels,
        }
    }

    /// Convert each column to its share (in percent) of the column sum.
    /// Mirrors the `--normalize` behaviour of the Python plot scripts.
    ///
    /// Returns `f64` because percentages are non-integer.
    pub fn normalize(&self) -> Vec<Vec<f64>> {
        let cols = self.ts.len();
        let mut col_sum = vec![0u64; cols];
        for row in &self.y {
            for (i, v) in row.iter().enumerate() {
                col_sum[i] += v;
            }
        }
        self.y
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, &v)| {
                        if col_sum[i] == 0 {
                            0.0
                        } else {
                            100.0 * v as f64 / col_sum[i] as f64
                        }
                    })
                    .collect()
            })
            .collect()
    }
}
