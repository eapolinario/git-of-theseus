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
//! Author identities are normalized through the repository's `.mailmap`
//! (mirroring `get_mailmap_author_name_email` in the Python implementation)
//! via `git2::Repository::mailmap` / `Mailmap::resolve_signature`.
//!
//! Features intentionally deferred to follow-up PRs:
//! - `--opt` git-commit-graph generation
//! - Interactive SIGINT pause / process-count adjustment

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use git2::{BlameOptions, Mailmap, Oid, Repository, Signature, Sort};
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

/// Timing statistics for performance measurement.
#[derive(Debug, Clone, Default)]
pub struct TimingStats {
    pub blame_time_us: Arc<AtomicU64>,
    pub post_blame_time_us: Arc<AtomicU64>,
    pub fastdiff_time_us: Arc<AtomicU64>,
    pub tree_discovery_time_us: Arc<AtomicU64>,
    pub commit_walk_time_us: Arc<AtomicU64>,
    pub files_blamed: Arc<AtomicU64>,
}

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
    /// When analyzing multiple repositories, combine them into a single set
    /// of output files instead of one subdirectory per repository. Ignored
    /// when only one repository is given. See [`merge_results`].
    pub merge: bool,
    /// Enable detailed timing measurements.
    pub measure_time: bool,
    /// Timing statistics (only populated if measure_time is true).
    pub timing: TimingStats,
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
            merge: false,
            measure_time: false,
            timing: TimingStats::default(),
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

/// Analyzes one or more repositories, mirroring the multi-repo dispatch in
/// `git_of_theseus.analyze.analyze`.
///
/// With a single repository this behaves exactly like calling [`analyze`]
/// directly with `options.repo_dir` set to it. With multiple repositories
/// and `options.merge` unset, each one is analyzed independently and its
/// JSON output is written to a subdirectory of `options.outdir` named
/// after the repository's directory name (with a `-2`, `-3`, ... suffix
/// on name collisions). With multiple repositories and `options.merge`
/// set, all repositories are combined into a single set of output files
/// written directly to `options.outdir`; see [`merge_results`].
pub fn analyze_many(repo_dirs: &[PathBuf], options: &AnalyzeOptions) -> Result<Vec<AnalyzeResult>> {
    anyhow::ensure!(!repo_dirs.is_empty(), "at least one repository is required");

    if repo_dirs.len() == 1 {
        let mut opts = options.clone();
        opts.repo_dir = repo_dirs[0].clone();
        return Ok(vec![analyze(&opts)?]);
    }

    // Built once and shared across every repository below, instead of each
    // repository spawning (and tearing down) its own worker threads.
    let pool = build_thread_pool(options.procs)?;

    if options.merge {
        let mut results = Vec::with_capacity(repo_dirs.len());
        for repo_dir in repo_dirs {
            let mut opts = options.clone();
            opts.repo_dir = repo_dir.clone();
            results.push(analyze_in_memory_with_pool(&opts, &pool)?);
        }
        let merged = merge_results(&results)?;
        write_outputs(options, &merged)?;
        return Ok(vec![merged]);
    }

    std::fs::create_dir_all(&options.outdir)
        .with_context(|| format!("creating outdir {}", options.outdir.display()))?;

    let mut used_names: HashSet<String> = HashSet::new();
    let mut results = Vec::with_capacity(repo_dirs.len());
    for repo_dir in repo_dirs {
        let name = unique_repo_output_name(repo_dir, &mut used_names);
        let mut opts = options.clone();
        opts.repo_dir = repo_dir.clone();
        opts.outdir = options.outdir.join(name);
        let result = analyze_in_memory_with_pool(&opts, &pool)?;
        write_outputs(&opts, &result)?;
        results.push(result);
    }
    Ok(results)
}

