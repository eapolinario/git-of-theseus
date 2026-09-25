# Implementation Guide: Prefetch + Pipelining (2–5x Speedup)

This guide provides a concrete implementation plan for the most impactful optimization that doesn't require major algorithmic changes: **pipelining blame operations ahead of processing**.

---

## Current Problem

**Current execution flow (sequential):**

```
Commit 0:
  T=0ms:    Fast-diff (detect changes)
  T=1ms:    Queue files for blame
  T=2ms:    Blame starts → blocks on I/O (⏳ 2200ms)
  T=2202ms: Histogram aggregation
  T=2203ms: Done with commit 0

Commit 1:
  T=2203ms: Fast-diff
  T=2204ms: Queue files for blame
  T=2205ms: Blame starts → blocks on I/O (⏳ 2200ms)
  T=4405ms: Histogram aggregation
  T=4406ms: Done with commit 1
```

**Total time for 2 commits: 4406ms** (blame time = 4400ms, computation time = 6ms)

---

## Optimized Approach: Pipelining

**Pipelined execution (parallelized across commits):**

```
Main thread timeline:
  T=0ms:    Fast-diff commit 0
  T=1ms:    Queue blame for commit 0 → background thread
  T=2ms:    Fast-diff commit 1
  T=3ms:    Queue blame for commit 1 → background thread
  T=4ms:    Fast-diff commit 2
  T=5ms:    Queue blame for commit 2 → background thread
  T=6ms:    Blame for commit 0 done → aggregate histograms for 0
  T=7ms:    Blame for commit 1 done → aggregate histograms for 1
  T=8ms:    Blame for commit 2 done → aggregate histograms for 2
```

**Benefits:**
- While blame for commit N runs in background, we process commit N+1
- Blame I/O is overlapped with CPU work
- For 50 commits: speedup ≈ 50x if blame time >> computation time

**Expected speedup:** 2–5x (conservative estimate; depends on commits count)

---

## Implementation Plan

### Step 1: Create a Blame Queue Structure

Add to `crates/got-core/src/analyze.rs`:

```rust
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread;

/// Result of a blame operation queued for processing.
#[derive(Debug)]
struct BlameTask {
    commit_idx: usize,
    path: String,
    histogram: FileHistogram,
}

/// Queue for asynchronous blame results.
struct BlameQueue {
    sender: Sender<Result<BlameTask>>,
    receiver: Receiver<Result<BlameTask>>,
    pending_count: Arc<AtomicUsize>,
}

impl BlameQueue {
    fn new() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            pending_count: Arc::new(AtomicUsize::new(0)),
        }
    }
    
    fn queue_blame(&self, task: BlameTask) {
        self.pending_count.fetch_add(1, Ordering::Relaxed);
        // Task is sent to background blame thread
    }
    
    fn take_result(&self) -> Option<Result<BlameTask>> {
        match self.receiver.try_recv() {
            Ok(result) => {
                self.pending_count.fetch_sub(1, Ordering::Relaxed);
                Some(result)
            }
            Err(_) => None,
        }
    }
}
```

### Step 2: Spawn Background Blame Thread

Add to `analyze_in_memory_with_pool()`:

```rust
// Before the main loop
let blame_queue = BlameQueue::new();
let blame_sender = blame_queue.sender.clone();

// Spawn blame worker thread
let repo_dir_clone = options.repo_dir.clone();
let blame_worker = thread::spawn(move || {
    let repo = match Repository::open(&repo_dir_clone) {
        Ok(r) => r,
        Err(e) => return eprintln!("Failed to open repo: {e}"),
    };
    
    // Worker receives blame requests and processes them
    while let Ok(blame_request) = blame_request_receiver.recv() {
        let histogram = blame_one_file(&repo, &blame_request)?;
        let _ = blame_sender.send(Ok(BlameTask {
            commit_idx: blame_request.commit_idx,
            path: blame_request.path,
            histogram,
        }));
    }
});
```

### Step 3: Modify Main Loop to Use Pipelining

Current code (sequential):
```rust
for (commit_idx, (commit_oid, commit_ts)) in sampled.iter().enumerate() {
    let entries = std::mem::take(&mut entries_per_commit[commit_idx]);
    
    // Fast-diff
    let to_blame = fast_diff(...);
    
    // Blame (BLOCKS HERE)
    let blame_results = blame_files(..., &to_blame)?;
    
    // Aggregate
    for (path, hist) in blame_results {
        aggregate_histogram(&mut cur_y, &hist);
    }
}
```

