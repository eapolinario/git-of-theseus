use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::tempdir;

const LINE: &str = env!("CARGO_BIN_EXE_git-of-theseus-line-plot");
const STACK: &str = env!("CARGO_BIN_EXE_git-of-theseus-stack-plot");
const SURVIVAL: &str = env!("CARGO_BIN_EXE_git-of-theseus-survival-plot");
const ANALYZE: &str = env!("CARGO_BIN_EXE_git-of-theseus-analyze");

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

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Alice")
        .env("GIT_AUTHOR_EMAIL", "alice@example.com")
        .env("GIT_COMMITTER_NAME", "Alice")
        .env("GIT_COMMITTER_EMAIL", "alice@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn make_repository() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let repo = dir.path();
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("a.rs"), "fn main() {}\n").unwrap();
    run_git(repo, &["add", "a.rs"]);
    run_git(repo, &["commit", "-q", "-m", "initial"]);
    dir
}

#[test]
fn missing_branch_warns_with_attached_or_detached_head_fallback() {
    let dir = make_repository();
    let repo = dir.path();
    let run_analyze = || {
        Command::new(ANALYZE)
            .args(["--branch", "missing", "--quiet", "--procs", "1", "--outdir"])
            .arg(repo.join("out"))
            .arg(repo)
            .output()
            .unwrap()
    };

    let attached = run_analyze();
    assert!(attached.status.success(), "{attached:?}");
    assert!(String::from_utf8_lossy(&attached.stderr).contains(
        "Requested branch: 'missing' does not exist. Falling back to default branch 'main'"
    ));

    let head = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    run_git(repo, &["checkout", "-q", "--detach"]);

    let detached = run_analyze();
    assert!(detached.status.success(), "{detached:?}");
    assert!(String::from_utf8_lossy(&detached.stderr).contains(&format!(
        "Requested branch: 'missing' does not exist. Falling back to HEAD commit '{}'",
        head.trim()
    )));
}
