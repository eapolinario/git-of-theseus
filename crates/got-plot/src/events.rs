//! Optional calendar events. Dates describe external events, not model use.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, ensure, Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Timelike, Utc};
use plotters::coord::{CoordTranslate, Shift};
use plotters::prelude::*;
use plotters::style::text_anchor::{HPos, Pos, VPos};
use serde_yaml_ng::{Mapping, Value};
use sha2::{Digest, Sha256};

pub(crate) const PLOT_WIDTH: u32 = 1920;
pub(crate) const PLOT_HEIGHT: u32 = 1440;
pub(crate) const MARGIN: u32 = 30;
pub(crate) const X_LABEL_AREA: u32 = 60;
pub(crate) const Y_LABEL_AREA: u32 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Announced,
    Available,
    Pilot,
    Default,
    Retired,
}

impl EventKind {
    fn parse(value: &str, index: usize) -> Result<Self> {
        match value {
            "announced" => Ok(Self::Announced),
            "available" => Ok(Self::Available),
            "pilot" => Ok(Self::Pilot),
            "default" => Ok(Self::Default),
            "retired" => Ok(Self::Retired),
            _ => bail!(
                "events[{index}].event must be one of: announced, available, default, pilot, retired"
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Announced => "announced",
            Self::Available => "available",
            Self::Pilot => "pilot",
            Self::Default => "default",
            Self::Retired => "retired",
        }
    }

    fn pattern(self) -> &'static [i32] {
        match self {
            Self::Announced => &[7, 5],
            Self::Available => &[],
            Self::Pilot => &[10, 4, 2, 4],
            Self::Default => &[2, 4],
            Self::Retired => &[10, 4, 2, 4, 2, 4],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub date: NaiveDate,
    pub provider: String,
    pub model: String,
    pub event: EventKind,
    pub scope: String,
    pub label: String,
    pub source: String,
    pub notes: Option<String>,
}

impl Event {
    fn timestamp(&self) -> DateTime<Utc> {
        self.date.and_hms_opt(0, 0, 0).unwrap().and_utc()
    }

    fn tooltip_text(&self) -> String {
        let mut text = format!(
            "{}\nDate: {}\nProvider: {}\nModel: {}\nEvent: {}\nScope: {}\nSource: {}",
            self.label,
            self.date,
            self.provider,
            self.model,
            self.event.as_str(),
            self.scope,
            self.source,
        );
        if let Some(notes) = self.notes.as_ref().filter(|notes| !notes.is_empty()) {
            text.push_str("\nNotes: ");
            text.push_str(notes);
        }
        text
    }
}

fn parse_date(value: &str, index: usize) -> Result<NaiveDate> {
    let invalid = || anyhow!("events[{index}].date must be an ISO date or timestamp");
    let prefix = value.get(..10).ok_or_else(invalid)?;
    ensure!(
        prefix.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        }),
        "{}",
        invalid()
    );
    let mut date = NaiveDate::parse_from_str(prefix, "%Y-%m-%d").map_err(|_| invalid())?;
    ensure!((1..=9999).contains(&date.year()), "{}", invalid());
    if value.len() == 10 {
        return Ok(date);
    }
    ensure!(
        matches!(value.as_bytes()[10], b'T' | b't' | b' '),
        "{}",
        invalid()
    );
    let time_and_zone = &value[11..];
    let zone_start = time_and_zone
        .find(['Z', 'z', '+', '-'])
        .unwrap_or(time_and_zone.len());
    let (time, zone) = time_and_zone.split_at(zone_start);
    let fraction_start = time.find(['.', ',']).unwrap_or(time.len());
    let (whole, fraction) = time.split_at(fraction_start);
    let mut time = match whole.len() {
        2 => format!("{whole}:00:00"),
        4 if whole.bytes().all(|b| b.is_ascii_digit()) => {
            format!("{}:{}:00", &whole[..2], &whole[2..])
        }
        5 if whole.as_bytes()[2] == b':' => format!("{whole}:00"),
        6 if whole.bytes().all(|b| b.is_ascii_digit()) => {
            format!("{}:{}:{}", &whole[..2], &whole[2..4], &whole[4..])
        }
        8 if whole.as_bytes()[2] == b':' && whole.as_bytes()[5] == b':' => whole.to_owned(),
        _ => return Err(invalid()),
    };
    ensure!(
        time.bytes().enumerate().all(|(i, b)| {
            if i == 2 || i == 5 {
                b == b':'
            } else {
                b.is_ascii_digit()
            }
        }),
        "{}",
        invalid()
    );
    if !fraction.is_empty() {
        ensure!(
            matches!(whole.len(), 6 | 8)
                && fraction.len() > 1
                && fraction[1..].bytes().all(|b| b.is_ascii_digit()),
            "{}",
            invalid()
        );
        time.push('.');
        time.push_str(&fraction[1..fraction.len().min(10)]);
    }
    if time.starts_with("24:") {
        ensure!(
            time[3..].bytes().all(|b| matches!(b, b'0' | b':' | b'.')),
            "{}",
            invalid()
        );
        date = date.succ_opt().ok_or_else(invalid)?;
        time.replace_range(..2, "00");
    }
    let normalized = format!("{date}T{time}{zone}");
    let parsed = if zone.is_empty() {
        NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S%.f")
            .map_err(|_| invalid())?
            .and_utc()
    } else {
        DateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S%.f%#z")
            .map_err(|_| invalid())?
            .with_timezone(&Utc)
    };
    ensure!(
        (1..=9999).contains(&parsed.year()) && parsed.nanosecond() < 1_000_000_000,
        "{}",
        invalid()
    );
    Ok(parsed.date_naive())
}

