//! End-to-end integration test: build a tiny git repo from scratch and
//! exercise `got_core::analyze`. This protects the public API and the JSON
//! output schema against accidental regressions.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use got_core::{analyze, analyze_many, AnalyzeOptions};
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
    run_with_date(
        repo,
        "2020-01-15T00:00:00Z",
        &["commit", "-q", "-m", "first"],
    );

    fs::write(
        repo.join("b.py"),
        "def f():\n    return 1\n\ndef g():\n    return 2\n",
    )
    .unwrap();
    run(repo, &["add", "b.py"]);
    run_with_date(
        repo,
        "2021-06-15T00:00:00Z",
        &["commit", "-q", "-m", "second"],
    );

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
        let bytes = fs::read(outdir.join(name)).unwrap_or_else(|e| panic!("reading {name}: {e}"));
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

/// Creates a tiny one-commit repo named `name` under `parent`, and returns
/// its path. Used by the multi-repo test below.
fn make_tiny_repo(parent: &Path, name: &str) -> std::path::PathBuf {
    let repo = parent.join(name);
    fs::create_dir_all(&repo).unwrap();
    run(&repo, &["init", "-q", "-b", "main"]);
    run(&repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("a.py"), "print('hi')\n").unwrap();
    run(&repo, &["add", "a.py"]);
    run_with_date(
        &repo,
        "2020-01-15T00:00:00Z",
        &["commit", "-q", "-m", "first"],
    );
    repo
}

#[test]
fn analyze_many_writes_one_subdirectory_per_repository() {
    let dir = tempdir().unwrap();
    let repo_one = make_tiny_repo(dir.path(), "repo-one");
    let repo_two = make_tiny_repo(dir.path(), "repo-two");

    let outdir = dir.path().join("out");
    let template = AnalyzeOptions {
        branch: "main".into(),
        outdir: outdir.clone(),
        quiet: true,
        procs: 1,
        ..Default::default()
    };
    let results = analyze_many(&[repo_one, repo_two], &template).unwrap();
    assert_eq!(results.len(), 2);

    for name in ["repo-one", "repo-two"] {
        let repo_outdir = outdir.join(name);
        for file in ["cohorts.json", "exts.json", "authors.json", "survival.json"] {
            let bytes = fs::read(repo_outdir.join(file))
                .unwrap_or_else(|e| panic!("reading {name}/{file}: {e}"));
            let _: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        }
    }
}

#[test]
fn analyze_many_dedupes_repositories_with_the_same_directory_name() {
    let dir = tempdir().unwrap();
    let group_a = dir.path().join("group-a");
    let group_b = dir.path().join("group-b");
    fs::create_dir_all(&group_a).unwrap();
    fs::create_dir_all(&group_b).unwrap();
    let repo_one = make_tiny_repo(&group_a, "repo");
    let repo_two = make_tiny_repo(&group_b, "repo");

    let outdir = dir.path().join("out");
    let template = AnalyzeOptions {
        branch: "main".into(),
        outdir: outdir.clone(),
        quiet: true,
        procs: 1,
        ..Default::default()
    };
    analyze_many(&[repo_one, repo_two], &template).unwrap();

    assert!(outdir.join("repo").join("cohorts.json").exists());
    assert!(outdir.join("repo-2").join("cohorts.json").exists());
}

#[test]
fn analyze_many_with_a_single_repo_matches_analyze() {
    let dir = tempdir().unwrap();
    let repo = make_tiny_repo(dir.path(), "solo");

    let outdir = dir.path().join("out");
    let template = AnalyzeOptions {
        branch: "main".into(),
        outdir: outdir.clone(),
        quiet: true,
        procs: 1,
        ..Default::default()
    };
    let results = analyze_many(std::slice::from_ref(&repo), &template).unwrap();
    assert_eq!(results.len(), 1);
    // Single-repo output goes directly to `outdir`, not a named subdirectory.
    assert!(outdir.join("cohorts.json").exists());
}

#[test]
fn analyze_many_merge_sums_overlapping_labels_across_repositories() {
    let dir = tempdir().unwrap();

    // repo-a: one commit in 2020, one line, author Alice.
    let repo_a = dir.path().join("repo-a");
    fs::create_dir_all(&repo_a).unwrap();
    run(&repo_a, &["init", "-q", "-b", "main"]);
    run(&repo_a, &["config", "commit.gpgsign", "false"]);
    fs::write(repo_a.join("a.py"), "print('one')\n").unwrap();
    run(&repo_a, &["add", "a.py"]);
    run_with_date(
        &repo_a,
        "2020-01-15T00:00:00Z",
        &["commit", "-q", "-m", "first-a"],
    );

    // repo-b: one commit in 2021, two lines, same author/extension/domain.
    let repo_b = dir.path().join("repo-b");
    fs::create_dir_all(&repo_b).unwrap();
    run(&repo_b, &["init", "-q", "-b", "main"]);
    run(&repo_b, &["config", "commit.gpgsign", "false"]);
    fs::write(repo_b.join("a.py"), "print('two')\nprint('three')\n").unwrap();
    run(&repo_b, &["add", "a.py"]);
    run_with_date(
        &repo_b,
        "2021-06-15T00:00:00Z",
        &["commit", "-q", "-m", "first-b"],
    );

    let outdir = dir.path().join("out");
    let template = AnalyzeOptions {
        branch: "main".into(),
        outdir: outdir.clone(),
        quiet: true,
        procs: 1,
        merge: true,
        ..Default::default()
    };
    let results = analyze_many(&[repo_a, repo_b], &template).unwrap();
    assert_eq!(results.len(), 1);
    let merged = &results[0];

    // Union of both repos' single sampled commit each: two timepoints.
    assert_eq!(merged.timestamps.len(), 2);

    // Both repos share the same extension/author/domain: values are
    // summed, forward-filling repo-a's count before repo-b's first commit.
    assert_eq!(merged.exts.get(".py"), Some(&vec![1, 3]));
    assert_eq!(merged.authors.get("Alice"), Some(&vec![1, 3]));
    assert_eq!(merged.domains.get("example.com"), Some(&vec![1, 3]));

    // Cohorts differ by year: each curve only grows once its own
    // repository's commit has landed.
    assert_eq!(merged.cohorts.get("2020"), Some(&vec![1, 1]));
    assert_eq!(merged.cohorts.get("2021"), Some(&vec![0, 2]));

    // Output lands directly in `outdir` -- no per-repo subdirectories.
    for file in [
        "cohorts.json",
        "exts.json",
        "authors.json",
        "dirs.json",
        "domains.json",
        "survival.json",
    ] {
        let bytes = fs::read(outdir.join(file)).unwrap_or_else(|e| panic!("reading {file}: {e}"));
        let _: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    }
    assert!(!outdir.join("repo-a").exists());
    assert!(!outdir.join("repo-b").exists());
}

