use std::fs;
use std::path::PathBuf;

use chrono::NaiveDate;
use got_plot::events::{load_events, provider_color, EventKind};
use got_plot::{line_plot, stack_plot, LinePlotOptions, StackPlotOptions};
use plotters::style::RGBColor;
use tempfile::tempdir;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn record(date: &str) -> String {
    format!(
        "events:\n  - date: {date}\n    provider: Example\n    model: Model\n    event: available\n    scope: public\n    label: A model release\n    source: release notes\n"
    )
}

fn check_tooltips(svg: &str) {
    let document = roxmltree::Document::parse(svg).unwrap();
    assert_eq!(
        document
            .descendants()
            .filter(|n| n.has_tag_name("title"))
            .count(),
        15
    );
    for number in 1..=5 {
        let mut titles = Vec::new();
        for part in ["line", "label", "key"] {
            let id = format!("event-{part}-{number}");
            let group = document
                .descendants()
                .find(|node| node.attribute("id") == Some(id.as_str()))
                .unwrap();
            let title = group.first_element_child().unwrap();
            assert!(title.has_tag_name("title"));
            assert_eq!(group.attribute("aria-labelledby"), title.attribute("id"));
            assert_eq!(group.attribute("cursor"), Some("help"));
            let target = title.next_sibling_element().unwrap();
            assert!(target.has_tag_name("rect"));
            assert_eq!(target.attribute("pointer-events"), Some("all"));
            for dimension in ["width", "height"] {
                assert!(target.attribute(dimension).unwrap().parse::<i32>().unwrap() > 0);
            }
            titles.push(title.text().unwrap());
        }
        assert_eq!(titles[0], titles[1]);
        assert_eq!(titles[1], titles[2]);
        assert!(!titles[0].contains("Excluded"));
        if number == 2 {
            assert_eq!(
                titles[0],
                "Available <One> & \"Two\"\nDate: 2024-01-05\nProvider: Example A\n\
                 Model: Model One\nEvent: available\nScope: Public test preview\n\
                 Source: release notes\nNotes: The UTC date is January 5."
            );
        } else {
            assert!(!titles[0].contains("Notes:"));
        }
    }
}

#[test]
fn shared_manifest_dates_order_and_metadata() {
    let events = load_events(fixture("events.yaml")).unwrap();
    let dates: Vec<_> = events.iter().map(|event| event.date.to_string()).collect();
    assert_eq!(
        dates,
        [
            "2023-12-01",
            "2024-01-01",
            "2024-01-05",
            "2024-01-05",
            "2024-01-05",
            "2024-01-10",
            "2024-02-01",
        ]
    );
    assert_eq!(events[2].event, EventKind::Available);
    assert_eq!(events[3].event, EventKind::Pilot);
    assert_eq!(events[4].event, EventKind::Default);
    assert_eq!(
        events[2].notes.as_deref(),
        Some("The UTC date is January 5.")
    );
    assert_eq!(events[2].label, "Available <One> & \"Two\"");
}

#[test]
fn accepts_dates_and_normalizes_timestamps_to_utc() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("events.yaml");
    for (value, expected) in [
        ("2024-01-05", "2024-01-05"),
        ("'2024-01-05'", "2024-01-05"),
        ("2024-01-05T12:34:56", "2024-01-05"),
        ("2024-01-05T00:30:00+02:00", "2024-01-04"),
        ("2024-01-05T23:30:00-02:00", "2024-01-06"),
        ("2024-01-05T23:30:00.123456Z", "2024-01-05"),
        ("'2024-01-05 12:00:00Z'", "2024-01-05"),
        ("2024-01-05T12", "2024-01-05"),
        ("2024-01-05T1230", "2024-01-05"),
        ("2024-01-05T123000", "2024-01-05"),
        ("2024-01-05T12:30:00+02", "2024-01-05"),
        ("2024-01-05T24:00:00", "2024-01-06"),
        ("2024-01-05T23:30:00,123456-0200", "2024-01-06"),
    ] {
        fs::write(&path, record(value)).unwrap();
        let events = load_events(&path).unwrap();
        assert_eq!(events[0].date.to_string(), expected, "{value}");
    }
}

