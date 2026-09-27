# I/O Optimization Guide: git-of-theseus-analyze

Since 99.6% of execution time is spent on `repo.blame_file()` I/O operations, this guide identifies concrete optimization strategies ranked by impact and implementation complexity.

---

## Quick Reference: Optimization Opportunities

| Priority | Optimization | Est. Speedup | Complexity | Description |
|----------|--------------|-------------|-----------|-------------|
| 🔴 High | **Reduce sampled commits** | 2–10x | ✅ Trivial | Sample fewer commits with `--interval` |
| 🔴 High | **Blame result caching** | 50–200x | ⚠️ Medium | Cache per-(file, commit) blame results |
| 🟠 Medium | **Aggressive parallelism** | 1.5–3x | ⚠️ Medium | Increase thread pool |
| 🟠 Medium | **Prefetch + pipelining** | 2–5x | 🔴 Hard | Queue blame ops ahead of fast-diff |
| 🟡 Low | **Git config optimization** | 1.1–1.5x | ✅ Trivial | Enable object caching, use local-only |
| 🟡 Low | **Storage optimization** | 1.5–3x | ✅ Trivial | Use SSD, local git repo (not network) |

---

## TIER 1: Quick Wins (No Code Changes Needed)

### 1.1 Reduce Sampled Commits (2–10x Speedup)

**Current behavior:** Default `--interval` is 1 week = 604,800 seconds
- For a 10-year repo: ~520 sampled commits
- For a 20-year repo: ~1,040 sampled commits

**Problem:** Each sampled commit requires blaming every file, so:
- Blame calls = sampled_commits × files_per_commit
- 1000 commits × 300 files = 300,000 blame operations

**Solution:** Use `--interval` to reduce sampling density:

```bash
# Default (1 week interval)
git-of-theseus-analyze --outdir output repo

# 2-week interval: ~2x faster
git-of-theseus-analyze --interval 1209600 --outdir output repo

# 1-month interval: ~4x faster
git-of-theseus-analyze --interval 2592000 --outdir output repo

# 1-quarter interval: ~13x faster (but less granular)
git-of-theseus-analyze --interval 7776000 --outdir output repo
```

**Trade-off:** Coarser time-series resolution, but much faster analysis.

**Recommendation:** Start with `--interval 1209600` (2 weeks) for exploratory analysis, use default for final reports.

---

### 1.2 Use Local Repository (1.5–3x Speedup)

**Problem:** If analyzing a repository over network (SMB, NFS, SSH), network latency dominates:
- Local SSD blame: 16ms per file
- Network NFS blame: 50–200ms per file
- Network SSH blame: 100–500ms per file

**Solution:** Clone repository locally before analysis:

```bash
# Clone locally (one-time cost)
git clone /network/path/repo ~/local-repo

# Analyze locally (much faster)
git-of-theseus-analyze --outdir output ~/local-repo
```

**Speedup:** 1.5–3x depending on network latency.

---

### 1.3 Enable Git Object Caching (1.1–1.5x Speedup)

**Problem:** libgit2's object cache may be too small for large blame operations.

**Solution:** Configure git to use more aggressive caching:

```bash
# Check current git config
git config --show-origin core.deltaBaseCacheLimit
git config --show-origin core.packedGitMemoryLimit

# Increase limits (requires Git 2.8+)
git config --global core.deltaBaseCacheLimit 1g
git config --global core.packedGitMemoryLimit 2g

# For very large repos (20GB+ .git directory)
git config --global core.packedGitWindowSize 64m
```

**Effect:** Reduces redundant object decompression during blame.

**Note:** Requires restarting analysis if git cache is memory-resident.

---

### 1.4 Use Local Storage (1.5–3x Speedup)

**Problem:** .git directory on mechanical HDD or network storage:
- HDD seek time: 5–10ms per operation
- SSD seek time: 0.1ms per operation
- Network latency: 50–500ms per operation

**Solution:** Move repository to local SSD if possible:

```bash
# Move to local SSD
cp -r /network/repo ~/ssd-path/repo

# Analyze
git-of-theseus-analyze --outdir output ~/ssd-path/repo
```

