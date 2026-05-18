//! Survival plot — port of `git_of_theseus/survival_plot.py`.
//!
//! Reads one or more `survival.json` files (commit -> [[unix_ts, count], …])
//! and renders a single chart showing the percentage of code lines still
//! present `n` years after they were introduced.
//!
//! For multiple inputs, each file is drawn as its own labeled line; the
//! label is the immediate parent directory name (matching Python's
//! `os.path.split(fn)[-2]`).
//!
//! Optional `--exp-fit` fits an exponential decay `100 * exp(-k*t)` to the
//! aggregate of all inputs and overlays it as a red dashed line. Python
//! uses `scipy.optimize.fmin` (Nelder–Mead); we use golden-section search
//! over `k` to avoid a `scipy`/`argmin` dependency. The two methods can
//! converge to slightly different `k` (typically <1e-3 relative); the
//! reported half-life prints to stdout for comparison.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use plotters::prelude::*;

use crate::colors::generate_n_colors;

/// Seconds in a Julian year (matches Python `YEAR = 365.25 * 24 * 60 * 60`).
const SECONDS_PER_YEAR: f64 = 365.25 * 24.0 * 60.0 * 60.0;

#[derive(Debug, Clone)]
pub struct SurvivalPlotOptions {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub exp_fit: bool,
    /// X-axis upper bound, in years.
    pub years: f64,
}

impl Default for SurvivalPlotOptions {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output: PathBuf::from("survival_plot.png"),
            exp_fit: false,
            years: 5.0,
        }
    }
}

/// Per-input result from delta accumulation; kept around so [`fit_k`] can
/// re-walk the histories without re-reading the JSON.
struct InputData {
    label: Option<String>,
    /// Total starting count (`total_n` in Python before mutations).
    total_n: u64,
    /// Map `t - t0` (seconds) -> (delta_k, delta_n).
    deltas: BTreeMap<i64, (i64, i64)>,
}

pub fn survival_plot(opts: &SurvivalPlotOptions) -> Result<PathBuf> {
    if opts.inputs.is_empty() {
        return Err(anyhow!("no input files"));
    }

    // Load + accumulate deltas from each input.
    let mut datasets: Vec<InputData> = Vec::with_capacity(opts.inputs.len());
    for fn_ in &opts.inputs {
        datasets.push(load_input(fn_)?);
    }

    // For each input, walk the deltas and produce (xs, ys) for plotting.
    let mut series: Vec<LabeledCurve> = Vec::with_capacity(datasets.len());
    for d in &datasets {
        series.push((d.label.clone(), survival_curve(d)));
    }

    // Optional exp-fit overlay.
    let fit_overlay = if opts.exp_fit {
        let k = fit_k(&datasets);
        let half_life = std::f64::consts::LN_2 / k;
        println!("exp-fit: k = {k:.6}, half-life = {half_life:.2} years");
        let mut pts = Vec::with_capacity(1000);
        for i in 0..1000 {
            let t = opts.years * i as f64 / 999.0;
            pts.push((t, 100.0 * (-k * t).exp()));
        }
        Some((half_life, pts))
    } else {
        None
    };

    let path: &Path = opts.output.as_ref();
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

    if is_svg {
        let backend = SVGBackend::new(path, (1560, 960));
        draw(
            backend.into_drawing_area(),
            &series,
            &fit_overlay,
            opts.years,
        )?;
    } else {
        let backend = BitMapBackend::new(path, (1560, 960));
        draw(
            backend.into_drawing_area(),
            &series,
            &fit_overlay,
            opts.years,
        )?;
    }

    Ok(opts.output.clone())
}

fn load_input(path: &Path) -> Result<InputData> {
    let bytes =
        fs::read(path).with_context(|| format!("reading survival file {}", path.display()))?;
    // Schema: { "<sha>": [[unix_ts: i64, count: u64], ...], ... }
    let raw: BTreeMap<String, Vec<(i64, u64)>> = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing survival file {}", path.display()))?;

    println!("reading {}", path.display());
    println!("counting {} commits", raw.len());

    let mut deltas: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
    let mut total_n: u64 = 0;

    for history in raw.values() {
        if history.is_empty() {
            continue;
        }
        let (t0, orig_count) = history[0];
        total_n += orig_count;
        let mut last_count = orig_count as i64;
        for &(t, count) in &history[1..] {
            let entry = deltas.entry(t - t0).or_insert((0, 0));
            entry.0 += count as i64 - last_count;
            last_count = count as i64;
        }
        let last_t = history.last().unwrap().0;
        let entry = deltas.entry(last_t - t0).or_insert((0, 0));
        entry.0 += -last_count;
        entry.1 += -(orig_count as i64);
    }

    println!("adding {} deltas...", deltas.len());

    // Label = name of the immediate parent directory, if any (matches
    // Python's `os.path.split(fn)[-2]`, used only when len(parts) > 1).
    let label = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    Ok(InputData {
        label,
        total_n,
        deltas,
    })
}

