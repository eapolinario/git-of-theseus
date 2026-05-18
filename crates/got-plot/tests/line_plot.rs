//! Smoke tests for `line_plot`.

use std::fs;

use got_plot::{line_plot, LinePlotOptions};
use tempfile::tempdir;

fn write_fixture(dir: &std::path::Path) -> std::path::PathBuf {
    // A small two-series, three-timestamp curve.
    let json = r#"{
        "y": [[1, 2, 3], [3, 2, 1]],
        "ts": ["2020-01-01T00:00:00", "2020-06-01T00:00:00", "2021-01-01T00:00:00"],
        "labels": ["alpha", "beta"]
    }"#;
    let path = dir.join("curve.json");
    fs::write(&path, json).unwrap();
    path
}

#[test]
fn writes_nonempty_png() {
    let dir = tempdir().unwrap();
    let input = write_fixture(dir.path());
    let output = dir.path().join("out.png");
    let opts = LinePlotOptions {
        input,
        output: output.clone(),
        max_n: 20,
        normalize: false,
    };
    let written = line_plot(&opts).unwrap();
    assert_eq!(written, output);
    let meta = fs::metadata(&output).unwrap();
    // PNG header alone is 8 bytes; a real image will be much larger.
    assert!(meta.len() > 1000, "output too small: {} bytes", meta.len());
    let bytes = fs::read(&output).unwrap();
    assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
}

#[test]
fn writes_svg() {
    let dir = tempdir().unwrap();
    let input = write_fixture(dir.path());
    let output = dir.path().join("out.svg");
    let opts = LinePlotOptions {
        input,
        output: output.clone(),
        max_n: 20,
        normalize: true,
    };
    line_plot(&opts).unwrap();
    let bytes = fs::read(&output).unwrap();
    let head = std::str::from_utf8(&bytes[..bytes.len().min(200)]).unwrap_or("");
    assert!(head.contains("<svg"), "not an SVG: {head:?}");
}
