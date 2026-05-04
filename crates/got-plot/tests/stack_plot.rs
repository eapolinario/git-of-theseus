//! Smoke tests for `stack_plot`.

use std::fs;

use got_plot::{stack_plot, StackPlotOptions};
use tempfile::tempdir;

fn write_fixture(dir: &std::path::Path, n_series: usize) -> std::path::PathBuf {
    // n_series labeled series, three timestamps; values ramp so the
    // "max per series" ordering is well-defined for the top_n test.
    let ts = r#"["2020-01-01T00:00:00", "2020-06-01T00:00:00", "2021-01-01T00:00:00"]"#;
    let mut y_rows = Vec::new();
    let mut labels = Vec::new();
    for i in 0..n_series {
        let v = (i + 1) * 10;
        y_rows.push(format!("[{v}, {v}, {v}]"));
        labels.push(format!("\"label-{i:02}\""));
    }
    let json = format!(
        r#"{{"y": [{}], "ts": {ts}, "labels": [{}]}}"#,
        y_rows.join(","),
        labels.join(",")
    );
    let path = dir.join("curve.json");
    fs::write(&path, json).unwrap();
    path
}

#[test]
fn writes_nonempty_png() {
    let dir = tempdir().unwrap();
    let input = write_fixture(dir.path(), 3);
    let output = dir.path().join("stack.png");
    let opts = StackPlotOptions {
        input,
        output: output.clone(),
        max_n: 20,
        normalize: false,
    };
    stack_plot(&opts).unwrap();
    let bytes = fs::read(&output).unwrap();
    assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
    assert!(bytes.len() > 1000);
}

#[test]
fn aggregates_other_when_above_max_n() {
    // 5 series, max_n=2 -> top 2 by max + "other" band = 3 bands total.
    // We can't introspect the rendered PNG cheaply, but we can at least
    // confirm rendering succeeds (the band loop touches `cum[i-1]`, so a
    // miscount would panic).
    let dir = tempdir().unwrap();
    let input = write_fixture(dir.path(), 5);
    let output = dir.path().join("stack.svg");
    let opts = StackPlotOptions {
        input,
        output: output.clone(),
        max_n: 2,
        normalize: true,
    };
    stack_plot(&opts).unwrap();
    let s = fs::read_to_string(&output).unwrap();
    assert!(s.contains("<svg"));
    // Three filled polygons + at least one legend entry per band.
    assert!(
        s.matches("<polygon").count() >= 3,
        "expected >=3 polygons, got {}: {}",
        s.matches("<polygon").count(),
        &s[..200.min(s.len())]
    );
}