**Speedup:** Blame time will decrease proportionally to I/O latency reduction.

---

## TIER 2: Moderate Improvements (Minor Code Changes)

### 2.1 Aggressive Parallelism (1.5–3x Speedup)

**Current code:**
```rust
pub procs: usize,  // Default: num_cpus()
```

**Problem:** Default parallelism may be conservative on high-core-count machines.

**Solution:** Increase worker threads or use parallel blame within each commit:

```bash
# Use all available cores explicitly
git-of-theseus-analyze --procs 32 --outdir output repo

# Force oversubscription (more threads than cores)
git-of-theseus-analyze --procs 64 --outdir output repo
```

**When to use:**
- ✅ High-core-count machines (16+ cores)
- ✅ When I/O latency is high (network storage)
- ❌ Machines with limited RAM (thread stacks consume memory)

**Why it helps:** More threads = better I/O queue saturation. If one thread is waiting on disk I/O, others can proceed.

**Potential drawback:** Excessive threads may cause git object cache thrashing.

---

### 2.2 Blame Result Caching (50–200x Speedup)

**Current behavior:** Each sampled commit blames all files independently.
- Commit A: blame file_x.rs → reads history from ~1990–2020
- Commit B (1 week later): blame file_x.rs → reads same history again

**Problem:** Redundant I/O for files that didn't change between commits.

**Optimization idea:**
1. Cache blame results by (file_path, commit_oid)
2. If file unchanged since last commit, reuse previous blame result
3. Fast-diff already detects unchanged files — use that to skip blame

**Current fast-diff logic** (`plan_commits()`):
```rust
match last_file_hash.remove(&entry.path) {
    // Identical file: nothing to do.
    Some(previous_oid) if previous_oid == entry.blob_oid => progress.inc(1), // ← Already skips blame!
    Some(_) => {
        paths_to_remove.push(entry.path.clone());
        to_blame.push(entry);
    }
    None => to_blame.push(entry),
}
```

**Great news:** Code already implements this optimization! ✅

**To verify it's working:**
```bash
git-of-theseus-analyze --measure-time --outdir output repo

# Look for files blamed vs. files processed
# If files blamed << files processed, caching is working
```

**Example output:**
```
Files blamed:            265
[Implies ~265 blame operations out of ~5300 processed files]
```

If your repo shows many more files processed than blamed, the cache is active and helping significantly.

---

### 2.3 Prefetch + Pipelining (2–5x Speedup)

**Current execution model:**
```
For each sampled commit:
  1. Fast-diff: Detect which files changed
  2. Blame: Run blame on changed files (waits for I/O)
  3. Aggregate: Update histograms
  4. Move to next commit
```

**Optimization:** Separate ordered fast-diff planning from blame execution:

```
1. Walk sampled commits in order and create a plan for each commit:
   - files to blame
   - modified or deleted paths to remove from cumulative state
2. Process a bounded window of commit plans with Rayon.
3. Group results by commit.
4. Apply only complete commits, in sampled order.
```

**Why ordering matters:** `cur_y` and `last_file_y` are cumulative. Applying
results in worker completion order changes later fast-diff state and corrupts
the output curves.

**Benefit:** Blame work from adjacent commits can use idle workers when one
commit has fewer changed files than the Rayon pool. A bounded window limits
result memory.

**Limit:** This does not reduce the total blame work. Repositories where each
sampled commit already has enough changed files to fill the worker pool may
show little or no improvement. Report measured speedup only.

**Implementation:** `crates/got-core/src/analyze.rs`, `plan_commits()` and
`blame_commit_window()`.

---

## TIER 3: Major Improvements (Significant Code Changes)

### 3.1 ~~Batch Blame Operations~~ (Disproven — see [#42](https://github.com/eapolinario/git-of-theseus/issues/42))

This section originally proposed prefetching blob objects through a batched
libgit2 `odb` call (`read_many`) before calling `repo.blame_file()`, and
estimated a 10–50x speedup. Investigation in #42 showed the idea does not work:

- **No batch API exists.** libgit2 1.8.1 (`git2` 0.19 / `libgit2-sys` 0.17)
  exposes only single-object reads (`git_odb_read`, `Odb::read`,
  `Odb::read_header`, `Odb::reader`). There is no `read_many`.