/// Walks the deltas to produce the (years, %) survival curve; stops once
/// P drops below 5% (matches Python).
fn survival_curve(d: &InputData) -> Vec<(f64, f64)> {
    let mut total_n = d.total_n as i64;
    let mut p = 1.0_f64;
    let mut out = Vec::with_capacity(d.deltas.len());
    for (&t, &(delta_k, delta_n)) in &d.deltas {
        out.push((t as f64 / SECONDS_PER_YEAR, 100.0 * p));
        if total_n > 0 {
            p *= 1.0 + delta_k as f64 / total_n as f64;
        }
        total_n += delta_n;
        if p < 0.05 {
            break;
        }
    }
    out
}

/// Loss function for the exponential fit, summed over all inputs.
/// Mirrors the Python `fit(k)` closure exactly: at each delta step,
/// `pred` and `actual` both use the *current* (mutating) `total_n`,
/// and `total_n` is decremented by `delta_n` after each step. The
/// optimizer therefore sees the same loss surface as scipy's
/// `fmin(fit, 0.5)` does in `survival_plot.py`.
fn fit_loss(k: f64, datasets: &[InputData]) -> f64 {
    let mut loss = 0.0_f64;
    for d in datasets {
        let mut total_n = d.total_n as i64;
        let mut p = 1.0_f64;
        for (&t, &(delta_k, delta_n)) in &d.deltas {
            let n = total_n as f64;
            let pred = n * (-k * t as f64 / SECONDS_PER_YEAR).exp();
            let actual = n * p;
            loss += (actual - pred).powi(2);
            if total_n > 0 {
                p *= 1.0 + delta_k as f64 / total_n as f64;
            }
            total_n += delta_n;
        }
    }
    loss
}

/// Golden-section search for the minimizer of [`fit_loss`] over `k`.
/// Bracket `[1e-4, 20.0]` covers half-lives from ~0.03 to ~7000 years.
fn fit_k(datasets: &[InputData]) -> f64 {
    let mut lo = 1.0e-4_f64;
    let mut hi = 20.0_f64;
    let phi: f64 = (1.0 + 5.0_f64.sqrt()) / 2.0;
    let invphi = 1.0 / phi;
    let mut c = hi - (hi - lo) * invphi;
    let mut d = lo + (hi - lo) * invphi;
    let mut fc = fit_loss(c, datasets);
    let mut fd = fit_loss(d, datasets);
    for _ in 0..100 {
        if fc < fd {
            hi = d;
            d = c;
            fd = fc;
            c = hi - (hi - lo) * invphi;
            fc = fit_loss(c, datasets);
        } else {
            lo = c;
            c = d;
            fc = fd;
            d = lo + (hi - lo) * invphi;
            fd = fit_loss(d, datasets);
        }
        if (hi - lo).abs() < 1e-9 {
            break;
        }
    }
    0.5 * (lo + hi)
}

/// (label, points) for one rendered survival curve.
type LabeledCurve = (Option<String>, Vec<(f64, f64)>);

