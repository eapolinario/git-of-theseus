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

use std::collections::{BTreeSet, HashMap};
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

    /// Loads and merges one or more curve JSON files, aligning them onto a
    /// shared, sorted timestamp axis. Mirrors
    /// `git_of_theseus.utils.load_curve_inputs`:
    ///
    /// - With a single path, this is equivalent to [`Curve::load`].
    /// - With multiple paths, the union of all timestamps (sorted) becomes
    ///   the new `ts` axis. Each source series is re-sampled onto that
    ///   axis by carrying its last known value forward (starting at 0
    ///   before the source's first sample), and its label is prefixed
    ///   with the source's directory name (e.g. `"repo-one: alpha"`).
    pub fn load_many<P: AsRef<Path>>(paths: &[P]) -> Result<Self> {
        anyhow::ensure!(!paths.is_empty(), "at least one input file is required");
        if paths.len() == 1 {
            return Self::load(&paths[0]);
        }

        struct Loaded {
            source: String,
            ts: Vec<NaiveDateTime>,
            y: Vec<Vec<u64>>,
            labels: Vec<String>,
        }

        let mut loaded = Vec::with_capacity(paths.len());
        let mut all_ts: BTreeSet<NaiveDateTime> = BTreeSet::new();
        for p in paths {
            let path = p.as_ref();
            let single = Self::load(path)?;
            all_ts.extend(single.ts.iter().copied());
            let source = path
                .parent()
                .and_then(|d| d.file_name())
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .or_else(|| path.file_stem().and_then(|s| s.to_str()))
                .unwrap_or("input")
                .to_string();
            loaded.push(Loaded {
                source,
                ts: single.ts,
                y: single.y,
                labels: single.labels,
            });
        }

        let timestamps: Vec<NaiveDateTime> = all_ts.into_iter().collect();
        let mut labels = Vec::new();
        let mut y = Vec::new();
        for entry in loaded {
            let positions: HashMap<NaiveDateTime, usize> =
                entry.ts.iter().enumerate().map(|(i, t)| (*t, i)).collect();
            for (label, row) in entry.labels.iter().zip(entry.y.iter()) {
                let mut aligned = Vec::with_capacity(timestamps.len());
                let mut current = 0u64;
                for t in &timestamps {
                    if let Some(&idx) = positions.get(t) {
                        current = row[idx];
                    }
                    aligned.push(current);
                }
                labels.push(format!("{}: {}", entry.source, label));
                y.push(aligned);
            }
        }

        Ok(Curve {
            y,
            ts: timestamps,
            labels,
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

    /// Column-wise sums of `self.y`. Useful for `--normalize` parity with
    /// `line_plot.py`, which computes `y_sums = numpy.sum(y, axis=0)` on
    /// the *untrimmed* matrix and divides the kept rows by those totals
    /// (so the kept share does NOT sum to 100% when some rows were dropped).
    pub fn column_sums(&self) -> Vec<u64> {
        let cols = self.ts.len();
        let mut col_sum = vec![0u64; cols];
        for row in &self.y {
            for (i, v) in row.iter().enumerate() {
                col_sum[i] += v;
            }
        }
        col_sum
    }

    /// Convert each column to its share (in percent) of an externally
    /// supplied per-column divisor. `col_sums` must have one entry per
    /// timestamp. Used by `line_plot` to normalize the trimmed top-N
    /// rows against the *pre-trim* column totals.
    pub fn normalize_by(&self, col_sums: &[u64]) -> Vec<Vec<f64>> {
        self.y
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, &v)| {
                        if col_sums[i] == 0 {
                            0.0
                        } else {
                            100.0 * v as f64 / col_sums[i] as f64
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// Convert each column to its share (in percent) of the column sum of
    /// `self.y`. Mirrors `stack_plot.py`'s `--normalize` behaviour: there,
    /// trimming aggregates dropped rows into `"other"`, so the post-trim
    /// column sums equal the pre-trim totals.
    pub fn normalize(&self) -> Vec<Vec<f64>> {
        let col_sum = self.column_sums();
        self.normalize_by(&col_sum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn dt(y: i32, m: u32, d: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn make(rows: Vec<Vec<u64>>, labels: Vec<&str>) -> Curve {
        let ncols = rows[0].len();
        let ts = (0..ncols).map(|i| dt(2020, 1, (i + 1) as u32)).collect();
        Curve {
            y: rows,
            ts,
            labels: labels.into_iter().map(String::from).collect(),
        }
    }

    /// Regression for line_plot parity: `column_sums` captured BEFORE
    /// `top_n` must equal the untrimmed column totals; normalizing the
    /// trimmed top-N against those sums must leave the dropped rows'
    /// share missing (i.e. kept rows do NOT sum to 100%).
    #[test]
    fn line_plot_normalize_uses_pretrim_sums() {
        // 3 series of constant value, totals = 100 per column.
        // labels chosen so alphabetical-after-top sort is deterministic.
        let curve = make(
            vec![vec![10, 10, 10], vec![50, 50, 50], vec![40, 40, 40]],
            vec!["c-small", "a-big", "b-mid"],
        );

        let pretrim = curve.column_sums();
        assert_eq!(pretrim, vec![100, 100, 100]);

        // Top-2 by per-series max, then alphabetical by label.
        let trimmed = curve.top_n(2, /*aggregate_other=*/ false);
        assert_eq!(trimmed.labels, vec!["a-big", "b-mid"]);

        let norm = trimmed.normalize_by(&pretrim);
        // Kept rows should be 50% and 40% of the *original* total per
        // column, summing to 90% — not 100%. This matches Python's
        // `y = 100.0 * y / y_sums` where `y_sums` was computed pre-trim.
        for i in 0..3 {
            let col_total: f64 = norm.iter().map(|r| r[i]).sum();
            assert!(
                (col_total - 90.0).abs() < 1e-9,
                "col {i}: expected 90.0, got {col_total}"
            );
        }
        assert!((norm[0][0] - 50.0).abs() < 1e-9);
        assert!((norm[1][0] - 40.0).abs() < 1e-9);
    }

    /// stack_plot parity: with `aggregate_other=true` the dropped rows
    /// are rolled into `"other"`, so post-trim column sums equal
    /// pre-trim sums and `normalize()` produces shares that sum to 100%.
    #[test]
    fn stack_plot_normalize_with_other_sums_to_100() {
        let curve = make(
            vec![vec![10, 10, 10], vec![50, 50, 50], vec![40, 40, 40]],
            vec!["c-small", "a-big", "b-mid"],
        );
        let trimmed = curve.top_n(2, /*aggregate_other=*/ true);
        assert_eq!(trimmed.labels, vec!["a-big", "b-mid", "other"]);
        let norm = trimmed.normalize();
        for i in 0..3 {
            let total: f64 = norm.iter().map(|r| r[i]).sum();
            assert!((total - 100.0).abs() < 1e-9, "col {i}: {total}");
        }
    }

    #[test]
    fn column_sums_zero_columns_produce_zero_shares() {
        let curve = make(vec![vec![0, 5], vec![0, 5]], vec!["a", "b"]);
        let sums = curve.column_sums();
        assert_eq!(sums, vec![0, 10]);
        let norm = curve.normalize_by(&sums);
        // Zero-divisor column => 0% rather than NaN.
        assert_eq!(norm[0][0], 0.0);
        assert_eq!(norm[1][0], 0.0);
        assert!((norm[0][1] - 50.0).abs() < 1e-9);
        assert!((norm[1][1] - 50.0).abs() < 1e-9);
    }

    /// Multi-repo support (issue #16): `load_many` aligns series from
    /// several curve files onto the union of their timestamps, carrying
    /// forward the last known value, and prefixes labels with the source
    /// directory name.
    #[test]
    fn load_many_aligns_repositories_and_prefixes_labels() {
        let dir = tempfile::tempdir().unwrap();
        let first_dir = dir.path().join("first");
        let second_dir = dir.path().join("second");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::create_dir_all(&second_dir).unwrap();

        let first = first_dir.join("cohorts.json");
        let second = second_dir.join("cohorts.json");
        std::fs::write(
            &first,
            r#"{"y": [[2, 4]], "ts": ["2020-01-01T00:00:00", "2020-01-03T00:00:00"], "labels": ["Code added in 2020"]}"#,
        )
        .unwrap();
        std::fs::write(
            &second,
            r#"{"y": [[3, 5]], "ts": ["2020-01-02T00:00:00", "2020-01-03T00:00:00"], "labels": ["Code added in 2020"]}"#,
        )
        .unwrap();

        let curve = Curve::load_many(&[first, second]).unwrap();

        assert_eq!(
            curve.ts,
            vec![dt(2020, 1, 1), dt(2020, 1, 2), dt(2020, 1, 3)]
        );
        assert_eq!(
            curve.labels,
            vec!["first: Code added in 2020", "second: Code added in 2020"]
        );
        assert_eq!(curve.y, vec![vec![2, 2, 4], vec![0, 3, 5]]);
    }

    /// A single input path should behave exactly like `load`, with no
    /// label prefixing.
    #[test]
    fn load_many_with_a_single_path_does_not_prefix_labels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cohorts.json");
        std::fs::write(
            &path,
            r#"{"y": [[1, 2]], "ts": ["2020-01-01T00:00:00", "2020-01-02T00:00:00"], "labels": ["alpha"]}"#,
        )
        .unwrap();

        let curve = Curve::load_many(&[path]).unwrap();
        assert_eq!(curve.labels, vec!["alpha"]);
    }
}