fn required_string(record: &Mapping, field: &str, index: usize) -> Result<String> {
    let value = record
        .get(Value::from(field))
        .with_context(|| format!("events[{index}] is missing required field: {field}"))?;
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .with_context(|| format!("events[{index}].{field} must be a non-empty string"))
}

/// Load all records before filtering, so invalid out-of-range records still fail.
pub fn load_events(path: impl AsRef<Path>) -> Result<Vec<Event>> {
    let path = path.as_ref();
    let bytes = fs::read(path)
        .with_context(|| format!("cannot read events manifest {}", path.display()))?;
    let document: Value = serde_yaml_ng::from_slice(&bytes)
        .with_context(|| format!("cannot parse events manifest {}", path.display()))?;
    let document = document
        .as_mapping()
        .context("events manifest must contain a YAML mapping")?;
    if let Some(version) = document.get(Value::from("schema_version")) {
        ensure!(
            matches!(version, Value::Number(n) if n.is_u64() && n.as_u64() == Some(1)),
            "events manifest schema_version must be 1"
        );
    }
    let records = document
        .get(Value::from("events"))
        .and_then(Value::as_sequence)
        .context("events manifest must contain an events list")?;
    let mut events = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let record = record
            .as_mapping()
            .with_context(|| format!("events[{index}] must be a YAML mapping"))?;
        let notes = match record.get(Value::from("notes")) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => bail!("events[{index}].notes must be a string when provided"),
        };
        events.push(Event {
            date: parse_date(&required_string(record, "date", index)?, index)?,
            provider: required_string(record, "provider", index)?,
            model: required_string(record, "model", index)?,
            event: EventKind::parse(&required_string(record, "event", index)?, index)?,
            scope: required_string(record, "scope", index)?,
            label: required_string(record, "label", index)?,
            source: required_string(record, "source", index)?,
            notes,
        });
    }
    events.sort_by_key(|event| event.date);
    Ok(events)
}

/// Match Python's SHA-256 palette selection, independent of event order.
pub fn provider_color(provider: &str) -> RGBColor {
    const COLORS: [RGBColor; 10] = [
        RGBColor(31, 119, 180),
        RGBColor(255, 127, 14),
        RGBColor(44, 160, 44),
        RGBColor(214, 39, 40),
        RGBColor(148, 103, 189),
        RGBColor(140, 86, 75),
        RGBColor(227, 119, 194),
        RGBColor(127, 127, 127),
        RGBColor(188, 189, 34),
        RGBColor(23, 190, 207),
    ];
    COLORS[Sha256::digest(provider.as_bytes())[0] as usize % COLORS.len()]
}

#[derive(Debug)]
struct Marker {
    left: i32,
    width: i32,
    lane: i32,
}

#[derive(Debug)]
struct KeyEntry {
    x: i32,
    y: i32,
    lines: Vec<String>,
}

pub(crate) struct EventLayout {
    events: Vec<Event>,
    markers: Vec<Marker>,
    entries: Vec<KeyEntry>,
    top_height: u32,
    key_height: u32,
}

pub(crate) struct SvgTooltip {
    id: String,
    text: String,
    bounds: [(i32, i32); 2],
}

