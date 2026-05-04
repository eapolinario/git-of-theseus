//! Line plot — port of `git_of_theseus/line_plot.py`.
//!
//! Renders one line per label, optionally normalized to per-timestamp
//! share-of-total in percent. Output format is determined by the file
//! extension of `output`: `.svg` produces SVG, anything else produces PNG.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use plotters::prelude::*;

use crate::colors::generate_n_colors;
use crate::curve::Curve;

/// Options for [`line_plot`], mirroring the flags of
/// `git-of-theseus-line-plot`.
#[derive(Debug, Clone)]
pub struct LinePlotOptions {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Maximum number of series to draw; extras are dropped (not aggregated
    /// into "other" — line_plot.py drops them).
    pub max_n: usize,
    pub normalize: bool,
}

impl Default for LinePlotOptions {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            output: PathBuf::from("line_plot.png"),
            max_n: 20,
            normalize: false,
        }
    }
}

/// Render the line plot. Returns the path that was written.
pub fn line_plot(opts: &LinePlotOptions) -> Result<PathBuf> {
    let curve = Curve::load(&opts.input)?;
    let curve = curve.top_n(opts.max_n, /* aggregate_other = */ false);
    let series_f64: Vec<Vec<f64>> = if opts.normalize {
        curve.normalize()
    } else {
        curve
            .y
            .iter()
            .map(|row| row.iter().map(|&v| v as f64).collect())
            .collect()
    };

    let y_max = series_f64
        .iter()
        .flat_map(|r| r.iter().copied())
        .fold(0.0_f64, f64::max);
    let y_max = if opts.normalize {
        100.0
    } else {
        // Add a small headroom so the topmost line isn't clipped.
        (y_max * 1.05).max(1.0)
    };

    let (t_min, t_max) = match (curve.ts.first(), curve.ts.last()) {
        (Some(a), Some(b)) => (to_utc(*a), to_utc(*b)),
        _ => return Err(anyhow!("curve has no timestamps")),
    };
    let ts_utc: Vec<DateTime<Utc>> = curve.ts.iter().copied().map(to_utc).collect();

    let path: &Path = opts.output.as_ref();
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

    if is_svg {
        let backend = SVGBackend::new(path, (1920, 1440));
        draw(
            backend.into_drawing_area(),
            &curve.labels,
            &ts_utc,
            &series_f64,
            t_min,
            t_max,
            y_max,
            opts.normalize,
        )?;
    } else {
        let backend = BitMapBackend::new(path, (1920, 1440));
        draw(
            backend.into_drawing_area(),
            &curve.labels,
            &ts_utc,
            &series_f64,
            t_min,
            t_max,
            y_max,
            opts.normalize,
        )?;
    }

    Ok(opts.output.clone())
}

fn to_utc(t: NaiveDateTime) -> DateTime<Utc> {
    DateTime::<Utc>::from_naive_utc_and_offset(t, Utc)
}

#[allow(clippy::too_many_arguments)]
fn draw<DB>(
    root: DrawingArea<DB, plotters::coord::Shift>,
    labels: &[String],
    ts: &[DateTime<Utc>],
    series: &[Vec<f64>],
    t_min: DateTime<Utc>,
    t_max: DateTime<Utc>,
    y_max: f64,
    normalize: bool,
) -> Result<()>
where
    DB: DrawingBackend,
    DB::ErrorType: 'static,
{
    // ggplot-ish background.
    root.fill(&RGBColor(229, 229, 229))
        .map_err(|e| anyhow!("fill: {e}"))?;

    let mut chart = ChartBuilder::on(&root)
        .margin(30)
        .x_label_area_size(60)
        .y_label_area_size(80)
        .build_cartesian_2d(t_min..t_max, 0.0_f64..y_max)
        .map_err(|e| anyhow!("build chart: {e}"))?;

    let y_desc = if normalize {
        "Share of lines of code (%)"
    } else {
        "Lines of code"
    };

    chart
        .configure_mesh()
        .light_line_style(WHITE)
        .bold_line_style(WHITE.mix(0.8))
        .axis_style(BLACK.mix(0.5))
        .y_desc(y_desc)
        .label_style(("sans-serif", 18))
        .draw()
        .map_err(|e| anyhow!("draw mesh: {e}"))?;

    let palette = generate_n_colors(labels.len());
    for ((label, row), color) in labels.iter().zip(series.iter()).zip(palette.iter()) {
        let color = *color;
        let pts: Vec<(DateTime<Utc>, f64)> = ts.iter().copied().zip(row.iter().copied()).collect();
        chart
            .draw_series(LineSeries::new(pts, color.stroke_width(3)))
            .map_err(|e| anyhow!("draw series {label:?}: {e}"))?
            .label(label.clone())
            .legend(move |(x, y)| {
                PathElement::new(vec![(x, y), (x + 20, y)], color.stroke_width(3))
            });
    }

    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperLeft)
        .background_style(WHITE.mix(0.85))
        .border_style(BLACK.mix(0.4))
        .label_font(("sans-serif", 16))
        .draw()
        .map_err(|e| anyhow!("draw legend: {e}"))?;

    root.present().map_err(|e| anyhow!("present: {e}"))?;
    Ok(())
}
