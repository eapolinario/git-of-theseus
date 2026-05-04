//! Core analysis: walks a git repository's history along one branch, samples
//! commits at fixed time intervals, runs `git blame` on each sampled
//! revision (with a fast-diff optimisation that skips unchanged blobs) and
//! aggregates surviving line counts by cohort / extension / author / dir /
//! domain / commit SHA.
//!
//! This is a port of `git_of_theseus/analyze.py` to Rust. The output JSON
//! files match the Python schema byte-for-byte so the existing
//! `git-of-theseus-stack-plot` / `-line-plot` / `-survival-plot` Python
//! commands can consume them unchanged.
//!
//! Features intentionally deferred to follow-up PRs:
//! - `mailmap` author rewriting (`get_mailmap_author_name_email` in Python)
//! - `--opt` git-commit-graph generation
//! - Interactive SIGINT pause / process-count adjustment

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use git2::{BlameOptions, Oid, Repository, Sort};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::cohort::format_cohort;
use crate::output::{write_curve_json, write_survival_json};
use crate::path_filter::{extension, top_dir, PathFilter};

/// Default interval between sampled commits (one week, in seconds), matching
/// the Python CLI default.
pub const DEFAULT_INTERVAL_SECS: i64 = 7 * 24 * 60 * 60;

/// Survival series for a single commit: `[[unix_ts, surviving_lines], ...]`.
pub type SurvivalSeries = Vec<(i64, u64)>;

/// User-facing parameters for `analyze`. Mirrors the keyword arguments of
/// `git_of_theseus.analyze.analyze`.
#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    pub repo_dir: PathBuf,
    pub branch: String,
    pub cohort_format: String,
    pub interval_secs: i64,
    pub only: Vec<String>,
    pub ignore: Vec<String>,
    pub all_filetypes: bool,
    pub ignore_whitespace: bool,
    pub procs: usize,
    pub quiet: bool,
    pub outdir: PathBuf,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            repo_dir: PathBuf::from("."),
            branch: "master".to_string(),
            cohort_format: "%Y".to_string(),
            interval_secs: DEFAULT_INTERVAL_SECS,
            only: Vec::new(),
            ignore: Vec::new(),
            all_filetypes: false,
            ignore_whitespace: false,
            procs: num_cpus_default(),
            quiet: false,
            outdir: PathBuf::from("."),
        }
    }
}

fn num_cpus_default() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Categories used when keying curves. Mirrors the `(category, key)` tuples
/// in the Python implementation.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
enum Category {
    Cohort,
    Ext,
    Author,
    Dir,
    Domain,
    Sha,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
struct Key(Category, String);

/// Per-(path) histogram returned by blaming a single file: maps each curve
/// key it contributes to onto the number of lines it contributes.
type FileHistogram = HashMap<Key, u64>;

/// In-memory result of `analyze`, primarily useful for testing. The CLI
/// path additionally writes JSON files via `write_outputs`.
#[derive(Debug)]
pub struct AnalyzeResult {
    pub timestamps: Vec<DateTime<Utc>>,
    pub cohorts: BTreeMap<String, Vec<u64>>,
    pub exts: BTreeMap<String, Vec<u64>>,
    pub authors: BTreeMap<String, Vec<u64>>,
    pub dirs: BTreeMap<String, Vec<u64>>,
    pub domains: BTreeMap<String, Vec<u64>>,
    pub survival: BTreeMap<String, SurvivalSeries>,
}

/// Runs the full analysis and writes the standard set of JSON output files
/// to `options.outdir`. Returns the in-memory result for callers that want
/// to inspect it (tests, library consumers).
pub fn analyze(options: &AnalyzeOptions) -> Result<AnalyzeResult> {
    let result = analyze_in_memory(options)?;
    write_outputs(options, &result)?;
    Ok(result)
}

/// Runs the analysis but does not touch the filesystem. Useful for tests
/// and embedding in WASM / library contexts.
pub fn analyze_in_memory(options: &AnalyzeOptions) -> Result<AnalyzeResult> {
    let repo = Repository::open(&options.repo_dir)
        .with_context(|| format!("opening repository {}", options.repo_dir.display()))?;

    let branch_oid = resolve_branch(&repo, &options.branch, options.quiet)?;
    let filter = PathFilter::new(&options.only, &options.ignore, options.all_filetypes)?;

    // Step 1: walk every reachable commit on the branch, build cohort map
    // and the up-front `curve_key_tuples` for cohort / author / domain.
    let progress = make_bar(options.quiet, "Listing all commits", None);
    let mut commit2cohort: HashMap<Oid, String> = HashMap::new();
    let mut cohort_set: HashSet<String> = HashSet::new();
    let mut author_set: HashSet<String> = HashSet::new();
    let mut domain_set: HashSet<String> = HashSet::new();

    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    walk.push(branch_oid)?;
    for oid in walk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let committed_at = Utc
            .timestamp_opt(commit.time().seconds(), 0)
            .single()
            .ok_or_else(|| anyhow!("invalid timestamp on commit {oid}"))?;
        let cohort = format_cohort(committed_at, &options.cohort_format)?;
        commit2cohort.insert(oid, cohort.clone());
        cohort_set.insert(cohort);
        let author = commit.author();
        let name = author.name().unwrap_or("").to_string();
        let email = author.email().unwrap_or("").to_string();
        author_set.insert(name);
        domain_set.insert(extract_domain(&email));
        progress.inc(1);
    }
    progress.finish_and_clear();