pub(crate) fn save_event_svg(path: &Path, mut svg: String, tooltips: &[SvgTooltip]) -> Result<()> {
    if !tooltips.is_empty() {
        let end = svg
            .rfind("</svg>")
            .context("SVG has no closing root element")?;
        let mut targets = String::new();
        for tooltip in tooltips {
            ensure!(
                tooltip.text.chars().all(|ch| matches!(
                    ch,
                    '\t' | '\n' | '\r'
                        | '\u{20}'..='\u{D7FF}'
                        | '\u{E000}'..='\u{FFFD}'
                        | '\u{10000}'..='\u{10FFFF}'
                )),
                "SVG tooltip {:?} contains an invalid XML character",
                tooltip.id
            );
            let text = tooltip
                .text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let [(x0, y0), (x1, y1)] = tooltip.bounds;
            let id = &tooltip.id;
            targets.push_str(&format!(
                "<g id=\"{id}\" aria-labelledby=\"tooltip-{id}\" cursor=\"help\">\
                 <title id=\"tooltip-{id}\">{text}</title>\
                 <rect x=\"{x0}\" y=\"{y0}\" width=\"{}\" height=\"{}\" \
                 fill=\"none\" pointer-events=\"all\"/></g>\n",
                x1 - x0,
                y1 - y0,
            ));
        }
        svg.insert_str(end, &targets);
    }
    fs::write(path, svg).with_context(|| format!("writing SVG plot {}", path.display()))
}

fn text_width(font: &FontDesc<'_>, text: &str) -> Result<i32> {
    font.box_size(text)
        .map(|(w, _)| w as i32)
        .map_err(|err| anyhow!("measure event text: {err}"))
}

fn wrap_text(font: &FontDesc<'_>, text: &str, width: i32) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if text_width(font, &candidate)? <= width {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        for ch in word.chars() {
            let candidate = format!("{current}{ch}");
            if !current.is_empty() && text_width(font, &candidate)? > width {
                lines.push(std::mem::take(&mut current));
            }
            current.push(ch);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    Ok(lines)
}

impl EventLayout {
    pub(crate) fn load(
        path: Option<&Path>,
        t_min: DateTime<Utc>,
        t_max: DateTime<Utc>,
    ) -> Result<Self> {
        let mut events = match path {
            Some(path) => load_events(path)?,
            None => Vec::new(),
        };
        events.retain(|event| (t_min..=t_max).contains(&event.timestamp()));
        let mut layout = Self {
            events,
            markers: Vec::new(),
            entries: Vec::new(),
            top_height: 0,
            key_height: 0,
        };
        if layout.events.is_empty() {
            return Ok(layout);
        }

        let font = ("monospace", 16).into_font();
        let left = (MARGIN + Y_LABEL_AREA) as i32;
        let right = (PLOT_WIDTH - MARGIN - 1) as i32;
        let span = (t_max - t_min).num_seconds().max(1) as f64;
        let mut lane_ends = Vec::new();
        for (index, event) in layout.events.iter().enumerate() {
            let x = left
                + (((event.timestamp() - t_min).num_seconds() as f64 / span)
                    * (right - left) as f64)
                    .round() as i32;
            let width = text_width(&font, &(index + 1).to_string())? + 16;
            let label_left = (x - width / 2).clamp(left, right - width);
            let lane = lane_ends
                .iter()
                .position(|end| end + 8 <= label_left)
                .unwrap_or(lane_ends.len());
            if lane == lane_ends.len() {
                lane_ends.push(0);
            }
            lane_ends[lane] = label_left + width;
            layout.markers.push(Marker {
                left: label_left,
                width,
                lane: lane as i32,
            });
        }
        layout.top_height = (lane_ends.len() as u32 + 2) * 26;

        let column_width = (right - left) / 3;
        let mut y = 48;
        for (row_index, row) in layout.events.chunks(3).enumerate() {
            let mut row_height = 0;
            for (column, event) in row.iter().enumerate() {
                let index = row_index * 3 + column + 1;
                let mut lines = Vec::new();
                for part in [
                    format!(
                        "{index}. {} | {} | {}",
                        event.date,
                        event.provider,
                        event.event.as_str()
                    ),
                    event.label.clone(),
                    format!("Scope: {}", event.scope),
                ] {
                    lines.extend(wrap_text(&font, &part, column_width - 48)?);
                }
                row_height = row_height.max(lines.len() as i32 * 26 + 8);
                layout.entries.push(KeyEntry {
                    x: left + column as i32 * column_width,
                    y,
                    lines,
                });
            }
            y += row_height;
        }
        layout.key_height = y as u32 + 24;
        Ok(layout)
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (PLOT_WIDTH, PLOT_HEIGHT + self.top_height + self.key_height)
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.events.is_empty()
    }

