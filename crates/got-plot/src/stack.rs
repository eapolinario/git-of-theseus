//! Stack plot — port of `git_of_theseus/stack_plot.py`.
//!
//! Renders a stacked area chart: one filled region per label, summing
//! vertically. Like `stack_plot.py`, when there are more than `max_n`
//! labels we keep the top-N by per-series maximum and roll the rest
//! into a final `"other"` band.
//!
//! Output format is determined by the file extension of `output`:
//! `.svg` produces SVG, anything else produces PNG.
//!
//! Accepts one or more input curve files; with multiple inputs, series are
//! aligned onto a shared timestamp axis and labels are prefixed with the
//! source repository's directory name (see [`Curve::load_many`]).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use plotters::prelude::*;

use crate::colors::generate_n_colors;
use crate::curve::Curve;
use crate::events::{save_event_svg, EventLayout, SvgTooltip, MARGIN, X_LABEL_AREA, Y_LABEL_AREA};

/// Options for [`stack_plot`], mirroring the flags of
/// `git-of-theseus-stack-plot`.
#[derive(Debug, Clone)]
pub struct StackPlotOptions {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    /// Maximum number of bands to draw; extras are summed into `"other"`.
    pub max_n: usize,
    pub normalize: bool,
    /// Optional YAML manifest of external calendar events.
    pub events: Option<PathBuf>,
}

impl Default for StackPlotOptions {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            output: PathBuf::from("stack_plot.png"),
            max_n: 20,
            normalize: false,
            events: None,
        }
    }
}

/// Render the stack plot. Returns the path that was written.
pub fn stack_plot(opts: &StackPlotOptions) -> Result<PathBuf> {
    let curve = Curve::load_many(&opts.inputs)?;
    let curve = curve.top_n(opts.max_n, /* aggregate_other = */ true);
    let series_f64: Vec<Vec<f64>> = if opts.normalize {
        curve.normalize()
    } else {
        curve
            .y
            .iter()
            .map(|row| row.iter().map(|&v| v as f64).collect())
            .collect()
    };

    let n_pts = curve.ts.len();
    if n_pts == 0 {
        return Err(anyhow!("curve has no timestamps"));
    }

    // Cumulative sums: cum[i][t] = sum of series[0..=i] at timestamp t.
    let mut cum: Vec<Vec<f64>> = Vec::with_capacity(series_f64.len());
    let mut acc = vec![0.0_f64; n_pts];
    for row in &series_f64 {
        for (i, v) in row.iter().enumerate() {
            acc[i] += v;
        }
        cum.push(acc.clone());
    }

    let y_max = if opts.normalize {
        100.0
    } else {
        cum.last()
            .and_then(|r| {
                r.iter()
                    .copied()
                    .fold(None, |m, v| Some(m.map_or(v, |m: f64| m.max(v))))
            })
            .unwrap_or(1.0)
            * 1.05
    }
    .max(1.0);

    let ts_utc: Vec<DateTime<Utc>> = curve.ts.iter().copied().map(to_utc).collect();
    let t_min = *ts_utc.first().unwrap();
    let t_max = *ts_utc.last().unwrap();
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
            &cum,
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
            &cum,
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
    cum: &[Vec<f64>],
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

    // For each band i, draw the polygon between cum[i-1] (lower) and
    // cum[i] (upper). Lower for the first band is the constant zero.
    for (i, ((label, upper), color)) in labels
        .iter()
        .zip(cum.iter())
        .zip(palette.iter())
        .enumerate()
    {
        let color = *color;
        let mut poly: Vec<(DateTime<Utc>, f64)> = Vec::with_capacity(2 * ts.len());
        // Upper boundary, left -> right.
        for (t, y) in ts.iter().zip(upper.iter()) {
            poly.push((*t, *y));
        }
        // Lower boundary, right -> left.
        if i == 0 {
            for t in ts.iter().rev() {
                poly.push((*t, 0.0));
            }
        } else {
            let lower = &cum[i - 1];
            for (t, y) in ts.iter().zip(lower.iter()).rev() {
                poly.push((*t, *y));
            }
        }
        chart
            .draw_series(std::iter::once(Polygon::new(poly, color.filled())))
            .map_err(|e| anyhow!("draw band {label:?}: {e}"))?
            .label(label.clone())
            .legend(move |(x, y)| Rectangle::new([(x, y - 6), (x + 18, y + 6)], color.filled()));
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