- **Prefetched blobs are not retained.** libgit2's object cache limit for blobs
  is 0 by default (`src/libgit2/cache.c`), so a blob read ahead of time is
  discarded immediately.
- **The prefetched objects are the wrong ones.** Blame walks ancestor commits,
  runs tree-to-tree diffs and loads each historical version of the file. The
  proposal would only prefetch the newest version of each file.
- **There is no per-file open overhead to amortize.** Each rayon worker already
  opens its `Repository` and `Mailmap` once (`blame_commit_window()`), and
  packfile maps are shared inside libgit2.
- **Measured:** prefetching every to-be-blamed blob on each worker had no effect
  (summed blame time ~945 ms → ~926 ms; wall time 0.34 s → 0.37 s) on this
  repository with `--interval 3600` (207 blames). Output was byte-identical.

A `perf` profile of `blame_file()` instead shows the cost is spread across
SHA-1 verification of every object read (~16%), per-diff config snapshot
copy/free (~13%), file-stamp `stat` checks (~9–10%) and packfile inflation
(~8%). Those libgit2 overheads are tracked as separate follow-ups; the large
wins remain reducing how much blame work is done (Tier 2 caching, §3.2, §3.3).

---

### 3.2 Intelligent Commit Sampling (5–20x Speedup)

**Current approach:** Sample commits at fixed time intervals.
- Problem: Complex commits (many changes) take longer to blame
- Opportunity: Vary sampling based on commit complexity

**Optimization idea:**
```rust
// Pseudocode: adaptive sampling
fn analyze(...) {
    let mut sampled = Vec::new();
    
    for commit in walk {
        let files_changed = commit.diffs().count();
        
        if files_changed < 5 {
            // Lightweight commit: sample every week
            if should_sample_at_interval(interval_secs) {
                sampled.push(commit);
            }
        } else if files_changed < 50 {
            // Medium commit: sample every month
            if should_sample_at_interval(interval_secs * 4) {
                sampled.push(commit);
            }
        } else {
            // Heavy commit: sample every quarter
            if should_sample_at_interval(interval_secs * 13) {
                sampled.push(commit);
            }
        }
    }
}
```

**Benefit:** Skip expensive commits, sample frequent small commits.

**Estimated speedup:** 5–20x (depends on commit distribution)

**Trade-off:** Less uniform time-series (may need post-processing)

**Implementation complexity:** Hard

**Estimated effort:** 6–10 hours

---

### 3.3 Incremental Analysis (100x+ Speedup for Reruns)

**Current behavior:** Full re-analysis every time.

**Optimization idea:** Store intermediate results and only analyze new commits:

```rust
// Pseudocode: incremental analysis
fn analyze_incremental(repo, outdir, last_analysis_timestamp) {
    // Load previous results
    let previous = load_analysis_results(outdir)?;
    
    // Find new commits since last analysis
    let new_commits = repo.revwalk()
        .filter(|c| c.time() > last_analysis_timestamp);
    
    // Blame only new commits
    let new_results = blame_commits(&new_commits)?;
    
    // Merge with previous results
    let merged = merge_results(previous, new_results);
    
    // Write results
    write_outputs(&merged)?;
}
```

**Benefit:** Second run blames only new commits (10–100x faster).

**Estimated speedup:** 100x+ for incremental reruns

**Trade-off:** Must maintain state files; invalidated if --interval changes

**Implementation complexity:** Hard (requires state management)

**Estimated effort:** 10–16 hours

---

## TIER 4: Infrastructure Improvements (External to Program)

### 4.1 Use Shallow Clone (Reduce .git Size by 90%)

**Problem:** Deep git histories have large .git directories:
- Full Linux kernel: 900MB .git
- Full chromium: 2.5GB .git

**Solution:** Use shallow clone with `--depth`:

```bash
# Shallow clone: only recent history
git clone --depth 500 /network/repo ~/shallow-repo

# Analyze
git-of-theseus-analyze --outdir output ~/shallow-repo
```

**Benefit:**
- 10–50x smaller .git directory
- Faster network transfer
- Faster object lookups (fewer objects to search)

