use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::tempdir;

const LINE: &str = env!("CARGO_BIN_EXE_git-of-theseus-line-plot");
const STACK: &str = env!("CARGO_BIN_EXE_git-of-theseus-stack-plot");
const SURVIVAL: &str = env!("CARGO_BIN_EXE_git-of-theseus-survival-plot");

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn calendar_clis_render_event_manifests() {
    let dir = tempdir().unwrap();
    for binary in [LINE, STACK] {
        let output = dir.path().join("events.svg");
        let result = Command::new(binary)
            .arg(fixture("curve.json"))
            .arg("--events")
            .arg(fixture("events.yaml"))
            .args(["--normalize", "--outfile"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let svg = fs::read_to_string(&output).unwrap();
        assert!(svg.contains("External events (5)"));
        assert!(svg.contains("First-day announcement"));
        assert!(svg.contains("Last-day retirement"));
        assert!(!svg.contains("Excluded"));
    }
}

#[test]
fn invalid_events_fail_without_replacing_output() {
    let dir = tempdir().unwrap();
    let manifest = dir.path().join("bad.yaml");
    fs::write(&manifest, "events: [").unwrap();
    for binary in [LINE, STACK, SURVIVAL] {
        let output = dir.path().join("existing.svg");
        fs::write(&output, "existing output").unwrap();
        let result = Command::new(binary)
            .arg(fixture("curve.json"))
            .arg("--events")
            .arg(&manifest)
            .arg("--outfile")
            .arg(&output)
            .output()
            .unwrap();
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        if binary == SURVIVAL {
            assert_eq!(result.status.code(), Some(2));
            assert!(error.contains("elapsed age"), "{error}");
        } else {
            assert!(error.contains("cannot parse events manifest"), "{error}");
        }
        assert_eq!(fs::read_to_string(&output).unwrap(), "existing output");
    }
}

#[test]
fn all_plot_help_lists_events() {
    for binary in [LINE, STACK, SURVIVAL] {
        let result = Command::new(binary).arg("--help").output().unwrap();
        assert!(result.status.success());
        assert!(String::from_utf8_lossy(&result.stdout).contains("--events"));
    }
}