    // Step 2: backtrack along first-parent of HEAD (the Python code uses
    // `repo.head.commit.parents[0]`), sampling at `interval_secs`.
    let progress = make_bar(options.quiet, "Backtracking the master branch", None);
    let mut sampled: Vec<(Oid, i64)> = Vec::new();
    let mut current = repo.find_commit(branch_oid)?;
    let mut last_date: Option<i64> = None;
    loop {
        let date = current.time().seconds();
        if last_date.map_or(true, |last| date < last - options.interval_secs) {
            sampled.push((current.id(), date));
            last_date = Some(date);
        }
        progress.inc(1);
        if current.parent_count() == 0 {
            break;
        }
        current = current.parent(0)?;
    }
    progress.finish_and_clear();
    sampled.reverse(); // chronological ascending

    // Step 3: for each sampled commit, walk the tree and collect blob
    // entries that pass the path filter. Cache the entries; also build
    // `ext_set` / `dir_set` for the curve keys.
    let mut ext_set: HashSet<String> = HashSet::new();
    let mut dir_set: HashSet<String> = HashSet::new();
    let mut entries_per_commit: Vec<Vec<TreeEntry>> = Vec::with_capacity(sampled.len());

    let progress = make_bar(
        options.quiet,
        "Discovering entries",
        Some(sampled.len() as u64),
    );
    for (oid, _) in &sampled {
        let commit = repo.find_commit(*oid)?;
        let tree = commit.tree()?;
        let entries = collect_blob_entries(&repo, &tree, &filter)?;
        for entry in &entries {
            ext_set.insert(extension(&entry.path));
            dir_set.insert(top_dir(&entry.path));
        }
        entries_per_commit.push(entries);
        progress.inc(1);
    }
    progress.finish_and_clear();

    // Step 4: walk sampled commits chronologically, performing fast-diff
    // and blame to update per-commit cumulative state.
    let timestamps: Vec<DateTime<Utc>> = sampled
        .iter()
        .map(|(_, ts)| Utc.timestamp_opt(*ts, 0).single().expect("valid ts"))
        .collect();

    let mut cur_y: HashMap<Key, u64> = HashMap::new();
    let mut last_file_y: HashMap<String, FileHistogram> = HashMap::new();
    let mut last_file_hash: HashMap<String, Oid> = HashMap::new();
    let mut commit_history: BTreeMap<String, SurvivalSeries> = BTreeMap::new();

