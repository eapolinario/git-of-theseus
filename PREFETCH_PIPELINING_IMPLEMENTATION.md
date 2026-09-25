# Prefetch and Pipelining Implementation

The Rust analyzer pipelines blame work across a bounded group of sampled
commits. The implementation increases worker use without changing the
chronological analysis result.

## Correctness Requirement

The analysis state is cumulative:

- `cur_y` contains the current line-count histogram.
- `last_file_y` contains each file's last applied histogram.
- Modified and deleted files remove their previous contribution.
- Each output curve is a snapshot after one sampled commit.

Workers can finish in any order, but the analyzer must apply complete commits
in sampled order. Applying each file when it finishes can corrupt all later
snapshots.

## Design

The implementation has three phases:

1. `plan_commits()` walks sampled commits in chronological order. It compares
   blob IDs and records files to blame and paths whose old histograms must be
   removed.
2. `blame_commit_window()` flattens blame work from a bounded group of commit
   plans. Rayon runs these independent tasks in parallel and groups the
   results by commit.
3. The caller applies each complete group in sampled order. It removes old
   histograms, adds new histograms, and records the curve snapshot.

The window size is the configured process count. This permits cross-commit
work when one commit cannot fill the pool and limits the number of completed
histograms held before application.

## Error Handling

Each worker opens its own `git2::Repository` because `Repository` is not
`Sync`. Repository-open and blame errors include file and commit context and
return to the caller. The analyzer does not replace a failed blame with an
empty histogram.

## Performance Limits

Pipelining does not reduce total blame work. It can improve elapsed time when
several adjacent commits have too few changed files to use all workers. It can
show little improvement when one commit already has enough files to fill the
Rayon pool or when storage contention is the limit.

Do not use a fixed speedup estimate. Compare the same repository, revision,
interval, filters, and process count before and after the change.

## Verification

The end-to-end regression test runs the same history with one worker and four
workers. The history includes modified, added, and deleted files. It requires
all six JSON output files to be byte-identical.