#[test]
fn rejects_partial_or_invalid_dates() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("events.yaml");
    for value in [
        "null",
        "''",
        "true",
        "123",
        "'2024'",
        "'2024-01'",
        "'2024-001'",
        "'20240105'",
        "'2024-02-30'",
        "'2024-01-05not-a-time'",
        "'0000-01-01'",
        "'0001-01-01T00:00:00+02:00'",
        "'9999-12-31T23:00:00-02:00'",
        "'2024-01-05T23:59:60Z'",
        "'2024-01-05T24:00:01'",
        "'2024-01-05T12:30.5'",
        "'2024-01-05T24:00:00.1'",
    ] {
        fs::write(&path, record(value)).unwrap();
        let error = load_events(&path).unwrap_err().to_string();
        assert!(error.contains("events[0].date"), "{value}: {error}");
    }
}

#[test]
fn rejects_invalid_schema_versions() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("events.yaml");
    for version in ["true", "1.0", "'1'", "null", "2"] {
        fs::write(&path, format!("schema_version: {version}\nevents: []")).unwrap();
        let error = load_events(&path).unwrap_err().to_string();
        assert!(error.contains("schema_version must be 1"), "{error}");
    }
    for input in ["events: []", "schema_version: 1\nevents: []"] {
        fs::write(&path, input).unwrap();
        assert!(load_events(&path).unwrap().is_empty());
    }
}

#[test]
fn rejects_invalid_manifest_shapes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("events.yaml");
    for (input, expected) in [
        ("[]".into(), "YAML mapping"),
        ("".into(), "YAML mapping"),
        ("{}".into(), "events list"),
        ("events: {}".into(), "events list"),
        ("events: [bad]".into(), "events[0] must be a YAML mapping"),
        ("events: [{}]".into(), "missing required field"),
        (
            record("2024-01-05").replace("provider: Example", "provider: ' '"),
            "provider must be a non-empty string",
        ),
        (
            record("2024-01-05").replace("model: Model", "model: 3"),
            "model must be a non-empty string",
        ),
        (
            record("2024-01-05").replace("event: available", "event: unknown"),
            "event must be one of",
        ),
        (
            format!("{}    notes: 3\n", record("2024-01-05")),
            "notes must be a string",
        ),
    ] {
        fs::write(&path, input).unwrap();
        let error = load_events(&path).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn file_errors_include_manifest_path() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("missing.yaml");
    let error = load_events(&path).unwrap_err().to_string();
    assert!(error.contains("cannot read events manifest"), "{error}");
    assert!(error.contains("missing.yaml"), "{error}");
    fs::write(&path, "events: [").unwrap();
    let error = load_events(&path).unwrap_err().to_string();
    assert!(error.contains("cannot parse events manifest"), "{error}");
}

#[test]
fn provider_colors_match_python() {
    assert_eq!(provider_color("anthropic"), RGBColor(23, 190, 207));
    assert_eq!(provider_color("openai"), RGBColor(140, 86, 75));
    assert_eq!(provider_color("deepseek"), RGBColor(255, 127, 14));
    assert_eq!(provider_color("kimi"), RGBColor(188, 189, 34));
}

#[test]
fn renders_line_and_stack_png_and_svg_with_all_event_types() {
    let dir = tempdir().unwrap();
    for stacked in [false, true] {
        for normalize in [false, true] {
            for extension in ["png", "svg"] {
                let output = dir
                    .path()
                    .join(format!("plot-{stacked}-{normalize}.{extension}"));
                if stacked {
                    stack_plot(&StackPlotOptions {
                        input: fixture("curve.json"),
                        output: output.clone(),
                        normalize,
                        max_n: 1,
                        events: Some(fixture("events.yaml")),
                    })
                    .unwrap();
                } else {
                    line_plot(&LinePlotOptions {
                        input: fixture("curve.json"),
                        output: output.clone(),
                        normalize,
                        max_n: 1,
                        events: Some(fixture("events.yaml")),
                    })
                    .unwrap();
                }
                let bytes = fs::read(output).unwrap();
                assert!(bytes.len() > 1000);
                if extension == "png" {
                    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
                } else {
                    let svg = String::from_utf8(bytes).unwrap();
                    for label in [
                        "First-day announcement",
                        "Last-day retirement",
                        "Same-day pilot",
                        "Same-day default",
                        "available",
                        "Public test preview",
                        "dates do not show model use or impact",
                    ] {
                        assert!(svg.contains(label), "missing {label}");
                    }
                    assert!(svg.contains("Available &lt;One&gt; &amp; &quot;Two&quot;"));
                    assert!(!svg.contains("Excluded"));
                    assert!(svg.contains("External events (5)"));
                    check_tooltips(&svg);
                }
            }
        }
    }
}

