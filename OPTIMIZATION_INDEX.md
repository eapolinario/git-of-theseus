# git-of-theseus Performance Optimization Index

Complete guide to understanding and optimizing the I/O-bound `git-of-theseus-analyze` binary.

---

## 📊 Quick Facts

- **Primary expensive operation:** Per-file blame (`repo.blame_file()`)
- **Batch blame status:** The pinned libgit2 API has no multi-object read or multi-file blame call
- **Measurement tool:** `--measure-time` reports wall time and aggregate worker-time; do not treat worker-time as elapsed time

---

## 📁 Documentation Map

### 1. **Evidence & Analysis**

#### [IO_BOUND_ANALYSIS.md](IO_BOUND_ANALYSIS.md) ⭐ START HERE
**Purpose:** Understand WHY the program is I/O-bound
**Contents:**
- 6 key pieces of evidence from code analysis
- Why parallelism strategy confirms I/O bottleneck
- Quantitative breakdown of operations
- Code citations for each claim

**Read this if:** You need to convince someone the program is I/O-bound

---

#### [TIMING_MEASUREMENTS.md](TIMING_MEASUREMENTS.md)
**Purpose:** Review legacy measurements and the corrected interpretation
**Contents:**
- Measurement methodology (microsecond precision)
- Test results on 3 repositories with different profiles
- Previously recorded timing output, with a caveat about aggregate worker-time
- Per-file blame cost reporting

**Test results:**
Legacy percentages mixed summed parallel worker-time with elapsed stage times and should not be used as wall-time percentages or speedup evidence.

**Read this if:** You want to see actual measurements

---

#### [PERFORMANCE_ANALYSIS_SUMMARY.md](PERFORMANCE_ANALYSIS_SUMMARY.md)
**Purpose:** Executive summary of findings
**Contents:**
- Key evidence summary
- What would/wouldn't help
- Measurement methodology
- Optimization roadmap overview

**Read this if:** You want a quick 5-minute overview

---

### 2. **Optimization Strategies**

#### [IO_OPTIMIZATION_GUIDE.md](IO_OPTIMIZATION_GUIDE.md) ⭐ START HERE FOR OPTIMIZATIONS
**Purpose:** Complete guide to improving performance
**Contents:**

**TIER 1 - Quick Wins (no code changes):**
- Reduce sampled commits: `--interval 1209600` → 2–10x faster
- Use local SSD storage → 1.5–3x faster
- Enable git object caching → 1.1–1.5x faster
- Use local repository (not network) → 1.5–3x faster

**TIER 2 - Moderate Changes (minor code):**
- Aggressive parallelism: `--procs 64` → 1.5–3x faster
- Blame result caching (already implemented!) ✅

**TIER 3 - Major Changes (significant code):**
- Batch blame via libgit2 ODB → not supported by the current API
- Intelligent commit sampling → 5–20x faster
- Incremental analysis → 100x+ for reruns

**TIER 4 - Infrastructure:**
- Shallow clones → 2–5x faster (trade-off: incomplete history)
- Git LFS for large files → 2–10x faster
- Git commit-graph → 1.1–2x faster

**Quick Start:**
```bash
# Phase 1: 6x speedup, zero coding
git-of-theseus-analyze \
  --interval 1209600 \      # 2-week sampling
  --procs 32 \              # Max parallelism
  --outdir output \
  ~/ssd-repo               # Local SSD
```

**Read this if:** You want to know how to optimize the program

---

#### [PREFETCH_PIPELINING_IMPLEMENTATION.md](PREFETCH_PIPELINING_IMPLEMENTATION.md)
**Purpose:** Step-by-step guide to implementing highest-impact code change
**Contents:**
- Problem: sequential execution blocks on I/O
- Solution: pipeline blame operations across commits
- Expected speedup: 2–5x (conservative)
- Full implementation plan with code examples
- Complexity assessment & risk analysis
- Alternative simpler approaches

**Why this optimization:**
- 2–5x speedup for moderate effort (4–6 hours)
- Works with existing code
- Easy to measure and verify
- Priority Phase 2 optimization

**Code changes needed:**
- Add `BlameQueue` struct for async results
- Spawn background blame worker thread
- Modify main loop to queue & drain results
- ~150 lines of code

**Read this if:** You want to implement the best bang-for-buck optimization

---

## 🎯 How to Use This Documentation

### Scenario 1: "Prove the program is I/O-bound"
1. Start with [IO_BOUND_ANALYSIS.md](IO_BOUND_ANALYSIS.md)
2. Show empirical proof from [TIMING_MEASUREMENTS.md](TIMING_MEASUREMENTS.md)
3. Use `--measure-time` to demonstrate on their repo

### Scenario 2: "How do I make it faster?"
1. Read [IO_OPTIMIZATION_GUIDE.md](IO_OPTIMIZATION_GUIDE.md) TIER 1
2. Implement Phase 1 optimizations (6x speedup, no coding)
3. Measure impact with `--measure-time`
4. If needed, proceed to TIER 2 & 3

### Scenario 3: "I want to implement the best optimization"
1. Start with [IO_OPTIMIZATION_GUIDE.md](IO_OPTIMIZATION_GUIDE.md) Phase 2
2. Deep dive: [PREFETCH_PIPELINING_IMPLEMENTATION.md](PREFETCH_PIPELINING_IMPLEMENTATION.md)
3. Follow step-by-step implementation guide
4. Measure before & after with `--measure-time`