#[test]
fn pipelined_analysis_matches_single_commit_windows() {
    let dir = tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    run(&repo, &["init", "-q", "-b", "main"]);
    run(&repo, &["config", "commit.gpgsign", "false"]);

    fs::write(repo.join("a.py"), "a = 1\n").unwrap();
    fs::write(repo.join("deleted.py"), "old = 1\n").unwrap();
    run(&repo, &["add", "."]);
    run_with_date(
        &repo,
        "2020-01-01T00:00:00Z",
        &["commit", "-q", "-m", "first"],
    );

    fs::write(repo.join("a.py"), "a = 2\nb = 1\n").unwrap();
    fs::write(repo.join("b.rs"), "fn main() {}\n").unwrap();
    run(&repo, &["add", "."]);
    run_with_date(
        &repo,
        "2020-02-01T00:00:00Z",
        &["commit", "-q", "-m", "second"],
    );

    fs::remove_file(repo.join("deleted.py")).unwrap();
    fs::write(repo.join("a.py"), "a = 3\nb = 1\nc = 1\n").unwrap();
    run(&repo, &["add", "-A"]);
    run_with_date(
        &repo,
        "2020-03-01T00:00:00Z",
        &["commit", "-q", "-m", "third"],
    );

    let sequential_out = dir.path().join("sequential");
    analyze(&AnalyzeOptions {
        repo_dir: repo.clone(),
        branch: "main".into(),
        outdir: sequential_out.clone(),
        quiet: true,
        procs: 1,
        ..Default::default()
    })
    .unwrap();

    let pipelined_out = dir.path().join("pipelined");
    analyze(&AnalyzeOptions {
        repo_dir: repo,
        branch: "main".into(),
        outdir: pipelined_out.clone(),
        quiet: true,
        procs: 4,
        ..Default::default()
    })
    .unwrap();

    for file in [
        "cohorts.json",
        "exts.json",
        "authors.json",
        "dirs.json",
        "domains.json",
        "survival.json",
    ] {
        assert_eq!(
            fs::read(sequential_out.join(file)).unwrap(),
            fs::read(pipelined_out.join(file)).unwrap(),
            "{file} changed with pipeline width"
        );
    }
}

/// A repository's `.mailmap` file should rewrite author identities the same
/// way `git check-mailmap` (and the Python `get_mailmap_author_name_email`
/// helper it ported) would: commits authored under an old name/email are
/// folded into the canonical identity, for both the `authors` and `domains`
/// curves.
#[test]
fn analyze_applies_mailmap_author_rewriting() {
    let dir = tempdir().unwrap();
    let repo = dir.path();
    run(repo, &["init", "-q", "-b", "main"]);
    run(repo, &["config", "commit.gpgsign", "false"]);

    fn run_as_old_identity(repo: &Path, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "Alice")
            .env("GIT_AUTHOR_EMAIL", "alice@oldmail.com")
            .env("GIT_COMMITTER_NAME", "Alice")
            .env("GIT_COMMITTER_EMAIL", "alice@oldmail.com")
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git available");
        assert!(status.success(), "git {args:?} failed");
    }

    fs::write(
        repo.join(".mailmap"),
        "Alice Wonderland <alice@newmail.com> Alice <alice@oldmail.com>\n",
    )
    .unwrap();
    run(repo, &["add", ".mailmap"]);
    run_as_old_identity(
        repo,
        "2020-01-01T00:00:00Z",
        &["commit", "-q", "-m", "add mailmap"],
    );

    fs::write(repo.join("a.py"), "print('hi')\n").unwrap();
    run(repo, &["add", "a.py"]);
    run_as_old_identity(
        repo,
        "2020-01-20T00:00:00Z",
        &["commit", "-q", "-m", "add a.py"],
    );

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

    // The mailmapped name/domain are present, the raw pre-mailmap identity
    // is not.
    assert!(result.authors.contains_key("Alice Wonderland"));
    assert!(!result.authors.contains_key("Alice"));
    assert!(result.domains.contains_key("newmail.com"));
    assert!(!result.domains.contains_key("oldmail.com"));

    let authors: serde_json::Value =
        serde_json::from_slice(&fs::read(outdir.join("authors.json")).unwrap()).unwrap();
    let labels: Vec<&str> = authors["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(labels.contains(&"Alice Wonderland"));
    assert!(!labels.contains(&"Alice"));
}