    pub(crate) fn plot_area<DB: DrawingBackend>(
        &self,
        root: &DrawingArea<DB, Shift>,
    ) -> DrawingArea<DB, Shift> {
        if self.events.is_empty() {
            return root.clone();
        }
        root.clone()
            .shrink((0, self.top_height), (PLOT_WIDTH, PLOT_HEIGHT))
    }

    pub(crate) fn draw<DB, CT>(
        &self,
        root: &DrawingArea<DB, Shift>,
        plot: &DrawingArea<DB, CT>,
        y_max: f64,
    ) -> Result<Vec<SvgTooltip>>
    where
        DB: DrawingBackend,
        DB::ErrorType: 'static,
        CT: CoordTranslate<From = (DateTime<Utc>, f64)>,
    {
        if self.events.is_empty() {
            return Ok(Vec::new());
        }
        let mut tooltips = Vec::with_capacity(self.events.len() * 3);
        let font = ("monospace", 16).into_font();
        let label_style = font.color(&BLACK).pos(Pos::new(HPos::Center, VPos::Bottom));
        let key_style = font.color(&BLACK).pos(Pos::new(HPos::Left, VPos::Top));
        let heading_style = ("sans-serif", 18)
            .into_font()
            .color(&BLACK)
            .pos(Pos::new(HPos::Left, VPos::Top));
        let key_top = (self.top_height + PLOT_HEIGHT) as i32;
        root.draw_text(
            &format!(
                "External events ({}) - dates do not show model use or impact",
                self.events.len()
            ),
            &heading_style,
            ((MARGIN + Y_LABEL_AREA) as i32, 16),
        )
        .map_err(|err| anyhow!("draw event notice: {err}"))?;
        root.draw_text(
            "Event key",
            &heading_style,
            ((MARGIN + Y_LABEL_AREA) as i32, key_top + 12),
        )
        .map_err(|err| anyhow!("draw event key heading: {err}"))?;

        let (x_range, _) = plot.get_pixel_range();
        for (index, (event, marker)) in self.events.iter().zip(&self.markers).enumerate() {
            let color = provider_color(&event.provider);
            let top = plot.map_coordinate(&(event.timestamp(), y_max));
            let bottom = plot.map_coordinate(&(event.timestamp(), 0.0));
            draw_event_line(root, top, bottom, event.event, color)?;
            tooltips.push(SvgTooltip {
                id: format!("event-line-{}", index + 1),
                text: event.tooltip_text(),
                bounds: [
                    ((top.0 - 4).max(x_range.start), top.1),
                    ((top.0 + 4).min(x_range.end), bottom.1),
                ],
            });
            let center = marker.left + marker.width / 2;
            let y = top.1 - 10 - marker.lane * 26;
            root.draw(&PathElement::new(
                vec![top, (center, y + 4)],
                color.stroke_width(1),
            ))
            .map_err(|err| anyhow!("draw event connector: {err}"))?;
        }
        for (index, ((event, marker), entry)) in self
            .events
            .iter()
            .zip(&self.markers)
            .zip(&self.entries)
            .enumerate()
        {
            let color = provider_color(&event.provider);
            let top = plot.map_coordinate(&(event.timestamp(), y_max));
            let y = top.1 - 10 - marker.lane * 26;
            let rect = [(marker.left, y - 20), (marker.left + marker.width, y + 3)];
            tooltips.push(SvgTooltip {
                id: format!("event-label-{}", index + 1),
                text: event.tooltip_text(),
                bounds: rect,
            });
            root.draw(&Rectangle::new(rect, WHITE.filled()))
                .map_err(|err| anyhow!("draw event label background: {err}"))?;
            root.draw(&Rectangle::new(rect, color.stroke_width(1)))
                .map_err(|err| anyhow!("draw event label border: {err}"))?;
            root.draw_text(
                &(index + 1).to_string(),
                &label_style,
                (marker.left + marker.width / 2, y),
            )
            .map_err(|err| anyhow!("draw event label: {err}"))?;
            draw_event_line(
                root,
                (entry.x + 4, key_top + entry.y + 8),
                (entry.x + 28, key_top + entry.y + 8),
                event.event,
                color,
            )?;
            for (line, text) in entry.lines.iter().enumerate() {
                root.draw_text(
                    text,
                    &key_style,
                    (entry.x + 36, key_top + entry.y + line as i32 * 26),
                )
                .map_err(|err| anyhow!("draw event key: {err}"))?;
            }
            let key_width = entry.lines.iter().try_fold(0, |width: i32, line| {
                text_width(&font, line).map(|line_width| width.max(line_width))
            })?;
            tooltips.push(SvgTooltip {
                id: format!("event-key-{}", index + 1),
                text: event.tooltip_text(),
                bounds: [
                    (entry.x, key_top + entry.y),
                    (
                        entry.x + 36 + key_width + 4,
                        key_top + entry.y + entry.lines.len() as i32 * 26,
                    ),
                ],
            });
        }
        Ok(tooltips)
    }
}