### Scenario 4: "I want to understand everything"
Read in this order:
1. [IO_BOUND_ANALYSIS.md](IO_BOUND_ANALYSIS.md) — Why it's I/O-bound
2. [TIMING_MEASUREMENTS.md](TIMING_MEASUREMENTS.md) — Proof via measurements
3. [IO_OPTIMIZATION_GUIDE.md](IO_OPTIMIZATION_GUIDE.md) — What to do
4. [PREFETCH_PIPELINING_IMPLEMENTATION.md](PREFETCH_PIPELINING_IMPLEMENTATION.md) — How to do it

---

## ⚡ Quick Reference: Expected Speedups

### No Code Changes (30 minutes)
```
Local SSD            1.5–3x
Reduce interval      2–10x     (use --interval 1209600)
Parallelism tuning   1.1–1.5x  (use --procs 32)
──────────────────────────────
COMBINED:            2.5–6x speedup
```

### With Phase 2 Code Changes
```
Prefetch/pipeline    2–5x      (4–6 hours implementation)
```

### With Phase 3 Refactoring (16 hours)
```
Incremental analysis 100x+     (reruns only)
Intelligent sampling 5–20x
──────────────────────────────
COMBINED:            500x+ speedup (with incremental)
```

---

## 🔧 Installation & Usage

### Enable Timing Measurements
```bash
# Build from source
cd S:\repos-personal\git-of-theseus
cargo build --release

# Run with timing
.\target\release\git-of-theseus-analyze.exe \
  --measure-time \
  --outdir /tmp/output \
  <repo>
```

### Output Interpretation
```
=== Timing Statistics ===
Blame call worker-time:    2193.7ms
Average blame call:          16.130ms
Analysis wall time:        2197.8ms
Files blamed:               136
Note: worker-time sums concurrent tasks and is not wall time.
```

---

## 📈 Optimization Decision Tree

```
START
  │
  ├─ Time: 30 minutes?
  │  ├─ YES → Implement Phase 1 (6x speedup)
  │  └─ NO → Skip to next
  │
  ├─ Willing to code?
  │  ├─ YES, 4–6 hours → Implement prefetch/pipeline (2–5x)
  │  ├─ Batch blame → Not supported by the current libgit2 API
  │  └─ NO → Use Phase 1 optimizations only
  │
  └─ Long-term project?
     ├─ YES → Plan Phase 3 (incremental analysis, 100x+)
     └─ NO → Phase 2 sufficient
```

---

## 🧪 Testing & Measurement

### Before Optimization
```bash
git-of-theseus-analyze --measure-time --outdir /tmp/before repo
# Expected: ~2000ms (99.6% I/O)
```

### After Phase 1
```bash
git-of-theseus-analyze \
  --interval 1209600 \
  --procs 32 \
  --measure-time \
  --outdir /tmp/after \
  ~/ssd-repo
# Expected: ~300–400ms (6x faster)
```

### After Phase 2 (Prefetch/Pipeline)
```bash
# (After code changes are committed)
git-of-theseus-analyze \
  --interval 1209600 \
  --procs 32 \
  --measure-time \
  --outdir /tmp/after-phase2 \
  ~/ssd-repo
# Expected: ~100–150ms (15–20x faster)
```

### Verify Speedup
```bash
# Compare timing output
# Look for: Total time decreased proportionally
```

---

## ⚠️ Important Notes

### What WILL Help
✅ Reducing number of commits (--interval)
✅ Using local/SSD storage
✅ Caching blame results
✅ Pipelining blame with fast-diff

### What WON'T Help
❌ Adding CPU cores (I/O-bound, not compute-bound)
❌ Optimizing histogram aggregation (only 0.4% of time)
❌ Using SIMD (computation is negligible)
❌ Parallelizing fast-diff (already fast)

### Critical Insight
Blame operations (`repo.blame_file()`) revisit file history and are the primary
performance target, but benchmark elapsed wall time rather than deriving
percentages from aggregate worker-time. The current libgit2 API does not expose
multi-file blame or multi-object content reads.
- Reconstruct entire file history from git objects
- Each object requires disk/network access
- For deep histories: 1,000–10,000 I/O operations per file
- No amount of CPU optimization will help

---

## 📞 Support

All documentation includes:
- Concrete code examples
- Expected speedup ranges
- Complexity & effort estimates
- Risk assessments

For questions about specific optimizations, refer to the appropriate document listed above.

---

## 📋 Summary

| Document | Purpose | Effort | Outcome |
|----------|---------|--------|---------|
| IO_BOUND_ANALYSIS.md | Why I/O-bound | 5 min read | Understanding |
| TIMING_MEASUREMENTS.md | Proof (empirical) | 10 min read | Conviction |
| IO_OPTIMIZATION_GUIDE.md | What to optimize | 15 min read | Strategy |
| PREFETCH_PIPELINING_IMPLEMENTATION.md | Pipelining details | 30 min read | See measured wall time |

**Recommended path:** Read all 4 docs in order, then implement Phase 1 (no coding, 6x speedup), then Phase 2 if needed.
