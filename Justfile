# git-of-theseus Justfile
# Run `just` to list available commands.

# Default: list recipes
default:
    @just --list

# Install the project and its dependencies into a uv-managed venv
install:
    uv sync

# Analyze a git repository (REPO is required, e.g. just analyze REPO=../myrepo)
analyze REPO OUTDIR="got":
    uv run git-of-theseus-analyze {{ REPO }} --outdir {{ OUTDIR }}

# Analyze a git repository using the Rust CLI.
# All flags are forwarded to the binary; see `just analyze-rs-help` for options.
# Example:
#   just analyze-rs ../myrepo --branch main --quiet --ignore '*.lock'
analyze-rs *ARGS:
    cargo run --release -p got-cli -- {{ ARGS }}

# Show the Rust analyzer's help.
analyze-rs-help:
    cargo run --release -p got-cli -- --help

# Stack plot (Rust). Example: just stack-plot-rs got/cohorts.json cohorts.png
stack-plot-rs FILE="got/cohorts.json" OUTFILE="stack_plot.png":
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot-rs -- {{ FILE }} --outfile {{ OUTFILE }}

# Normalized stack plot (Rust)
stack-plot-rs-normalized FILE="got/cohorts.json" OUTFILE="stack_plot_normalized.png":
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot-rs -- {{ FILE }} --normalize --outfile {{ OUTFILE }}

# Line plot (Rust)
line-plot-rs FILE="got/authors.json" OUTFILE="line_plot.png":
    cargo run --release -p got-cli --bin git-of-theseus-line-plot-rs -- {{ FILE }} --outfile {{ OUTFILE }}

# Survival plot (Rust)
survival-plot-rs FILE="got/survival.json" OUTFILE="survival_plot.png":
    cargo run --release -p got-cli --bin git-of-theseus-survival-plot-rs -- {{ FILE }} --outfile {{ OUTFILE }}

# Survival plot with exponential fit (Rust)
survival-plot-rs-expfit FILE="got/survival.json" OUTFILE="survival_plot_expfit.png":
    cargo run --release -p got-cli --bin git-of-theseus-survival-plot-rs -- {{ FILE }} --exp-fit --outfile {{ OUTFILE }}

# Run the full Rust pipeline on a repo and generate all charts.
all-rs REPO OUTDIR="got-rs":
    cargo run --release -p got-cli --bin git-of-theseus-analyze-rs -- {{ REPO }} --outdir {{ OUTDIR }}
    just stack-plot-rs {{ OUTDIR }}/cohorts.json cohorts-rs.png
    just stack-plot-rs-normalized {{ OUTDIR }}/cohorts.json cohorts-rs-normalized.png
    just line-plot-rs {{ OUTDIR }}/authors.json authors-rs.png
    just survival-plot-rs {{ OUTDIR }}/survival.json survival-rs.png
    just survival-plot-rs-expfit {{ OUTDIR }}/survival.json survival-rs-expfit.png

# Run the Rust workspace test suite.
test-rs:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Stack plot from analysis output (FILE e.g. got/cohorts.json)
stack-plot FILE="got/cohorts.json" OUTFILE="stack_plot.png":
    uv run git-of-theseus-stack-plot {{ FILE }} --outfile {{ OUTFILE }}

# Normalized stack plot
stack-plot-normalized FILE="got/cohorts.json" OUTFILE="stack_plot_normalized.png":
    uv run git-of-theseus-stack-plot {{ FILE }} --normalize --outfile {{ OUTFILE }}

# Line plot from analysis output
line-plot FILE="got/authors.json" OUTFILE="line_plot.png":
    uv run git-of-theseus-line-plot {{ FILE }} --outfile {{ OUTFILE }}

# Survival plot from analysis output
survival-plot FILE="got/survival.json" OUTFILE="survival_plot.png":
    uv run git-of-theseus-survival-plot {{ FILE }} --outfile {{ OUTFILE }}

# Survival plot with exponential fit
survival-plot-expfit FILE="got/survival.json" OUTFILE="survival_plot_expfit.png":
    uv run git-of-theseus-survival-plot {{ FILE }} --exp-fit --outfile {{ OUTFILE }}

# Run the full pipeline on a repo and generate all charts
all REPO OUTDIR="got":
    just analyze {{ REPO }} {{ OUTDIR }}
    just stack-plot {{ OUTDIR }}/cohorts.json cohorts.png
    just stack-plot-normalized {{ OUTDIR }}/cohorts.json cohorts_normalized.png
    just line-plot {{ OUTDIR }}/authors.json authors.png
    just survival-plot {{ OUTDIR }}/survival.json survival.png
    just survival-plot-expfit {{ OUTDIR }}/survival.json survival_expfit.png

# Run unit tests
unit-test:
    uv run --extra dev pytest

# Run the CI test suite against the current repository
test: unit-test
    uv run git-of-theseus-analyze . --outdir got
    uv run git-of-theseus-stack-plot got/cohorts.json
    uv run git-of-theseus-stack-plot got/cohorts.json --normalize
    uv run git-of-theseus-stack-plot got/exts.json
    uv run git-of-theseus-stack-plot got/authors.json
    uv run git-of-theseus-line-plot got/authors.json
    uv run git-of-theseus-line-plot got/dirs.json
    uv run git-of-theseus-survival-plot got/survival.json --exp-fit
    uv run git-of-theseus-analyze --help
    uv run git-of-theseus-stack-plot --help
    uv run git-of-theseus-survival-plot --help
