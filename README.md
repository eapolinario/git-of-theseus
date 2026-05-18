# Git of Theseus

> *If a ship's planks are replaced one by one over time, is it still the same ship?*

Git of Theseus analyzes the evolution of a Git repository over time, answering questions like: how much of the code written in 2018 still exists today? Which authors' code has the longest survival half-life? How has the codebase grown across different file types?

Here's an example running it on this very repository — code broken down by the year it was added:

![git-of-theseus cohort stack plot](pics/got-cohorts.png)

## Installation

This is a fork of the original [git-of-theseus](https://github.com/erikbern/git-of-theseus) project. Install directly from this repository:

```shell
pip install git+https://github.com/eapolinario/git-of-theseus.git
```

Or clone and install with [uv](https://github.com/astral-sh/uv):

```shell
git clone https://github.com/eapolinario/git-of-theseus.git
cd git-of-theseus
uv sync
```

## Usage

### Step 1 — Analyze a repository

```shell
git-of-theseus-analyze <path-to-repo> --outdir <output-dir>
```

This writes several JSON files to `<output-dir>`:

| File | Contents |
|------|----------|
| `cohorts.json` | Lines of code grouped by the year they were added |
| `authors.json` | Lines of code grouped by author |
| `exts.json` | Lines of code grouped by file extension |
| `dirs.json` | Lines of code grouped by top-level directory |
| `survival.json` | Data for survival curve estimation |

Analysis can take a while on large repos. Run `git-of-theseus-analyze --help` for all options including `--interval`, `--branch`, `--ignore`, and `--only`.

#### Faster analysis with the Rust port (experimental)

A Rust reimplementation is being developed in this repository under `crates/got-core`, `crates/got-cli`, and `crates/got-plot`. It uses [libgit2](https://libgit2.org/) directly and runs significantly faster than the Python version on large histories. The full pipeline — analyze + line/stack/survival plots — is available in Rust:

| Python CLI | Rust CLI |
|---|---|
| `git-of-theseus-analyze` | `git-of-theseus-analyze-rs` |
| `git-of-theseus-line-plot` | `git-of-theseus-line-plot-rs` |
| `git-of-theseus-stack-plot` | `git-of-theseus-stack-plot-rs` |
| `git-of-theseus-survival-plot` | `git-of-theseus-survival-plot-rs` |

The Rust analyzer writes the same JSON schema as Python, so you can mix and match — e.g. analyze with Rust and plot with Python, or vice versa.

Build and run end-to-end:

```shell
cargo build --release
OUT=got-rs
./target/release/git-of-theseus-analyze-rs <path-to-repo> --outdir $OUT
./target/release/git-of-theseus-stack-plot-rs $OUT/cohorts.json --outfile cohorts.png
./target/release/git-of-theseus-line-plot-rs   $OUT/authors.json --normalize --outfile authors.png
./target/release/git-of-theseus-survival-plot-rs $OUT/survival.json --exp-fit --outfile survival.png
```

All Rust plot binaries support both PNG and SVG output (chosen by file extension) and accept the same flags as their Python counterparts (`--outfile`, `--max-n`, `--normalize`, `--exp-fit`, `--years`). `--display` is currently a no-op.

Flags on `git-of-theseus-analyze-rs` mirror `git-of-theseus-analyze`. Some Python-only features (mailmap rewriting via `git check-mailmap`, the `--opt` commit-graph flag, and interactive SIGINT pause/resume) are not yet implemented in the Rust port; the Python CLI remains the reference implementation while the migration is in progress.

##### Rust port — TODO

The Rust port is being delivered incrementally. Tracked work:

**Part 1 — got-core / got-cli scaffold (this PR)**
- [x] Cargo workspace with `got-core` library and `got-cli` (`git-of-theseus-analyze-rs`) binary
- [x] Commit walking, interval-based commit sampling, tree enumeration, `--only` / `--ignore` / default-filetype filtering
- [x] Default-filetype list snapshot generated from pygments via `scripts/gen_filetypes.py`
- [x] Parallel blame via rayon with per-thread `git2::Repository`
- [x] Fast diff that skips blame on unchanged blobs
- [x] JSON output matching `cohorts.json` / `exts.json` / `authors.json` / `dirs.json` / `domains.json` / `survival.json`, consumable by the existing Python plot scripts
- [x] Unit + end-to-end integration tests; `fmt --check`, `clippy -D warnings`, build/test in CI; CI cross-checks Rust JSON via the Python plot scripts

**Part 1.x — fill in deferred Python features**
- [ ] `mailmap` author/email rewriting (the Python `get_mailmap_author_name_email` helper)
- [ ] `--opt` flag: write `git commit-graph` for faster history walking on large repos
- [ ] Interactive SIGINT pause / process-count adjustment (the `handler` function in the Python CLI)
- [ ] Warn-and-fall-back behaviour exactly matching Python when `--branch` does not exist (currently emits a one-line warning to stderr; Python uses `warnings.warn` and special-cases detached HEAD)
- [ ] Investigate cohort-bucket distribution differences vs Python (libgit2 vs `git blame` rename detection — totals already match exactly)

**Part 2 — `got-wasm` web target**
- [ ] New `crates/got-wasm` crate exposing `analyze` to JS via `wasm-bindgen`
- [ ] `GitBackend` trait abstraction in `got-core` so WASM can plug in an `isomorphic-git`-backed implementation instead of `git2`
- [ ] Browser glue: clone via `isomorphic-git` (CORS proxy, see decision log) into an in-memory FS, then drive `got-wasm`
- [ ] Run blame on a Web Worker so the UI stays responsive
- [ ] Optional: GitHub GraphQL `blame` API as an alternate backend (no proxy, rate-limited)

**Part 3 — Rust ports of plot CLIs**
- [x] `git-of-theseus-line-plot-rs`, `git-of-theseus-stack-plot-rs`, `git-of-theseus-survival-plot-rs` binaries built on [`plotters`](https://crates.io/crates/plotters), with PNG + SVG output and exp-fit parity to scipy's Nelder-Mead (verified to 6 decimals on a real `survival.json`)
- [ ] Switch `plotters` to `default-features = false` + `ab_glyph` + a bundled font (e.g. DejaVuSans) so the Rust CLI no longer requires system `fontconfig`/`freetype` (and drop those deps from `flake.nix`)
- [ ] Wire `--display` to actually open the rendered file (e.g. via the `open` crate / `xdg-open`); currently a no-op that prints a hint
- [ ] Visual-regression snapshot tests for the rendered PNGs (golden-file diff with a small tolerance) once the rendering style stabilizes
- [ ] Investigate visual parity with matplotlib's `ggplot` style: tick density, axis label font size, legend placement
- [ ] Decide whether to keep the Python plot scripts or deprecate them once Rust parity is reached

**Part 4 — Cutover**
- [ ] Rename `git-of-theseus-analyze-rs` → `git-of-theseus-analyze` once feature parity and a release strategy are agreed
- [ ] Ship pre-built binaries (release workflow + GitHub Releases)
- [ ] Update `Dockerfile`, `flake.nix`, `Justfile`, and the existing CI matrix accordingly
- [ ] Remove the Python `analyze.py` (and possibly the rest of the Python package) after a deprecation window


### Step 2 — Generate plots

**Stack plot** (cohorts, authors, file extensions, or directories):

```shell
git-of-theseus-stack-plot <output-dir>/cohorts.json
git-of-theseus-stack-plot <output-dir>/authors.json
git-of-theseus-stack-plot <output-dir>/exts.json --outfile exts.png
```

**Survival plot** (percentage of lines still present after N years):

```shell
git-of-theseus-survival-plot <output-dir>/survival.json
git-of-theseus-survival-plot <output-dir>/survival.json --exp-fit
```

**Line plot** (normalized trends for authors or cohorts):

```shell
git-of-theseus-line-plot <output-dir>/authors.json --normalize
```

All commands accept `--help` for the full list of options.

## Sample Plots

The following plots were generated by running Git of Theseus on **its own repository**.

### Code by cohort (year added)

Lines of code broken down by the year they were first committed:

![Cohort stack plot](pics/got-cohorts.png)

### Code by file extension

The codebase is almost entirely Python, with shell scripts and Nix/TOML files added in later years:

![Extensions stack plot](pics/got-exts.png)

### Code by author

Contributions over time from each author:

![Authors stack plot](pics/got-authors.png)

### Author contributions (normalized)

The same data normalized to 100%:

![Authors normalized](pics/got-authors-normalized.png)

### Survival of a line of code

What percentage of lines written at a given point in time are still present N years later, estimated using [Kaplan-Meier](https://en.wikipedia.org/wiki/Kaplan%E2%80%93Meier_estimator):

![Survival plot](pics/got-survival.png)

With an exponential decay fit:

![Survival plot with exp fit](pics/got-survival-exp-fit.png)

## Analyzing Multiple Repositories

To compare survival curves across projects, analyze each repository separately and pass all `survival.json` files to the survival plot command:

```shell
git-of-theseus-analyze /path/to/repo-a --outdir repo-a-data
git-of-theseus-analyze /path/to/repo-b --outdir repo-b-data
git-of-theseus-survival-plot repo-a-data/survival.json repo-b-data/survival.json --exp-fit
```

## Working with Authors

If the same contributor appears under multiple names or email addresses, create a [`.mailmap`](https://git-scm.com/docs/gitmailmap) file in the root of the repository to deduplicate them.

To list unique author/email combinations:

**macOS / Linux**
```shell
git log --pretty=format:"%an %ae" | sort | uniq
```

**Windows PowerShell**
```powershell
git log --pretty=format:"%an %ae" | Sort-Object | Select-Object -Unique
```

## Troubleshooting

**`AttributeError: Unknown property labels`** — upgrade matplotlib:
```shell
pip install matplotlib --upgrade
```

## Related Projects

[Hercules](https://github.com/src-d/hercules) by [Markovtsev Vadim](https://twitter.com/tmarkhor) performs a similar analysis and claims to be 20%–6x faster. There's a good [blog post](https://web.archive.org/web/20180918135417/https://blog.sourced.tech/post/hercules.v4/) covering the complexity involved in analyzing Git history.