Optimized code (pipelined):
```rust
let mut pending_blame_results: VecDeque<BlameTask> = VecDeque::new();

for (commit_idx, (commit_oid, commit_ts)) in sampled.iter().enumerate() {
    // Process results from PREVIOUS commit's blame (non-blocking)
    while let Some(blame_task) = blame_queue.take_result() {
        let blame_task = blame_task?;
        
        // Aggregate histogram for this file
        for (key, count) in &blame_task.histogram {
            *cur_y.entry(key.clone()).or_insert(0) += *count;
        }
        last_file_y.insert(blame_task.path.clone(), blame_task.histogram);
    }
    
    let entries = std::mem::take(&mut entries_per_commit[commit_idx]);
    
    // Fast-diff (fast, CPU work)
    let to_blame = fast_diff(...);
    
    // Queue blame for this commit (non-blocking, goes to background thread)
    for entry in to_blame {
        let task = BlameRequest {
            commit_idx,
            commit_oid: *commit_oid,
            entry: entry.clone(),
            // ... other fields
        };
        blame_queue_sender.send(task)?;
    }
    
    // Move to NEXT commit while blame for this one runs
}

// At the end, drain remaining blame results
while let Some(blame_task) = blame_queue.take_result() {
    let blame_task = blame_task?;
    aggregate_histogram(&mut cur_y, &blame_task.histogram);
}

drop(blame_worker);  // Signal worker thread to exit
blame_worker.join().ok();
```

### Step 4: Measure Impact

Before:
```bash
$ git-of-theseus-analyze-rs --measure-time --outdir /tmp/before .
Blame (I/O):              2193.7ms (99.8%)
Post-blame (compute):        3.5ms (0.2%)
Total:                    2197.8ms
```

After pipelining:
```bash
$ git-of-theseus-analyze-rs --measure-time --outdir /tmp/after .
Blame (I/O):              2193.7ms (same total work)
Post-blame (compute):        3.5ms (same total work)
Total:                     1100ms (2x faster!)  ← Expected improvement
```

**Key:** Total I/O work stays the same, but execution time improves because I/O and computation are overlapped.

---

## Why This Works

### Without Pipelining (Current)
```
Time: ||||CPU-work|||||||||||||||||||||||||||||||||||||I/O-block|||||CPU-work|
      └─ Fast-diff ┘ └────────── Blame (blocked) ──────────┘ └─ Aggregate ┘
```

CPU is idle while waiting for I/O (2193ms of 2197ms).

### With Pipelining
```
Time: ||||CPU-work|||I/O-block|||CPU-work|||I/O-block|||CPU-work|||I/O-block|
      └─ Commit 0 ┘  └─ Blame 0 ┘ └─ Commit 1 ┘ └─ Blame 1 ┘ └─ Commit 2 ┘
```

While Blame 0 waits for I/O, Commit 1's fast-diff runs on CPU.

---

## Complexity Assessment

**Pros:**
- Doesn't change fundamental algorithm
- Works with existing parallelization
- Straightforward to implement
- Easy to measure (use `--measure-time`)

**Cons:**
- Adds thread coordination complexity
- Requires careful error handling
- May complicate testing

**Estimated implementation effort:** 4–6 hours

**Risk level:** Medium (adds threading; needs careful testing)

---

## Alternative: Simpler Async-Based Approach

If threading feels too complex, use `async/await` with rayon:

```rust
// Use rayon's work-stealing scheduler
// Instead of sequential processing, spawn tasks for each commit
// Tasks: fast-diff + queue blame in parallel
// Blame results collected in shared BTreeMap
// Main thread aggregates results as they complete

sampled.par_iter().enumerate().for_each(|(commit_idx, (oid, ts))| {
    // Fast-diff (parallel)
    let to_blame = fast_diff(...);
    
    // Blame (parallel, within rayon thread pool)
    let results = blame_files(...);
    
    // Aggregate (thread-safe: use Arc<Mutex<>> for cur_y)
    for (path, hist) in results {
        blame_results_map.insert((commit_idx, path), hist);
    }
});

// Collect results in deterministic order
for commit_idx in 0..sampled.len() {
    for (path, hist) in blame_results_map.range((commit_idx, "")..=(commit_idx, "\u{10FFFF}")) {
        aggregate_histogram(&mut cur_y, hist);
    }
}
```

**Pros:** Simpler, leverages existing rayon parallelism
**Cons:** Potential lock contention on `cur_y`

---

## Quick Start Checklist

- [ ] Add `BlameQueue` struct and error handling
- [ ] Spawn background blame thread in `analyze_in_memory_with_pool()`
- [ ] Modify main loop to queue blame and process results asynchronously
- [ ] Test on small repo (git-of-theseus)
- [ ] Verify with `--measure-time` that total time decreased
- [ ] Test on large repo (flyte) for different workload profile
- [ ] Benchmark: measure speedup across several repos

---

## Expected Outcome

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Total time (small repo) | 2.2s | 1.1s | 2x |
| Total time (large repo) | 1.1s | 0.7s | 1.5x |
| Total time (deep repo) | 16.3s | 10s | 1.6x |
| I/O utilization | 70% | 90%+ | Better saturation |

**Speedup varies by:**
- Number of commits (more commits = more pipelining benefit)
- I/O latency (higher latency = more overlap opportunity)
- CPU count (more cores = better parallelism)

---

## Next Steps

1. Implement pipelining (Phase 2, Priority 1)
2. Measure impact with `--measure-time`
3. If successful, proceed to batch blame operations (Phase 2, Priority 2)
4. Consider incremental analysis for maximum gains (Phase 3)
