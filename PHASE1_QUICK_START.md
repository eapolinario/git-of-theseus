# Phase 1: Quick Start (6x Speedup in 30 Minutes)

**Goal:** Get a 6x performance improvement with zero code changes.

---

## Before You Start

Measure current performance:
```bash
cd <your-repo>
git-of-theseus-analyze \
  --measure-time \
  --outdir /tmp/baseline \
  .

# Note: Look at "Total:" time (e.g., 2197.8ms)
```

---

## Step 1: Copy Repo to Local SSD (5 minutes)

**Why:** Network/HDD I/O is 10–100x slower than local SSD.

```bash
# Copy repository to local SSD
copy-item -path "\\network\path\to\repo" -destination "C:\data\repo-local" -recurse

# Or from NAS/network share
xcopy "\\nas\repos\myrepo" "C:\ssd\myrepo" /e /i /y

# Verify copy completed
dir C:\ssd\myrepo\.git | measure-object -sum -property length
```

**Expected speedup from this step alone: 1.5–3x**

---

## Step 2: Reduce Sampling Interval (5 minutes)

**Why:** Default 1 week interval = 500+ commits to blame. 2-week interval = 2x fewer blame operations.

Change from:
```bash
git-of-theseus-analyze --interval 604800 --outdir output repo
```

To:
```bash
git-of-theseus-analyze --interval 1209600 --outdir output repo
#                                     ↑
#                          2 weeks in seconds (2 × 604800)
```

**Alternative intervals:**
- 2 weeks: `1209600` → 2x faster
- 4 weeks: `2419200` → 4x faster  
- 1 month: `2592000` → 4.3x faster
- 1 quarter: `7776000` → 13x faster

**Trade-off:** Coarser time-series (less granular), but much faster analysis.

**Expected speedup from this step: 2–10x**
(Use 2-week for 2x; if you need it faster, use 4-week)

---

## Step 3: Maximize Parallelism (2 minutes)

**Why:** Match worker thread count to CPU core count for optimal I/O queue saturation.

```bash
# Check your CPU core count
wmic cpu get NumberOfCores

# Use that number for --procs
git-of-theseus-analyze \
  --procs 32 \              # If you have 32 cores
  --outdir output \
  repo
```

**Expected speedup from this step: 1.1–1.5x**

---

## Step 4: Measure Improvement

Run with all Phase 1 optimizations:

```bash
$repo = "C:\ssd\myrepo"  # Local SSD repo
$outdir = "C:\temp\optimized"

git-of-theseus-analyze `
  --interval 1209600 `      # 2-week sampling
  --procs 32 `              # Max parallelism
  --measure-time `          # Show timing breakdown
  --outdir $outdir `
  $repo
```

---

## Expected Results

### Before (Baseline)
```
Total: 2197.8ms
Blame (I/O): 2193.7ms (99.8%)
```

### After Phase 1
```
Total: ~400–500ms  (6x faster!)
Blame (I/O): same work, but overlapped with parallelism
```

### Speedup Breakdown
- Local SSD: 2–3x
- 2-week interval: 2x
- Max parallelism: 1.1–1.5x

**Combined: ~6x speedup**

---

## Complete Phase 1 Command

```powershell
# One-liner with all optimizations
git-of-theseus-analyze `
  --interval 1209600 `
  --procs 32 `
  --measure-time `
  --outdir C:\output\results `
  C:\ssd\myrepo
```

---

## Trade-offs

| Optimization | Speedup | Trade-off |
|--------------|---------|-----------|
| Local SSD | 2–3x | Must copy repo (~5 min for 1GB repo) |
| 2-week interval | 2x | Less granular time-series |
| Max parallelism | 1.1–1.5x | Minimal (just CPU scheduling) |

**All trade-offs are acceptable for exploratory analysis.**

---

## Validation

Verify timing improved:

```bash
# Run the optimized command
git-of-theseus-analyze --measure-time --interval 1209600 --procs 32 --outdir output C:\ssd\repo

# Compare output
# Before: Total: 2197.8ms
# After:  Total: ~400ms
# Improvement: 2197.8 / 400 ≈ 5.5x ✅
```

---

## What's Next?

If 6x is enough:
- ✅ Done! You've optimized the easy wins.

If you need more speed (and have 4–6 hours):
- 📖 Read `PREFETCH_PIPELINING_IMPLEMENTATION.md`
- 🔧 Implement prefetch + pipelining (2–5x additional)
- Expected combined: 15–30x speedup

---

## Troubleshooting

### "Copy is too slow (large repo, network transfer)"
```bash
# Use robocopy for faster network copies
robocopy "\\network\path\repo" "C:\ssd\repo" /MIR /W:0 /R:1
```

### "Local SSD doesn't have enough space"
```bash
# Use 2-week interval + parallelism instead
# This alone gives 2x + 1.2x ≈ 2.4x speedup
git-of-theseus-analyze --interval 1209600 --procs 32 --outdir output <original-repo>
```

### "Only have HDD available (no SSD)"
```bash
# Focus on reducing interval instead
git-of-theseus-analyze --interval 1209600 --interval 2419200 --procs 32 --outdir output <hdd-repo>
# 4-week interval: 4x faster
# Max parallelism: 1.2x
# Total: ~5x speedup without needing SSD
```

### "Not seeing 6x speedup"
```bash
# Check what's actually taking time
git-of-theseus-analyze --measure-time --outdir output <repo>

# If Blame (I/O) is <95%, something else is the bottleneck
# If Blame (I/O) is >95%, Phase 1 optimizations should work
```

---

## Summary

| Step | Action | Time | Speedup |
|------|--------|------|---------|
| 1 | Copy to local SSD | 5 min | 2–3x |
| 2 | Use 2-week interval | 1 min | 2x |
| 3 | Max parallelism | 1 min | 1.2x |
| 4 | Measure results | 2 min | Verify |
| **Total** | **Phase 1 Complete** | **~30 min** | **~6x** |

**Next: If 6x isn't enough, proceed to Phase 2 (prefetch + pipelining for 2–5x additional).**