    let cohort_keys: Vec<Key> = cohort_set
        .iter()
        .map(|c| Key(Category::Cohort, c.clone()))
        .collect();
    let ext_keys: Vec<Key> = ext_set
        .iter()
        .map(|e| Key(Category::Ext, e.clone()))
        .collect();
    let author_keys: Vec<Key> = author_set
        .iter()
        .map(|a| Key(Category::Author, a.clone()))
        .collect();
    let dir_keys: Vec<Key> = dir_set
        .iter()
        .map(|d| Key(Category::Dir, d.clone()))
        .collect();
    let domain_keys: Vec<Key> = domain_set
        .iter()
        .map(|d| Key(Category::Domain, d.clone()))
        .collect();

    let mut curves: HashMap<Key, Vec<u64>> = HashMap::new();
    for k in cohort_keys
        .iter()
        .chain(ext_keys.iter())
        .chain(author_keys.iter())
        .chain(dir_keys.iter())
        .chain(domain_keys.iter())
    {
        curves.insert(k.clone(), Vec::with_capacity(sampled.len()));
    }

    let total_entries: u64 = entries_per_commit.iter().map(|e| e.len() as u64).sum();
    let progress = make_bar(
        options.quiet,
        "Analyzing commits (blame)",
        Some(total_entries),
    );

    let pool = build_thread_pool(options.procs)?;

    for (commit_idx, (commit_oid, commit_ts)) in sampled.iter().enumerate() {
        let entries = std::mem::take(&mut entries_per_commit[commit_idx]);

        // Fast-diff: collect entries to actually blame, subtracting
        // contributions from modified or deleted files.
        let mut cur_file_hash: HashMap<String, Oid> = HashMap::new();
        let mut to_blame: Vec<TreeEntry> = Vec::new();
        for entry in &entries {
            cur_file_hash.insert(entry.path.clone(), entry.blob_oid);
            match last_file_hash.get(&entry.path) {
                Some(prev_oid) if *prev_oid == entry.blob_oid => {
                    // Identical file: nothing to do.
                    progress.inc(1);
                }
                Some(_) => {
                    // Modified: subtract previous contribution, will re-blame.
                    if let Some(prev) = last_file_y.remove(&entry.path) {
                        for (key, count) in prev {
                            if let Some(v) = cur_y.get_mut(&key) {
                                *v = v.saturating_sub(count);
                            }
                        }
                    }
                    to_blame.push(entry.clone());
                }
                None => {
                    // Newly added file.
                    to_blame.push(entry.clone());
                }
            }
            last_file_hash.remove(&entry.path);
        }
        // Whatever remains in `last_file_hash` from the previous iteration
        // are deleted files; subtract their contributions.
        for (deleted, _) in last_file_hash.drain() {
            if let Some(prev) = last_file_y.remove(&deleted) {
                for (key, count) in prev {
                    if let Some(v) = cur_y.get_mut(&key) {
                        *v = v.saturating_sub(count);
                    }
                }
            }
        }
        last_file_hash = cur_file_hash;

        // Blame the changed files (in parallel).
        let blame_results = blame_files(
            &pool,
            &options.repo_dir,
            *commit_oid,
            &to_blame,
            &commit2cohort,
            options.ignore_whitespace,
            &progress,
        )?;
        for (path, hist) in blame_results {
            for (key, count) in &hist {
                *cur_y.entry(key.clone()).or_insert(0) += *count;
            }
            last_file_y.insert(path, hist);
        }

        // Snapshot per-curve values for this sampled commit.
        for (key, series) in curves.iter_mut() {
            series.push(*cur_y.get(key).unwrap_or(&0));
        }
        // Survival data: any `(Sha, sha)` entry in cur_y with a non-zero
        // count contributes (commit_ts, count) at this point in time.
        for (key, count) in cur_y.iter() {
            if let Key(Category::Sha, sha) = key {
                if *count > 0 {
                    commit_history
                        .entry(sha.clone())
                        .or_default()
                        .push((*commit_ts, *count));
                }
            }
        }
    }
    progress.finish_and_clear();

