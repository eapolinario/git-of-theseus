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
use crate::events::{save_event_svg, EventLayout, SvgTooltip, MARGIN, X_LABEL_AREA, Y_LABEL_AREA};

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
    /// Optional YAML manifest of external calendar events.
    pub events: Option<PathBuf>,
}

impl Default for LinePlotOptions {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            output: PathBuf::from("line_plot.png"),
            max_n: 20,
            normalize: false,
            events: None,
        }
    }
}

/// Render the line plot. Returns the path that was written.
pub fn line_plot(opts: &LinePlotOptions) -> Result<PathBuf> {
    let curve = Curve::load(&opts.input)?;
    // line_plot.py computes y_sums BEFORE trimming and divides the kept
    // top-N rows by those untrimmed column totals, so kept shares do not
    // sum to 100% when some series were dropped. We must capture the
    // pre-trim sums here, before `top_n` discards rows.
    let pretrim_sums = if opts.normalize {
        Some(curve.column_sums())
    } else {
        None
    };
    let curve = curve.top_n(opts.max_n, /* aggregate_other = */ false);
    let series_f64: Vec<Vec<f64>> = if let Some(sums) = pretrim_sums.as_ref() {
        curve.normalize_by(sums)
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
    let events = EventLayout::load(opts.events.as_deref(), t_min, t_max)?;

    let path: &Path = opts.output.as_ref();
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

    if is_svg {
        let mut svg = String::new();
        let backend = SVGBackend::with_string(&mut svg, events.dimensions());
        let tooltips = draw(
            backend.into_drawing_area(),
            &curve.labels,
            &ts_utc,
            &series_f64,
            t_min,
            t_max,
            y_max,
            opts.normalize,
            &events,
        )?;
        save_event_svg(path, svg, &tooltips)?;
    } else {
        let backend = BitMapBackend::new(path, events.dimensions());
        draw(
            backend.into_drawing_area(),
            &curve.labels,
            &ts_utc,
            &series_f64,
            t_min,
            t_max,
            y_max,
            opts.normalize,
            &events,
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
    events: &EventLayout,
) -> Result<Vec<SvgTooltip>>
where
    DB: DrawingBackend,
    DB::ErrorType: 'static,
{
    let background = if events.is_active() {
        WHITE
    } else {
        RGBColor(229, 229, 229)
    };
    root.fill(&background).map_err(|e| anyhow!("fill: {e}"))?;
    let plot = events.plot_area(&root);
    if events.is_active() {
        plot.fill(&RGBColor(229, 229, 229))
            .map_err(|e| anyhow!("fill plot: {e}"))?;
    }

    let mut chart = ChartBuilder::on(&plot)
        .margin(MARGIN)
        .x_label_area_size(X_LABEL_AREA)
        .y_label_area_size(Y_LABEL_AREA)
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

    let tooltips = events.draw(&root, chart.plotting_area(), y_max)?;

    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperLeft)
        .background_style(WHITE.mix(0.85))
        .border_style(BLACK.mix(0.4))
        .label_font(("sans-serif", 16))
        .draw()
        .map_err(|e| anyhow!("draw legend: {e}"))?;

    root.present().map_err(|e| anyhow!("present: {e}"))?;
    Ok(tooltips)
}
