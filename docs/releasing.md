# Release versioning

This workspace uses one shared Rust version in the root `Cargo.toml`
`[workspace.package]` table. The member crates inherit it with
`version.workspace = true`, so release preparation must update the root
`Cargo.toml` and `Cargo.lock` together before any tag is created.

## Version policy

- Use SemVer for the shared workspace version.
- Before `1.0.0`, incompatible public library API changes or incompatible CLI
  behavior changes bump the minor version (`0.y.z` -> `0.(y+1).0`). Compatible
  additions and fixes bump the patch version.
- After `1.0.0`, incompatible changes bump the major version, compatible
  additions bump the minor version, and fixes bump the patch version.
- Prereleases advance by incrementing the prerelease identifier for another
  prerelease of the same base version, for example
  `0.4.0-alpha.1` -> `0.4.0-alpha.2`. If the accumulated changes require a new
  base version, bump the base version first and reset the prerelease identifier,
  for example `0.4.0-alpha.2` -> `0.5.0-alpha.1`. Remove the prerelease suffix
  when promoting that base version to a stable release.
- Treat CLI command names, flags, accepted values, output formats, exit-status
  meaning, and documented runtime behavior as part of the compatibility surface
  for choosing the shared version bump.

## Preparing the version-bump PR

Use the **Prepare release version PR** workflow to ask
[`release-plz`](https://release-plz.dev/) to open or update a reviewable PR. The
workflow runs `release-plz release-pr` only; it does not run
`release-plz release`, create tags, create GitHub Releases, or publish crates.

The repository configures release-plz in git-only mode because these crates do
not need to be published to crates.io for version calculation. It looks for
existing release tags named `v{{ version }}`; the tag-driven release workflow
must create those tags after the version-bump PR is merged. This release-plz
workflow intentionally does not create tags. It also groups `got-core`,
`got-cli`, and `got-plot` so the proposed PR preserves the shared workspace
version.

Reviewers should check that the proposed version matches the policy above and
that both of these files are updated in the same PR:

- `Cargo.toml`
- `Cargo.lock`

If release-plz chooses an unsuitable version, edit the PR before merging it. A
manual fallback is acceptable: update the root `[workspace.package]` version,
run a Cargo command such as `cargo check --workspace` to refresh only the
workspace package version entries in `Cargo.lock`, and review those changes in a
normal PR. Do not update dependency versions unless that is an intentional part
of the release PR.

Configure the workflow with a `RELEASE_PLZ_TOKEN` secret from a GitHub App or
personal access token if generated release-plz PRs should trigger the normal PR
checks. The workflow falls back to `GITHUB_TOKEN`, but PRs created with the
default token do not trigger other workflows.

## API-break guardrail

The **SemVer API checks** workflow runs
[`cargo-semver-checks`](https://github.com/obi1kenobi/cargo-semver-checks) for
the Rust library crates:

- `got-core`
- `got-plot`

On pull requests it compares against the PR base branch with
`--baseline-rev origin/<base-branch>`, so the check works before the crates are
published to any registry. The workflow can also be run manually with a custom
Git revision baseline. Manual `baseline_ref` values may be commits, tags, full
refs such as `origin/main`, or branch names. Full refs and SHA-like values are
resolved directly, tags are resolved from `refs/tags/<baseline_ref>`, and branch
names are resolved as `origin/<baseline_ref>` so they use the remote-tracking
branch instead of the workflow's checked-out branch. If those do not resolve,
the raw value is tried as a final fallback. If a manual run does not set
`baseline_ref`, it falls back to `origin/<default-branch>`.

Once the library crates are published and registry baselines are preferred, run
`cargo semver-checks --package <crate> --baseline-version <version>` locally or
adjust the workflow to use a registry version baseline.

## Limitations and review checklist

`cargo-semver-checks` is a library API guardrail. It does not calculate the next
workspace version, update manifests, inspect `got-cli`, or catch every possible
SemVer break. In particular, it does not cover:

- CLI command names, flags, accepted values, or help text
- CLI output formats, files written, or exit-status semantics
- Runtime behavior changes
- Every Rust SemVer edge case

Before approving a version-bump PR, reviewers should explicitly look for those
compatibility changes and confirm that the selected shared version accounts for
them.

## Tagging and release builds

Release builds must not rewrite versions. The version must already be committed
before tagging.

Any tag-driven release workflow should build from the tagged commit and verify
that the tag name matches the committed workspace version exactly. Because all
crates currently inherit the workspace version, this example reads `got-core` as
a representative workspace member:

```shell
test "v$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[] | select(.name == "got-core") | .version')" = "$GITHUB_REF_NAME"
```

If the tag is `v0.5.0`, the committed workspace version must be `0.5.0`.
Homebrew, WinGet, GitHub Release artifacts, and crate/package metadata should
all use that same committed version.