/// Picks a unique subdirectory name for a repository's output, based on
/// its (canonicalized, where possible) directory name. Mirrors the
/// collision handling in the Python `analyze()` dispatcher.
fn unique_repo_output_name(repo_dir: &Path, used: &mut HashSet<String>) -> String {
    let base = repo_dir
        .canonicalize()
        .unwrap_or_else(|_| repo_dir.to_path_buf())
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("repository")
        .to_string();

    let mut name = base.clone();
    let mut suffix = 2;
    while used.contains(&name) {
        name = format!("{base}-{suffix}");
        suffix += 1;
    }
    used.insert(name.clone());
    name
}

/// Merges multiple repositories' analysis results into a single combined
/// result, used by [`analyze_many`] when [`AnalyzeOptions::merge`] is set.
///
/// The combined timeline is the union of every input's sampled timestamps,
/// sorted ascending. Each repository's per-label series (cohorts / exts /
/// authors / dirs / domains) is resampled onto that shared timeline by
/// carrying its last known value forward (starting at 0 before the
/// repository's first sample) and then summed label-by-label across all
/// repositories: e.g. an author present in two repositories gets one
/// combined curve, the elementwise sum of their (resampled) contributions.
/// A label present in only one repository is unaffected other than being
/// resampled onto the shared timeline.
///
/// Survival data is keyed by commit SHA, already a globally-unique
/// identifier, so per-repository series are unioned as-is; in the
/// (practically impossible) event that two repositories share a commit,
/// their series are concatenated and re-sorted by timestamp so downstream
/// consumers can still assume chronological order.
pub fn merge_results(results: &[AnalyzeResult]) -> Result<AnalyzeResult> {
    anyhow::ensure!(
        !results.is_empty(),
        "at least one result is required to merge"
    );

    let mut ts_set: BTreeSet<DateTime<Utc>> = BTreeSet::new();
    for r in results {
        ts_set.extend(r.timestamps.iter().copied());
    }
    let timestamps: Vec<DateTime<Utc>> = ts_set.into_iter().collect();

    let cohorts = merge_curve_maps(results, |r| &r.cohorts, &timestamps);
    let exts = merge_curve_maps(results, |r| &r.exts, &timestamps);
    let authors = merge_curve_maps(results, |r| &r.authors, &timestamps);
    let dirs = merge_curve_maps(results, |r| &r.dirs, &timestamps);
    let domains = merge_curve_maps(results, |r| &r.domains, &timestamps);

    let mut survival: BTreeMap<String, SurvivalSeries> = BTreeMap::new();
    for r in results {
        for (sha, series) in &r.survival {
            survival
                .entry(sha.clone())
                .or_default()
                .extend(series.iter().copied());
        }
    }
    for series in survival.values_mut() {
        series.sort_by_key(|&(ts, _)| ts);
    }

    Ok(AnalyzeResult {
        timestamps,
        cohorts,
        exts,
        authors,
        dirs,
        domains,
        survival,
    })
}

/// Resamples each result's per-label series (selected by `select`) onto
/// `merged_ts` -- carrying the last known value forward, starting at 0
/// before that repository's first sample -- and sums label-by-label across
/// all results.
fn merge_curve_maps(
    results: &[AnalyzeResult],
    select: impl Fn(&AnalyzeResult) -> &BTreeMap<String, Vec<u64>>,
    merged_ts: &[DateTime<Utc>],
) -> BTreeMap<String, Vec<u64>> {
    let mut out: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for r in results {
        let positions: HashMap<DateTime<Utc>, usize> = r
            .timestamps
            .iter()
            .enumerate()
            .map(|(i, t)| (*t, i))
            .collect();
        for (label, row) in select(r) {
            let entry = out
                .entry(label.clone())
                .or_insert_with(|| vec![0u64; merged_ts.len()]);
            let mut current = 0u64;
            for (i, t) in merged_ts.iter().enumerate() {
                if let Some(&idx) = positions.get(t) {
                    current = row[idx];
                }
                entry[i] += current;
            }
        }
    }
    out
}

