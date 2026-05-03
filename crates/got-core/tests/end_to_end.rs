//! End-to-end integration test: build a tiny git repo from scratch and
//! exercise `got_core::analyze`. This protects the public API and the JSON
//! output schema against accidental regressions.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use got_core::{analyze, AnalyzeOptions};
use tempfile::tempdir;

fn run_with_date(repo: &Path, date: &str, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Alice")
        .env("GIT_AUTHOR_EMAIL", "alice@example.com")
        .env("GIT_COMMITTER_NAME", "Alice")
        .env("GIT_COMMITTER_EMAIL", "alice@example.com")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git available");
    assert!(status.success(), "git {args:?} failed");
}

fn run(repo: &Path, args: &[&str]) {
    run_with_date(repo, "2024-01-01T00:00:00Z", args);
}

#[test]
fn analyze_tiny_repo_end_to_end() {
    let dir = tempdir().unwrap();
    let repo = dir.path();
    run(repo, &["init", "-q", "-b", "main"]);
    run(repo, &["config", "commit.gpgsign", "false"]);

    fs::write(repo.join("a.py"), "print('hi')\nprint('there')\n").unwrap();
    run(repo, &["add", "a.py"]);
    run_with_date(repo, "2020-01-15T00:00:00Z", &["commit", "-q", "-m", "first"]);

    fs::write(
        repo.join("b.py"),
        "def f():\n    return 1\n\ndef g():\n    return 2\n",
    )
    .unwrap();
    run(repo, &["add", "b.py"]);
    run_with_date(repo, "2021-06-15T00:00:00Z", &["commit", "-q", "-m", "second"]);

    let outdir = dir.path().join("out");
    let result = analyze(&AnalyzeOptions {
        repo_dir: repo.to_path_buf(),
        branch: "main".into(),
        outdir: outdir.clone(),
        quiet: true,
        procs: 1,
        ..Default::default()
    })
    .unwrap();

    // Two sampled commits (one-week interval), so two timepoints.
    assert_eq!(result.timestamps.len(), 2);
    // Both .py files contribute, so the .py extension curve should grow.
    let py_curve = result.exts.get(".py").expect("ext .py present");
    assert_eq!(py_curve.len(), 2);
    assert!(py_curve[0] > 0);
    assert!(py_curve[1] >= py_curve[0]);

    // Cohorts: one per year (2020, 2021).
    assert!(result.cohorts.contains_key("2020"));
    assert!(result.cohorts.contains_key("2021"));

    // Domains: only example.com.
    let domain_keys: Vec<_> = result.domains.keys().cloned().collect();
    assert_eq!(domain_keys, vec!["example.com".to_string()]);

    // JSON files exist on disk and parse.
    for name in [
        "cohorts.json",
        "exts.json",
        "authors.json",
        "dirs.json",
        "domains.json",
        "survival.json",
    ] {
        let bytes =
            fs::read(outdir.join(name)).unwrap_or_else(|e| panic!("reading {name}: {e}"));
        let _: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    }

    // Curve files have the expected shape: {y, ts, labels}.
    let cohorts: serde_json::Value =
        serde_json::from_slice(&fs::read(outdir.join("cohorts.json")).unwrap()).unwrap();
    assert!(cohorts.get("y").is_some());
    assert!(cohorts.get("ts").is_some());
    assert!(cohorts.get("labels").is_some());
    assert_eq!(
        cohorts["labels"].as_array().unwrap().len(),
        cohorts["y"].as_array().unwrap().len()
    );
    // Timestamps have no timezone suffix, matching Python's
    // datetime.utcfromtimestamp(...).isoformat().
    let ts0 = cohorts["ts"][0].as_str().unwrap();
    assert!(!ts0.ends_with('Z') && !ts0.contains('+'));

    // Survival is a {sha: [[ts, count], ...]} mapping; either commit may
    // not contribute depending on blame attribution, but the file must be
    // a JSON object that deserialises into the expected shape.
    let survival: BTreeMap<String, Vec<(i64, u64)>> =
        serde_json::from_slice(&fs::read(outdir.join("survival.json")).unwrap()).unwrap();
    let _ = survival;
}
