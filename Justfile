# git-of-theseus Justfile
# Run `just` to list available commands.

# Default: list recipes
default:
    @just --list

# Analyze a git repository.
# All flags are forwarded to the binary; see `just analyze-help` for options.
# Example:
#   just analyze ../myrepo --branch main --quiet --ignore '*.lock'
analyze *ARGS:
    cargo run --release -p got-cli --bin git-of-theseus-analyze -- {{ ARGS }}

# Show the analyzer's help.
analyze-help:
    cargo run --release -p got-cli --bin git-of-theseus-analyze -- --help

# Stack plot. Example: just stack-plot got/cohorts.json cohorts.png
stack-plot FILE="got/cohorts.json" OUTFILE="stack_plot.png" *ARGS:
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot -- {{ FILE }} --outfile {{ OUTFILE }} {{ ARGS }}

# Normalized stack plot
stack-plot-normalized FILE="got/cohorts.json" OUTFILE="stack_plot_normalized.png" *ARGS:
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot -- {{ FILE }} --normalize --outfile {{ OUTFILE }} {{ ARGS }}

# Line plot
line-plot FILE="got/authors.json" OUTFILE="line_plot.png" *ARGS:
    cargo run --release -p got-cli --bin git-of-theseus-line-plot -- {{ FILE }} --outfile {{ OUTFILE }} {{ ARGS }}

# Survival plot
survival-plot FILE="got/survival.json" OUTFILE="survival_plot.png":
    cargo run --release -p got-cli --bin git-of-theseus-survival-plot -- {{ FILE }} --outfile {{ OUTFILE }}

# Survival plot with exponential fit
survival-plot-expfit FILE="got/survival.json" OUTFILE="survival_plot_expfit.png":
    cargo run --release -p got-cli --bin git-of-theseus-survival-plot -- {{ FILE }} --exp-fit --outfile {{ OUTFILE }}

# Run the full pipeline on a repo and generate all charts.
all REPO OUTDIR="got":
    cargo run --release -p got-cli --bin git-of-theseus-analyze -- {{ REPO }} --outdir {{ OUTDIR }}
    just stack-plot {{ OUTDIR }}/cohorts.json cohorts.png
    just stack-plot-normalized {{ OUTDIR }}/cohorts.json cohorts_normalized.png
    just line-plot {{ OUTDIR }}/authors.json authors.png
    just survival-plot {{ OUTDIR }}/survival.json survival.png
    just survival-plot-expfit {{ OUTDIR }}/survival.json survival_expfit.png

# Run the Rust workspace unit test suite.
unit-test:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Run the CI test suite against the current repository
test: unit-test
    cargo run --release -p got-cli --bin git-of-theseus-analyze -- . --outdir got
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot -- got/cohorts.json --outfile got/cohorts.png
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot -- got/cohorts.json --normalize --outfile got/cohorts_normalized.png
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot -- got/exts.json --outfile got/exts.png
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot -- got/authors.json --outfile got/authors.png
    cargo run --release -p got-cli --bin git-of-theseus-line-plot -- got/authors.json --outfile got/authors_line.png
    cargo run --release -p got-cli --bin git-of-theseus-line-plot -- got/dirs.json --outfile got/dirs.png
    cargo run --release -p got-cli --bin git-of-theseus-survival-plot -- got/survival.json --exp-fit --outfile got/survival.png
    cargo run --release -p got-cli --bin git-of-theseus-analyze -- --help
    cargo run --release -p got-cli --bin git-of-theseus-stack-plot -- --help
    cargo run --release -p got-cli --bin git-of-theseus-survival-plot -- --help