#[test]
fn no_visible_events_preserve_svg_exactly() {
    let dir = tempdir().unwrap();
    let manifest = dir.path().join("events.yaml");
    let output = dir.path().join("plot.svg");
    for stacked in [false, true] {
        let render = |events| {
            if stacked {
                stack_plot(&StackPlotOptions {
                    input: fixture("curve.json"),
                    output: output.clone(),
                    events,
                    ..Default::default()
                })
            } else {
                line_plot(&LinePlotOptions {
                    input: fixture("curve.json"),
                    output: output.clone(),
                    events,
                    ..Default::default()
                })
            }
        };
        render(None).unwrap();
        let before = fs::read(&output).unwrap();
        assert!(!String::from_utf8_lossy(&before).contains("<title"));
        for text in ["events: []".to_owned(), record("2025-01-01")] {
            fs::write(&manifest, text).unwrap();
            render(Some(manifest.clone())).unwrap();
            assert_eq!(fs::read(&output).unwrap(), before);
        }
    }
}

#[test]
fn invalid_manifest_does_not_replace_output() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("plot.svg");
    fs::write(&output, "existing output").unwrap();
    let opts = LinePlotOptions {
        input: fixture("curve.json"),
        output: output.clone(),
        events: Some(dir.path().join("missing.yaml")),
        ..Default::default()
    };
    assert!(line_plot(&opts).is_err());
    assert_eq!(fs::read_to_string(output).unwrap(), "existing output");
}

#[test]
fn retains_same_day_records_in_input_order() {
    let events = load_events(fixture("events.yaml")).unwrap();
    let same_day: Vec<_> = events
        .iter()
        .filter(|event| event.date == NaiveDate::from_ymd_opt(2024, 1, 5).unwrap())
        .map(|event| event.event)
        .collect();
    assert_eq!(
        same_day,
        [EventKind::Available, EventKind::Pilot, EventKind::Default]
    );
}

#[test]
fn tooltip_metadata_is_plain_xml_text() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("events.yaml");
    let output = dir.path().join("out.SVG");
    let source = "https://example.invalid/?a=1&b=\"<tag>\"";
    let notes = "<script>not executable</script>\nKeep \"quotes\" & Unicode: \u{e9}.";
    let manifest = record("2024-01-05").replace(
        "source: release notes",
        &format!(
            "source: {}\n    notes: {}",
            serde_json::to_string(source).unwrap(),
            serde_json::to_string(notes).unwrap(),
        ),
    );
    fs::write(&input, manifest).unwrap();
    line_plot(&LinePlotOptions {
        input: fixture("curve.json"),
        output: output.clone(),
        events: Some(input),
        ..Default::default()
    })
    .unwrap();
    let svg = fs::read_to_string(output).unwrap();
    let document = roxmltree::Document::parse(&svg).unwrap();
    assert!(!document
        .descendants()
        .any(|node| node.has_tag_name("script")));
    for title in document
        .descendants()
        .filter(|node| node.has_tag_name("title"))
    {
        let text = title.text().unwrap();
        assert!(text.contains(&format!("Source: {source}")));
        assert!(text.contains(&format!("Notes: {notes}")));
    }
}

#[test]
fn invalid_tooltip_text_does_not_replace_svg() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("events.yaml");
    let output = dir.path().join("out.svg");
    fs::write(
        &input,
        format!("{}    notes: \"invalid \\0 text\"\n", record("2024-01-05")),
    )
    .unwrap();
    fs::write(&output, "existing output").unwrap();
    let error = line_plot(&LinePlotOptions {
        input: fixture("curve.json"),
        output: output.clone(),
        events: Some(input),
        ..Default::default()
    })
    .unwrap_err();
    assert!(error.to_string().contains("invalid XML character"));
    assert_eq!(fs::read_to_string(output).unwrap(), "existing output");
}