fn draw<DB>(
    root: DrawingArea<DB, plotters::coord::Shift>,
    series: &[LabeledCurve],
    fit_overlay: &Option<(f64, Vec<(f64, f64)>)>,
    years: f64,
) -> Result<()>
where
    DB: DrawingBackend,
    DB::ErrorType: 'static,
{
    root.fill(&RGBColor(229, 229, 229))
        .map_err(|e| anyhow!("fill: {e}"))?;

    let mut chart = ChartBuilder::on(&root)
        .margin(30)
        .caption(
            "% of lines still present in code after n years",
            ("sans-serif", 24),
        )
        .x_label_area_size(60)
        .y_label_area_size(80)
        .build_cartesian_2d(0.0_f64..years, 0.0_f64..100.0_f64)
        .map_err(|e| anyhow!("build chart: {e}"))?;

    chart
        .configure_mesh()
        .light_line_style(WHITE)
        .bold_line_style(WHITE.mix(0.8))
        .axis_style(BLACK.mix(0.5))
        .x_desc("Years")
        .y_desc("%")
        .label_style(("sans-serif", 18))
        .draw()
        .map_err(|e| anyhow!("draw mesh: {e}"))?;

    // Decide series colors. With --exp-fit Python draws the data in
    // dark gray and the fit in red; otherwise it uses matplotlib's
    // default cycle. We approximate the latter with our palette.
    let palette = generate_n_colors(series.len().max(1));
    let dark_gray = RGBColor(80, 80, 80);

    for (i, (label, pts)) in series.iter().enumerate() {
        let color = if fit_overlay.is_some() {
            dark_gray
        } else {
            palette[i]
        };
        let pts_clipped: Vec<(f64, f64)> =
            pts.iter().copied().filter(|(x, _)| *x <= years).collect();
        let series_color = color;
        let series_handle = chart
            .draw_series(LineSeries::new(pts_clipped, color.stroke_width(2)))
            .map_err(|e| anyhow!("draw curve {label:?}: {e}"))?;
        if let Some(label) = label.as_deref() {
            series_handle
                .label(label.to_string())
                .legend(move |(x, y)| {
                    PathElement::new(vec![(x, y), (x + 20, y)], series_color.stroke_width(2))
                });
        }
    }

    if let Some((half_life, pts)) = fit_overlay {
        let red = RGBColor(220, 50, 50);
        chart
            .draw_series(LineSeries::new(pts.clone(), red.stroke_width(2)))
            .map_err(|e| anyhow!("draw fit: {e}"))?
            .label(format!(
                "Exponential fit, half-life = {:.2} years",
                half_life
            ))
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], red.stroke_width(2)));
    }

    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperRight)
        .background_style(WHITE.mix(0.85))
        .border_style(BLACK.mix(0.4))
        .label_font(("sans-serif", 16))
        .draw()
        .map_err(|e| anyhow!("draw legend: {e}"))?;

    root.present().map_err(|e| anyhow!("present: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one InputData synthesizing exponential decay at rate `k_true`
    /// across `n_commits` identical commits.
    fn synth(
        k_true: f64,
        n_commits: usize,
        per_commit: u64,
        samples: usize,
        span_years: f64,
    ) -> InputData {
        let mut deltas: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
        let mut total_n: u64 = 0;
        for c in 0..n_commits {
            let t0: i64 = 1_600_000_000 + c as i64 * 1000;
            total_n += per_commit;
            let mut last_count = per_commit as i64;
            let mut last_t = t0;
            for s in 1..samples {
                let t_yr = span_years * s as f64 / (samples as f64 - 1.0);
                let count = ((per_commit as f64) * (-k_true * t_yr).exp()).round() as i64;
                let t = t0 + (t_yr * SECONDS_PER_YEAR) as i64;
                let entry = deltas.entry(t - t0).or_insert((0, 0));
                entry.0 += count - last_count;
                last_count = count;
                last_t = t;
            }
            let entry = deltas.entry(last_t - t0).or_insert((0, 0));
            entry.0 += -last_count;
            entry.1 += -(per_commit as i64);
        }
        InputData {
            label: None,
            total_n,
            deltas,
        }
    }

    #[test]
    fn fit_k_is_local_minimum_of_loss() {
        let data = synth(0.35, 30, 10_000, 80, 8.0);
        let datasets = vec![data];
        let k_hat = fit_k(&datasets);
        let l_hat = fit_loss(k_hat, &datasets);
        let l_lo = fit_loss(k_hat * 0.9, &datasets);
        let l_hi = fit_loss(k_hat * 1.1, &datasets);
        assert!(
            l_hat < l_lo && l_hat < l_hi,
            "k_hat={k_hat:.4} not a local min: loss(0.9*k)={l_lo:.3e}, loss(k)={l_hat:.3e}, loss(1.1*k)={l_hi:.3e}"
        );
        assert!(k_hat > 0.0 && k_hat.is_finite());
    }

    #[test]
    fn survival_curve_starts_at_p_eq_1() {
        let data = synth(0.2, 5, 1000, 20, 5.0);
        let pts = survival_curve(&data);
        assert!(!pts.is_empty());
        // First emitted y is 100 * P with P initialized to 1.
        assert!((pts[0].1 - 100.0).abs() < 1e-9);
        // Curve should be monotonically non-increasing (decay only).
        for w in pts.windows(2) {
            assert!(
                w[1].1 <= w[0].1 + 1e-6,
                "non-monotone: {:?} -> {:?}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    #[ignore]
    fn diag_print_losses_for_real_input() {
        // cargo test -p got-plot survival::tests::diag -- --ignored --nocapture
        let path = match std::env::var("SURVIVAL_JSON") {
            Ok(p) => p,
            Err(_) => return,
        };
        let data = load_input(std::path::Path::new(&path)).unwrap();
        let datasets = vec![data];
        for &k in &[0.5_f64, 1.0, 1.5, 1.994286, 2.5, 2.704944, 3.0, 4.0] {
            println!("k={k:.6}  loss={:.4e}", fit_loss(k, &datasets));
        }
        let k_hat = fit_k(&datasets);
        println!(
            "fit_k -> k={k_hat:.6}  loss={:.4e}",
            fit_loss(k_hat, &datasets)
        );
    }
}