/// Runs the analysis but does not touch the filesystem. Useful for tests
/// and embedding in WASM / library contexts.
pub fn analyze_in_memory(options: &AnalyzeOptions) -> Result<AnalyzeResult> {
    let pool = build_thread_pool(options.procs)?;
    analyze_in_memory_with_pool(options, &pool)
}

/// Same as [`analyze_in_memory`], but reuses a caller-provided thread pool
/// instead of building a new one. [`analyze_many`] uses this so that
/// multiple repositories share one pool -- and one set of worker threads
/// -- instead of each repository spawning (and tearing down) its own.
fn analyze_in_memory_with_pool(
    options: &AnalyzeOptions,
    pool: &rayon::ThreadPool,
) -> Result<AnalyzeResult> {
    let repo = Repository::open(&options.repo_dir)
        .with_context(|| format!("opening repository {}", options.repo_dir.display()))?;

    let branch_oid = resolve_branch(&repo, &options.branch, options.quiet)?;
    let filter = PathFilter::new(&options.only, &options.ignore, options.all_filetypes)?;

    // Step 1: walk every reachable commit on the branch, build cohort map
    // and the up-front `curve_key_tuples` for cohort / author / domain.
    let progress = make_bar(options.quiet, "Listing all commits", None);
    let commit_walk_start = if options.measure_time {
        Some(Instant::now())
    } else {
        None
    };
    let mut commit2cohort: HashMap<Oid, String> = HashMap::new();
    let mut cohort_set: HashSet<String> = HashSet::new();
    let mut author_set: HashSet<String> = HashSet::new();
    let mut domain_set: HashSet<String> = HashSet::new();

    let mailmap = repo.mailmap().context("loading repository mailmap")?;

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
        let (name, email) = mailmap_author_name_email(&mailmap, &commit.author());
        author_set.insert(name);
        domain_set.insert(extract_domain(&email));
        progress.inc(1);
    }
    progress.finish_and_clear();
    if let Some(start) = commit_walk_start {
        let elapsed_us = start.elapsed().as_micros() as u64;
        options
            .timing
            .commit_walk_time_us
            .fetch_add(elapsed_us, Ordering::Relaxed);
    }

    // Step 2: backtrack along first-parent of HEAD (the Python code uses
    // `repo.head.commit.parents[0]`), sampling at `interval_secs`.
    let progress = make_bar(options.quiet, "Backtracking the master branch", None);
    let backtrack_start = if options.measure_time {
        Some(Instant::now())
    } else {
        None
    };
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
    if let Some(start) = backtrack_start {
        let elapsed_us = start.elapsed().as_micros() as u64;
        options
            .timing
            .commit_walk_time_us
            .fetch_add(elapsed_us, Ordering::Relaxed);
    }
    sampled.reverse(); // chronological ascending

    // Step 3: for each sampled commit, walk the tree and collect blob
    // entries that pass the path filter (in parallel, one worker thread
    // per sampled commit). Cache the entries; also build `ext_set` /
    // `dir_set` for the curve keys.
    let mut ext_set: HashSet<String> = HashSet::new();
    let mut dir_set: HashSet<String> = HashSet::new();

    let progress = make_bar(
        options.quiet,
        "Discovering entries",
        Some(sampled.len() as u64),
    );
    let discovery_start = if options.measure_time {
        Some(Instant::now())
    } else {
        None
    };
    let mut entries_per_commit =
        discover_entries(pool, &options.repo_dir, &sampled, &filter, &progress)?;
    if let Some(start) = discovery_start {
        let elapsed_us = start.elapsed().as_micros() as u64;
        options
            .timing
            .tree_discovery_time_us
            .fetch_add(elapsed_us, Ordering::Relaxed);
    }
    for entries in &entries_per_commit {
        for entry in entries {
            ext_set.insert(extension(&entry.path));
            dir_set.insert(top_dir(&entry.path));
        }
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

    let fastdiff_start = options.measure_time.then(Instant::now);
    let commit_plans = plan_commits(&sampled, &mut entries_per_commit, &progress);
    if let Some(start) = fastdiff_start {
        options
            .timing
            .fastdiff_time_us
            .fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);
    }

    // A window lets files from adjacent commits use idle Rayon workers.
    // Results are still applied one complete commit at a time in sampled
    // order because the cumulative histogram depends on chronological state.
    let window_size = options.procs.max(1);
    for window in commit_plans.chunks(window_size) {
        let blame_results = blame_commit_window(
            pool,
            &options.repo_dir,
            window,
            &commit2cohort,
            options.ignore_whitespace,
            &progress,
            &options.timing,
            options.measure_time,
        )?;

        for (plan, results) in window.iter().zip(blame_results) {
            let agg_start = options.measure_time.then(Instant::now);
            for path in &plan.paths_to_remove {
                if let Some(previous) = last_file_y.remove(path) {
                    subtract_histogram(&mut cur_y, previous);
                }
            }
            for (path, hist) in results {
                for (key, count) in &hist {
                    *cur_y.entry(key.clone()).or_insert(0) += *count;
                }
                last_file_y.insert(path, hist);
            }
            if let Some(start) = agg_start {
                options
                    .timing
                    .post_blame_time_us
                    .fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);
            }

            for (key, series) in curves.iter_mut() {
                series.push(*cur_y.get(key).unwrap_or(&0));
            }
            for (key, count) in cur_y.iter() {
                if let Key(Category::Sha, sha) = key {
                    if *count > 0 {
                        commit_history
                            .entry(sha.clone())
                            .or_default()
                            .push((plan.commit_ts, *count));
                    }
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

#[derive(Debug)]
struct CommitPlan {
    commit_oid: Oid,
    commit_ts: i64,
    paths_to_remove: Vec<String>,
    to_blame: Vec<TreeEntry>,
}

fn plan_commits(
    sampled: &[(Oid, i64)],
    entries_per_commit: &mut [Vec<TreeEntry>],
    progress: &ProgressBar,
) -> Vec<CommitPlan> {
    let mut last_file_hash: HashMap<String, Oid> = HashMap::new();
    let mut plans = Vec::with_capacity(sampled.len());

    for (commit_idx, (commit_oid, commit_ts)) in sampled.iter().enumerate() {
        let entries = std::mem::take(&mut entries_per_commit[commit_idx]);
        let mut cur_file_hash = HashMap::with_capacity(entries.len());
        let mut paths_to_remove = Vec::new();
        let mut to_blame = Vec::new();

        for entry in entries {
            cur_file_hash.insert(entry.path.clone(), entry.blob_oid);
            match last_file_hash.remove(&entry.path) {
                Some(previous_oid) if previous_oid == entry.blob_oid => progress.inc(1),
                Some(_) => {
                    paths_to_remove.push(entry.path.clone());
                    to_blame.push(entry);
                }
                None => to_blame.push(entry),
            }
        }
        paths_to_remove.extend(last_file_hash.drain().map(|(path, _)| path));
        last_file_hash = cur_file_hash;
        plans.push(CommitPlan {
            commit_oid: *commit_oid,
            commit_ts: *commit_ts,
            paths_to_remove,
            to_blame,
        });
    }

    plans
}

fn subtract_histogram(cur_y: &mut HashMap<Key, u64>, histogram: FileHistogram) {
    for (key, count) in histogram {
        if let Some(value) = cur_y.get_mut(&key) {
            *value = value.saturating_sub(count);
        }
    }
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

/// Resolves `sig` against `mailmap`, mirroring the Python
/// `get_mailmap_author_name_email` helper: the name and email are rewritten
/// according to the repository's `.mailmap` (falling back to the original
/// identity if resolution fails or fields are missing).
fn mailmap_author_name_email(mailmap: &Mailmap, sig: &Signature<'_>) -> (String, String) {
    match mailmap.resolve_signature(sig) {
        Ok(resolved) => (
            resolved.name().unwrap_or("").to_string(),
            resolved.email().unwrap_or("").to_string(),
        ),
        Err(_) => (
            sig.name().unwrap_or("").to_string(),
            sig.email().unwrap_or("").to_string(),
        ),
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

/// Walks the tree of each sampled commit in parallel and collects the blob
/// entries that pass `filter`, returning one entry list per sampled commit
/// in the same order as `sampled`. Each worker thread opens its own
/// `git2::Repository` because `Repository` is not `Sync`.
fn discover_entries(
    pool: &rayon::ThreadPool,
    repo_dir: &Path,
    sampled: &[(Oid, i64)],
    filter: &PathFilter,
    progress: &ProgressBar,
) -> Result<Vec<Vec<TreeEntry>>> {
    let results: Vec<Result<Vec<TreeEntry>>> = pool.install(|| {
        sampled
            .par_iter()
            .map_init(
                || Repository::open(repo_dir).context("opening repo on worker"),
                |repo_result, (oid, _)| -> Result<Vec<TreeEntry>> {
                    let repo = repo_result.as_ref().map_err(|e| anyhow!("{e}"))?;
                    let commit = repo.find_commit(*oid)?;
                    let tree = commit.tree()?;
                    let entries = collect_blob_entries(repo, &tree, filter)?;
                    progress.inc(1);
                    Ok(entries)
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

/// Blames all changed files in a bounded commit window. The indexed parallel
/// iterator preserves task order, then results are grouped by commit so the
/// caller can apply complete commits chronologically.
#[allow(clippy::too_many_arguments)]
fn blame_commit_window(
    pool: &rayon::ThreadPool,
    repo_dir: &Path,
    plans: &[CommitPlan],
    commit2cohort: &HashMap<Oid, String>,
    ignore_whitespace: bool,
    progress: &ProgressBar,
    timing: &TimingStats,
    measure_time: bool,
) -> Result<Vec<Vec<(String, FileHistogram)>>> {
    let tasks: Vec<(usize, Oid, &TreeEntry)> = plans
        .iter()
        .enumerate()
        .flat_map(|(plan_idx, plan)| {
            plan.to_blame
                .iter()
                .map(move |entry| (plan_idx, plan.commit_oid, entry))
        })
        .collect();
    if tasks.is_empty() {
        return Ok((0..plans.len()).map(|_| Vec::new()).collect());
    }

    let results: Vec<Result<(usize, String, FileHistogram)>> = pool.install(|| {
        tasks
            .par_iter()
            .map_init(
                || -> Result<(Repository, Mailmap)> {
                    let repo = Repository::open(repo_dir).context("opening repo on worker")?;
                    let mailmap = repo.mailmap().context("loading mailmap on worker")?;
                    Ok((repo, mailmap))
                },
                |init_result, (plan_idx, commit_oid, entry)| {
                    let (repo, mailmap) = init_result.as_ref().map_err(|e| {
                        anyhow!("{e:#}")
                            .context(format!("blaming {} at commit {}", entry.path, commit_oid))
                    })?;
                    let mut opts = BlameOptions::new();
                    opts.newest_commit(*commit_oid);
                    if ignore_whitespace {
                        opts.ignore_whitespace(true);
                    }
                    let histogram = blame_one(
                        repo,
                        mailmap,
                        entry,
                        &mut opts,
                        commit2cohort,
                        timing,
                        measure_time,
                    )
                    .with_context(|| format!("blaming {} at commit {}", entry.path, commit_oid))?;
                    progress.inc(1);
                    Ok((*plan_idx, entry.path.clone(), histogram))
                },
            )
            .collect()
    });

    let mut grouped: Vec<Vec<(String, FileHistogram)>> =
        (0..plans.len()).map(|_| Vec::new()).collect();
    for result in results {
        let (plan_idx, path, histogram) = result?;
        grouped[plan_idx].push((path, histogram));
    }
    Ok(grouped)
}

fn blame_one(
    repo: &Repository,
    mailmap: &Mailmap,
    entry: &TreeEntry,
    opts: &mut BlameOptions,
    commit2cohort: &HashMap<Oid, String>,
    timing: &TimingStats,
    measure_time: bool,
) -> Result<FileHistogram> {
    // Measure time spent on blame (I/O)
    let blame_start = if measure_time {
        Some(Instant::now())
    } else {
        None
    };
    let blame = repo.blame_file(Path::new(&entry.path), Some(opts))?;
    if let Some(start) = blame_start {
        let elapsed_us = start.elapsed().as_micros() as u64;
        timing
            .blame_time_us
            .fetch_add(elapsed_us, Ordering::Relaxed);
    }

    // Measure time spent on post-blame computation
    let post_blame_start = if measure_time {
        Some(Instant::now())
    } else {
        None
    };
    let mut h: FileHistogram = HashMap::new();
    for hunk in blame.iter() {
        let lines = hunk.lines_in_hunk() as u64;
        if lines == 0 {
            continue;
        }
        let orig_oid = hunk.orig_commit_id();
        let signature = hunk.orig_signature();
        let (author_name, author_email) = mailmap_author_name_email(mailmap, &signature);

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
    if let Some(start) = post_blame_start {
        let elapsed_us = start.elapsed().as_micros() as u64;
        timing
            .post_blame_time_us
            .fetch_add(elapsed_us, Ordering::Relaxed);
    }
    if measure_time {
        timing.files_blamed.fetch_add(1, Ordering::Relaxed);
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
        Some(_) => ProgressStyle::with_template(
            "{msg:<55} [{bar:30}] {pos}/{len} ({elapsed_precise} / ETA {eta_precise})",
        ),
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

    /// Verifies `merge_results`' core behaviors on hand-built results, where
    /// the expected numbers can be checked by hand:
    /// - overlapping labels ("2020") from different repositories are summed
    ///   after each is resampled onto the shared timeline;
    /// - a label unique to one repository ("2021") is unaffected other than
    ///   being resampled (zero before that repository's first sample);
    /// - a commit SHA that (improbably) collides across repositories has
    ///   its series concatenated and re-sorted into chronological order.
    #[test]
    fn merge_results_sums_curves_and_orders_survival_by_time() {
        let t0 = Utc.timestamp_opt(1_000, 0).unwrap();
        let t1 = Utc.timestamp_opt(2_000, 0).unwrap();
        let t2 = Utc.timestamp_opt(3_000, 0).unwrap();

        let mut cohorts_a = BTreeMap::new();
        cohorts_a.insert("2020".to_string(), vec![1u64, 1u64]); // sampled at t0, t1
        let mut cohorts_b = BTreeMap::new();
        cohorts_b.insert("2020".to_string(), vec![5u64]); // sampled at t2 only
        cohorts_b.insert("2021".to_string(), vec![2u64]);

        let a = AnalyzeResult {
            timestamps: vec![t0, t1],
            cohorts: cohorts_a,
            exts: BTreeMap::new(),
            authors: BTreeMap::new(),
            dirs: BTreeMap::new(),
            domains: BTreeMap::new(),
            // Deliberately out of order, to check the post-merge sort.
            survival: BTreeMap::from([("deadbeef".to_string(), vec![(3000, 5), (1000, 9)])]),
        };
        let b = AnalyzeResult {
            timestamps: vec![t2],
            cohorts: cohorts_b,
            exts: BTreeMap::new(),
            authors: BTreeMap::new(),
            dirs: BTreeMap::new(),
            domains: BTreeMap::new(),
            // Same SHA as `a`, interleaved in time.
            survival: BTreeMap::from([("deadbeef".to_string(), vec![(2000, 7)])]),
        };

        let merged = merge_results(&[a, b]).unwrap();

        assert_eq!(merged.timestamps, vec![t0, t1, t2]);
        // "2020": a contributes [1, 1] at t0/t1, forward-filled to t2, plus
        // b's 5 at t2 (b contributes 0 before its own first sample).
        assert_eq!(merged.cohorts.get("2020"), Some(&vec![1, 1, 1 + 5]));
        // "2021": only b contributes: 0 before t2, 2 at t2.
        assert_eq!(merged.cohorts.get("2021"), Some(&vec![0, 0, 2]));

        assert_eq!(
            merged.survival.get("deadbeef"),
            Some(&vec![(1000, 9), (2000, 7), (3000, 5)])
        );
    }

    /// Regression test for parallel tree-walk discovery: runs
    /// `discover_entries` on a pool with more worker threads than there are
    /// sampled commits (so every thread picks up work) and checks the
    /// result against a plain sequential walk of the same commits.
    #[test]
    fn discover_entries_matches_sequential_across_multiple_workers() {
        use std::process::Command;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let repo_path = dir.path();

        let run = |args: &[&str], date: &str| {
            let status = Command::new("git")
                .args(args)
                .current_dir(repo_path)
                .env("GIT_AUTHOR_NAME", "Alice")
                .env("GIT_AUTHOR_EMAIL", "alice@example.com")
                .env("GIT_COMMITTER_NAME", "Alice")
                .env("GIT_COMMITTER_EMAIL", "alice@example.com")
                .env("GIT_AUTHOR_DATE", date)
                .env("GIT_COMMITTER_DATE", date)
                .status()
                .expect("git available");
            assert!(status.success(), "git {args:?} failed");
        };
        let head_oid = || -> Oid {
            let output = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(repo_path)
                .output()
                .expect("git available");
            assert!(output.status.success());
            Oid::from_str(String::from_utf8(output.stdout).unwrap().trim()).unwrap()
        };

        run(&["init", "-q", "-b", "main"], "2024-01-01T00:00:00Z");
        run(
            &["config", "commit.gpgsign", "false"],
            "2024-01-01T00:00:00Z",
        );

        // Five commits, each touching several files. Sampled against a
        // 4-worker pool this guarantees every thread picks up at least one
        // commit, so the test actually exercises cross-thread dispatch
        // rather than a pool that happens to run everything on one thread.
        const N_COMMITS: usize = 5;
        const N_FILES: usize = 4;
        let mut sampled: Vec<(Oid, i64)> = Vec::new();
        for i in 0..N_COMMITS {
            for f in 0..N_FILES {
                std::fs::write(
                    repo_path.join(format!("file{f}.txt")),
                    format!("commit {i} touches file {f}\nline two\n"),
                )
                .unwrap();
            }
            let date = format!("2024-01-{:02}T00:00:00Z", i + 1);
            run(&["add", "-A"], &date);
            run(&["commit", "-q", "-m", &format!("commit {i}")], &date);
            sampled.push((head_oid(), i as i64));
        }

        let repo = Repository::open(repo_path).unwrap();
        let filter = PathFilter::new(&[], &[], true).unwrap();
        let progress = ProgressBar::hidden();

        // Sequential baseline: walk each sampled commit's tree directly.
        let sequential: Vec<Vec<(String, Oid)>> = sampled
            .iter()
            .map(|(oid, _)| {
                let commit = repo.find_commit(*oid).unwrap();
                let tree = commit.tree().unwrap();
                collect_blob_entries(&repo, &tree, &filter)
                    .unwrap()
                    .into_iter()
                    .map(|e| (e.path, e.blob_oid))
                    .collect()
            })
            .collect();

        // Parallel version, run on a pool with more worker threads than
        // there are sampled commits, so real cross-thread dispatch happens.
        let pool = build_thread_pool(4).unwrap();
        let parallel: Vec<Vec<(String, Oid)>> =
            discover_entries(&pool, repo_path, &sampled, &filter, &progress)
                .unwrap()
                .into_iter()
                .map(|entries| entries.into_iter().map(|e| (e.path, e.blob_oid)).collect())
                .collect();

        assert_eq!(parallel, sequential);
    }
}