fn draw_event_line<DB: DrawingBackend>(
    root: &DrawingArea<DB, Shift>,
    from: (i32, i32),
    to: (i32, i32),
    kind: EventKind,
    color: RGBColor,
) -> Result<()>
where
    DB::ErrorType: 'static,
{
    let length = (to.0 - from.0).abs() + (to.1 - from.1).abs();
    let pattern = kind.pattern();
    let mut offset = 0;
    let mut step = 0;
    while offset < length {
        let end = if pattern.is_empty() {
            length
        } else {
            (offset + pattern[step % pattern.len()]).min(length)
        };
        if step % 2 == 0 {
            let point = |distance| {
                (
                    from.0 + (to.0 - from.0).signum() * distance,
                    from.1 + (to.1 - from.1).signum() * distance,
                )
            };
            root.draw(&PathElement::new(
                vec![point(offset), point(end)],
                color.mix(0.8).stroke_width(2),
            ))
            .map_err(|err| anyhow!("draw event line: {err}"))?;
        }
        offset = end;
        step += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn dense_marker_and_key_layouts_do_not_overlap() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.yaml");
        let mut manifest = String::from("schema_version: 1\nevents:\n");
        for i in 0..100 {
            manifest.push_str(&format!(
                "  - date: '2024-01-{:02}'\n    provider: Example\n    model: Model\n    event: pilot\n    scope: Restricted group\n    label: Event {i} {}\n    source: test\n",
                1 + i % 10,
                "x".repeat(i % 75)
            ));
        }
        fs::write(&path, manifest).unwrap();
        let start = NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let end = start + chrono::Duration::days(9);
        let layout = EventLayout::load(Some(&path), start, end).unwrap();
        assert_eq!(layout.events.len(), 100);
        assert_eq!(layout.markers.len(), 100);
        assert_eq!(layout.entries.len(), 100);
        for (i, a) in layout.markers.iter().enumerate() {
            assert!(a.left >= (MARGIN + Y_LABEL_AREA) as i32);
            assert!(a.left + a.width < (PLOT_WIDTH - MARGIN) as i32);
            assert!((a.lane as u32 + 1) * 26 < layout.top_height);
            for b in &layout.markers[i + 1..] {
                if a.lane == b.lane {
                    assert!(a.left + a.width + 8 <= b.left || b.left + b.width + 8 <= a.left);
                }
            }
        }
        let font = ("monospace", 16).into_font();
        let column_width = ((PLOT_WIDTH - MARGIN * 2 - Y_LABEL_AREA - 1) / 3) as i32;
        for (i, a) in layout.entries.iter().enumerate() {
            let bottom = a.y + a.lines.len() as i32 * 26;
            assert!(bottom < layout.key_height as i32);
            for line in &a.lines {
                assert!(text_width(&font, line).unwrap() <= column_width - 48);
            }
            for b in &layout.entries[i + 1..] {
                if a.x == b.x {
                    assert!(bottom + 8 <= b.y);
                }
            }
        }
    }

    #[test]
    fn each_event_kind_has_a_distinct_line_pattern() {
        let kinds = [
            EventKind::Announced,
            EventKind::Available,
            EventKind::Pilot,
            EventKind::Default,
            EventKind::Retired,
        ];
        for (i, kind) in kinds.iter().enumerate() {
            for other in &kinds[i + 1..] {
                assert_ne!(kind.pattern(), other.pattern());
            }
        }
    }

    #[test]
    fn no_events_preserves_the_full_plot_area() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("plot.png");
        let start = NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let layout = EventLayout::load(None, start, start + chrono::Duration::days(1)).unwrap();
        let backend = BitMapBackend::new(&output, layout.dimensions());
        let root = backend.into_drawing_area();
        let plot = layout.plot_area(&root);
        let (x_range, y_range) = plot.get_pixel_range();

        assert_eq!(x_range.start, 0);
        assert_eq!(x_range.end, PLOT_WIDTH as i32);
        assert_eq!(y_range.start, 0);
        assert_eq!(y_range.end, PLOT_HEIGHT as i32);
    }
}