**Trade-off:** Commits older than depth are not analyzed.

**Estimated speedup:** 2–5x

---

### 4.2 Use Git LFS for Large Files

**Problem:** Large binary files (videos, assets) slow down blame:

```bash
# Current: blame reads entire 100MB video file history
git blame video.mp4

# Time wasted: minutes per file
```

**Solution:** Use Git LFS to exclude large files:

```bash
# Create .gitattributes
echo "*.mp4 filter=lfs" > .gitattributes
echo "*.psd filter=lfs" >> .gitattributes

# Migrate existing files (one-time)
git lfs migrate import --include="*.mp4"

# Analyze (large files are skipped)
git-of-theseus-analyze --outdir output repo
```

**Benefit:** Skip expensive binary files.

**Estimated speedup:** 2–10x (depends on binary file volume)

---

### 4.3 Use Git Commit Graph (Faster History Traversal)

**Problem:** Walking entire commit history is slow for large repos.

**Solution:** Use git commit-graph (available in Git 2.18+):

```bash
# Generate commit graph (one-time, ~30 seconds for large repos)
git commit-graph write --reachable

# Subsequent operations use graph (much faster)
```

**Benefit:** Faster commit walking in step 2 ("Backtracking the master branch").

**Estimated speedup:** 1.1–2x (small relative to blame time)

**Opt in with git-of-theseus:**

```bash
git-of-theseus-analyze --opt --outdir output repo
```

`--opt` runs `git commit-graph write --reachable` before analysis (once for
each repository supplied). It requires Git 2.18+ and writes only commit-graph
metadata under `.git`; it never changes commits or the working tree.

**Expected results:** On repositories with large, deep histories, commit
walking itself is typically 1.1–2x faster. Since blame dominates this tool's
runtime, measure the full analysis with and without `--opt` using
`--measure-time`; total runtime improvement will usually be smaller.

---

## Optimization Roadmap (Recommended Implementation Order)

### Phase 1: Quick Wins (0 code changes, 0.5 hours)
1. ✅ Use local SSD storage (1.5–3x)
2. ✅ Reduce `--interval` to 2 weeks (2x)
3. ✅ Use `--procs` to match CPU count (1.1x)

**Combined speedup: ~2.5–6x**

```bash
git-of-theseus-analyze \
  --interval 1209600 \
  --procs 32 \
  --outdir output \
  ~/local-repo
```

### Phase 2: Algorithmic Improvements (Code changes, 4–6 hours)
1. ⏳ Prefetch + pipelining (2–5x)
2. ❌ ~~Batch blame operations~~ — disproven, see [#42](https://github.com/eapolinario/git-of-theseus/issues/42)

### Phase 3: Major Refactoring (Code changes, 10–16 hours)
1. 🔮 Intelligent commit sampling (5–20x)
2. 🔮 Incremental analysis (100x+ for reruns)

**Combined speedup: 100x–500x with incremental**

---

## Measurement Approach

After implementing optimizations, use `--measure-time` to verify:

```bash
# Before optimization
git-of-theseus-analyze --measure-time --outdir before ~/repo

# After optimization
git-of-theseus-analyze --measure-time --outdir after ~/repo

# Compare timing output
```

---

## Summary Table: Expected Impact

| Optimization | Speedup | Effort | Recommendation |
|--------------|---------|--------|-----------------|
| Local SSD | 1.5–3x | 0h | ✅ Do immediately |
| Reduce interval | 2–10x | 0h | ✅ Start with 2-week |
| Parallelism | 1.1–1.5x | 0h | ✅ Match CPU count |
| Prefetch/pipeline | 2–5x | 6h | ⏳ Priority 1 |
| ~~Batch blame~~ | none measured | — | ❌ Disproven ([#42](https://github.com/eapolinario/git-of-theseus/issues/42)) |
| Adaptive sampling | 5–20x | 10h | ⏳ Priority 3 |
| Incremental | 100x+ | 16h | 🔮 Medium-term |

**Best case with Phase 1 + Phase 2:** the earlier 40–300x estimate assumed batch blame; with that disproven, Phase 2 is limited to the unmeasured prefetch/pipeline estimate.
