---
name: release-orchestrator
description: "Orchestrate a full GitHub release for git-of-theseus: open the version-bump PR, gate on CI, merge it, cut the tag, wait for the release workflow, and verify downstream Homebrew and WinGet publication."
---

# Release orchestrator

This skill treats release prep as a controlled workflow, not a single command. It follows the repository's documented release process in `docs/releasing.md` and the tag-driven release automation in `.github/workflows/release.yml`.

## Release flow

1. Prepare the version-bump PR
   - Prefer the repo's `release-plz` workflow (`.github/workflows/release-plz.yml`) to open or refresh the v{{ version }} release PR.
   - If needed, manually update the root workspace version in `Cargo.toml` and refresh `Cargo.lock` in a normal PR.
   - Keep the PR limited to the shared workspace version updates unless a deliberate dependency bump is required.

2. Review the PR
   - Verify the version bump matches the SemVer policy in `docs/releasing.md`.
   - Confirm the PR changes only touch the workspace version and lockfile for the shared release.
   - Check for any CLI compatibility changes that need a version bump even if Rust APIs are unchanged.

3. Gate on CI checks
   - Use GitHub PR checks as the release gate.
   - Do not merge until required checks are green.
   - If any jobs fail, fix the issues, update the PR, and wait for checks to become green again.

4. Merge the release PR
   - Merge only after the PR is reviewed and all required status checks pass.
   - Keep the repository on the default branch after merge.

5. Create the tag
   - Ensure the committed workspace version and the tag match exactly: `v<version>`.
   - Example: if the version is `0.5.0`, create and push `v0.5.0`.
   - Do not tag before the version is committed and reviewed.

6. Wait for the release workflow
   - The tag push triggers `.github/workflows/release.yml`.
   - This workflow validates that the tag matches the workspace version, builds the release binaries, generates WinGet manifests, publishes the GitHub Release, and updates the Homebrew formula.
   - Wait for all required jobs to finish and confirm the generated assets look correct.

7. Publish downstream packaging
   - Winget: generate the manifest files in your fork, validate them locally, and open a PR from that fork to `https://github.com/microsoft/winget-pkgs`.
   - Homebrew: verify that the release workflow committed the updated in-repo formula (`Formula/git-of-theseus.rb`) directly to `master`.

8. Complete the release
   - Confirm the GitHub Release assets exist.
   - Confirm the release workflow's in-repo Homebrew formula commit is on `master`.
   - Confirm the Winget PR to `microsoft/winget-pkgs` is open and ready for review.

## Operational command pattern

Use a safety-first sequence:

```bash
# 1) open or refresh the version PR
gh workflow run release-plz.yml

# 2) inspect the PR and check status
gh pr list --search "chore: prepare release v in:title"
gh pr view <pr-number> --web
gh pr checks <pr-number>

# 3) merge once green
gh pr merge <pr-number> --merge

# 4) update the branch and tag
git checkout master
git pull --ff-only
VERSION="0.5.0"
git tag -a "v${VERSION}" -m "Release v${VERSION}"
git push origin "v${VERSION}"

# 5) watch the release pipeline
gh run list --workflow release.yml --limit 20
```

## Guardrails

- Never create a tag before the version-bump PR is merged.
- Use your `winget-pkgs` fork for the PR head and `microsoft/winget-pkgs` for its destination.
- Let the tag-driven release workflow update the in-repo Homebrew formula directly on `master`.
- Never change dependency versions as part of a release PR unless the project specifically intends those changes.
- Treat CLI surface changes as part of the compatibility contract and pick the version bump accordingly.
- Do not consider the release complete until the GitHub Release is published and the downstream package updates are coordinated.

## Files to check when acting

- `docs/releasing.md`
- `docs/winget.md`
- `.github/workflows/release-plz.yml`
- `.github/workflows/release.yml`
- `release-plz.toml`
- `Cargo.toml`
- `Cargo.lock`