    // Convert curves to BTreeMap<String, Vec<u64>> grouped by category, with
    // alphabetically sorted labels (matches Python `sorted(...)`).
    let mut cohorts: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut exts: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut authors: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut dirs: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut domains: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for (Key(category, label), series) in curves.into_iter() {
        let target = match category {
            Category::Cohort => &mut cohorts,
            Category::Ext => &mut exts,
            Category::Author => &mut authors,
            Category::Dir => &mut dirs,
            Category::Domain => &mut domains,
            Category::Sha => continue, // sha is survival-only
        };
        target.insert(label, series);
    }

    Ok(AnalyzeResult {
        timestamps,
        cohorts,
        exts,
        authors,
        dirs,
        domains,
        survival: commit_history,
    })
}

/// Writes `cohorts.json`, `exts.json`, `authors.json`, `dirs.json`,
/// `domains.json` and `survival.json` to `options.outdir`.
pub fn write_outputs(options: &AnalyzeOptions, result: &AnalyzeResult) -> Result<()> {
    std::fs::create_dir_all(&options.outdir)
        .with_context(|| format!("creating outdir {}", options.outdir.display()))?;
    write_curve_json(
        options.outdir.join("cohorts.json"),
        &result.cohorts,
        &result.timestamps,
        |c| format!("Code added in {c}"),
    )?;
    write_curve_json(
        options.outdir.join("exts.json"),
        &result.exts,
        &result.timestamps,
        |s| s.to_string(),
    )?;
    write_curve_json(
        options.outdir.join("authors.json"),
        &result.authors,
        &result.timestamps,
        |s| s.to_string(),
    )?;
    write_curve_json(
        options.outdir.join("dirs.json"),
        &result.dirs,
        &result.timestamps,
        |s| s.to_string(),
    )?;
    write_curve_json(
        options.outdir.join("domains.json"),
        &result.domains,
        &result.timestamps,
        |s| s.to_string(),
    )?;
    write_survival_json(options.outdir.join("survival.json"), &result.survival)?;
    Ok(())
}

#[derive(Clone, Debug)]
struct TreeEntry {
    path: String,
    blob_oid: Oid,
}

fn collect_blob_entries(
    _repo: &Repository,
    tree: &git2::Tree<'_>,
    filter: &PathFilter,
) -> Result<Vec<TreeEntry>> {
    let mut out = Vec::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() != Some(git2::ObjectType::Blob) {
            return git2::TreeWalkResult::Ok;
        }
        let name = match entry.name() {
            Some(n) => n,
            None => return git2::TreeWalkResult::Ok, // skip non-utf8 names
        };
        let path = if dir.is_empty() {
            name.to_string()
        } else {
            format!("{dir}{name}")
        };
        if filter.allows(&path) {
            out.push(TreeEntry {
                path,
                blob_oid: entry.id(),
            });
        }
        git2::TreeWalkResult::Ok
    })?;
    Ok(out)
}

fn extract_domain(email: &str) -> String {
    match email.rfind('@') {
        Some(idx) => email[idx + 1..].to_string(),
        None => email.to_string(),
    }
}

fn resolve_branch(repo: &Repository, branch: &str, quiet: bool) -> Result<Oid> {
    if let Ok(reference) = repo.find_reference(&format!("refs/heads/{branch}")) {
        if let Some(oid) = reference.target() {
            return Ok(oid);
        }
    }
    // Fallback: HEAD (handles detached HEAD too).
    let head = repo.head().context("resolving HEAD")?;
    let head_oid = head
        .target()
        .ok_or_else(|| anyhow!("HEAD is not a direct reference"))?;
    if !quiet {
        eprintln!(
            "warning: requested branch '{branch}' does not exist; falling back to HEAD ({head_oid})"
        );
    }
    Ok(head_oid)
}

fn build_thread_pool(procs: usize) -> Result<rayon::ThreadPool> {
    let n = if procs == 0 { 1 } else { procs };
    rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .build()
        .map_err(|e| anyhow!("building thread pool: {e}"))
}

/// Blames each entry at `commit_oid` and returns `(path, histogram)` pairs.
/// Each worker thread opens its own `git2::Repository` because `Repository`
/// is not `Sync`.
fn blame_files(
    pool: &rayon::ThreadPool,
    repo_dir: &Path,
    commit_oid: Oid,
    entries: &[TreeEntry],
    commit2cohort: &HashMap<Oid, String>,
    ignore_whitespace: bool,
    progress: &ProgressBar,
) -> Result<Vec<(String, FileHistogram)>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let results: Vec<Result<(String, FileHistogram)>> = pool.install(|| {
        entries
            .par_iter()
            .map_init(
                || Repository::open(repo_dir).context("opening repo on worker"),
                |repo_result, entry| -> Result<(String, FileHistogram)> {
                    let repo = repo_result.as_ref().map_err(|e| anyhow!("{e}"))?;
                    let mut opts = BlameOptions::new();
                    opts.newest_commit(commit_oid);
                    if ignore_whitespace {
                        opts.ignore_whitespace(true);
                    }
                    let hist = blame_one(repo, entry, &mut opts, commit2cohort);
                    progress.inc(1);
                    Ok((entry.path.clone(), hist.unwrap_or_default()))
                },
            )
            .collect()
    });
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        out.push(r?);
    }
    Ok(out)
}

fn blame_one(
    repo: &Repository,
    entry: &TreeEntry,
    opts: &mut BlameOptions,
    commit2cohort: &HashMap<Oid, String>,
) -> Result<FileHistogram> {
    let blame = repo.blame_file(Path::new(&entry.path), Some(opts))?;
    let mut h: FileHistogram = HashMap::new();
    for hunk in blame.iter() {
        let lines = hunk.lines_in_hunk() as u64;
        if lines == 0 {
            continue;
        }
        let orig_oid = hunk.orig_commit_id();
        let signature = hunk.orig_signature();
        let author_name = signature.name().unwrap_or("").to_string();
        let author_email = signature.email().unwrap_or("").to_string();

        let cohort = commit2cohort
            .get(&orig_oid)
            .cloned()
            .unwrap_or_else(|| "MISSING".to_string());
        let ext = extension(&entry.path);
        let dir = top_dir(&entry.path);
        let domain = extract_domain(&author_email);

        let keys = [
            Key(Category::Cohort, cohort),
            Key(Category::Ext, ext),
            Key(Category::Author, author_name),
            Key(Category::Dir, dir),
            Key(Category::Domain, domain),
        ];
        for key in keys {
            *h.entry(key).or_insert(0) += lines;
        }
        if commit2cohort.contains_key(&orig_oid) {
            *h.entry(Key(Category::Sha, orig_oid.to_string()))
                .or_insert(0) += lines;
        }
    }
    Ok(h)
}

fn make_bar(quiet: bool, msg: &str, total: Option<u64>) -> ProgressBar {
    if quiet {
        return ProgressBar::hidden();
    }
    let bar = match total {
        Some(t) => ProgressBar::new(t),
        None => ProgressBar::new_spinner(),
    };
    let style = match total {
        Some(_) => {
            ProgressStyle::with_template("{msg:<55} [{bar:30}] {pos}/{len} ({elapsed_precise})")
        }
        None => ProgressStyle::with_template("{msg:<55} {pos} ({elapsed_precise})"),
    };
    if let Ok(s) = style {
        bar.set_style(s);
    }
    bar.set_message(msg.to_string());
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_domain_handles_emails() {
        assert_eq!(extract_domain("alice@example.com"), "example.com");
        assert_eq!(extract_domain("noemail"), "noemail");
        assert_eq!(extract_domain(""), "");
    }
}
