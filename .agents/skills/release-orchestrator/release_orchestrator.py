#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
SESSION_ROOT = Path(os.environ.get("COPILOT_SESSION_STATE", "/tmp"))
RELEASE_PR_PREFIX = "chore: prepare release v"


def run(cmd: list[str], *, check: bool = True, capture: bool = True, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    target = cwd or REPO_ROOT
    print(f"+ {' '.join(cmd)}")
    completed = subprocess.run(cmd, cwd=str(target), text=True, capture_output=True, check=False)
    if check and completed.returncode != 0:
        raise SystemExit(f"Command failed ({completed.returncode}): {' '.join(cmd)}\nSTDOUT:\n{completed.stdout}\nSTDERR:\n{completed.stderr}")
    if not capture:
        print(completed.stdout)
        print(completed.stderr)
    return completed


def ensure_tools() -> None:
    missing = [tool for tool in ("git", "gh") if shutil.which(tool) is None]
    if missing:
        raise SystemExit(f"Missing required tools: {', '.join(missing)}")

    gh_status = run(["gh", "auth", "status"], check=False)
    if gh_status.returncode != 0:
        raise SystemExit("GitHub CLI is not authenticated. Run `gh auth login` first.")


def ensure_clean_repo() -> None:
    result = run(["git", "status", "--short", "--branch"], capture=True)
    status = result.stdout.strip().splitlines()
    dirty = any(line for line in status if line and not line.startswith("##"))
    if dirty:
        raise SystemExit("Repository is not clean. Commit or stash changes before running a release.")


def workspace_version() -> str:
    result = run(["cargo", "metadata", "--no-deps", "--format-version", "1"])
    payload = json.loads(result.stdout)
    for package in payload.get("packages", []):
        if package.get("name") == "got-core":
            return str(package["version"])
    raise SystemExit("Could not determine the workspace version from Cargo metadata.")


def find_release_pr(prs: list[dict[str, object]], version: str | None = None) -> str | None:
    expected_title = f"{RELEASE_PR_PREFIX}{version}".lower() if version else None
    matching = [
        pr
        for pr in prs
        if (
            str(pr.get("title", "")).lower() == expected_title
            if expected_title
            else str(pr.get("title", "")).lower().startswith(RELEASE_PR_PREFIX)
        )
    ]
    if not matching:
        return None
    return str(sorted(matching, key=lambda pr: int(pr["number"]))[-1]["number"])


def open_release_pr(version: str | None = None) -> str:
    print("Opening the version-bump PR via the repo workflow.")
    run(["gh", "workflow", "run", "release-plz.yml"])
    time.sleep(10)
    result = run(["gh", "pr", "list", "--state", "open", "--json", "number,title"], capture=True)
    prs = json.loads(result.stdout or "[]")
    pr_number = find_release_pr(prs, version)
    if pr_number is None:
        raise SystemExit("No release version PR was created yet.")
    return pr_number


def wait_for_checks(pr_number: str, timeout_minutes: int = 60) -> None:
    print(f"Waiting for PR checks to pass on #{pr_number}...")
    deadline = time.time() + timeout_minutes * 60
    while time.time() < deadline:
        result = run(["gh", "pr", "checks", pr_number, "--required"], check=False)
        text = result.stdout.lower()
        if result.returncode == 0 and "failing" not in text and "pending" not in text and "no checks reported" not in text:
            return
        time.sleep(30)
    raise SystemExit(f"Timed out waiting for checks to pass on PR #{pr_number}.")


def merge_pr(pr_number: str) -> None:
    print(f"Merging PR #{pr_number}.")
    run(["gh", "pr", "merge", pr_number, "--merge", "--delete-branch"])


def maybe_update_version_pr(version: str | None) -> str:
    if version is None:
        pr_number = open_release_pr()
        wait_for_checks(pr_number)
        merge_pr(pr_number)
        run(["git", "checkout", "master"])
        run(["git", "pull", "--ff-only", "origin", "master"])
        return workspace_version()

    if not version.startswith("v"):
        version = f"v{version}"

    result = run(["gh", "pr", "list", "--state", "open", "--json", "number,title"], capture=True)
    prs = json.loads(result.stdout or "[]")
    pr_number = find_release_pr(prs, version[1:])
    if pr_number is None:
        pr_number = open_release_pr(version[1:])
    wait_for_checks(pr_number)
    merge_pr(pr_number)
    return version[1:]


def get_release_tag(version: str) -> str:
    if version.startswith("v"):
        return version
    return f"v{version}"


def tag_release(version: str) -> str:
    tag_name = get_release_tag(version)
    print(f"Creating tag {tag_name}.")
    run(["git", "checkout", "master"])
    run(["git", "pull", "--ff-only", "origin", "master"])
    run(["git", "tag", "-a", tag_name, "-m", f"Release {tag_name}"])
    run(["git", "push", "origin", tag_name])
    return tag_name


def wait_for_release(tag_name: str, timeout_minutes: int = 90) -> None:
    print(f"Waiting for the GitHub Release workflow for {tag_name} to finish successfully...")
    deadline = time.time() + timeout_minutes * 60
    while time.time() < deadline:
        result = run(
            [
                "gh",
                "run",
                "list",
                "--workflow",
                "release.yml",
                "--event",
                "push",
                "--limit",
                "100",
                "--json",
                "databaseId,headBranch,status,conclusion",
            ],
            check=False,
        )
        if result.returncode == 0:
            runs = json.loads(result.stdout or "[]")
            matching = [workflow_run for workflow_run in runs if workflow_run.get("headBranch") == tag_name]
            if matching:
                workflow_run = matching[0]
                if workflow_run.get("status") == "completed":
                    if workflow_run.get("conclusion") == "success":
                        return
                    raise SystemExit(
                        f"Release workflow run {workflow_run.get('databaseId')} finished with "
                        f"{workflow_run.get('conclusion')}."
                    )
        time.sleep(30)
    raise SystemExit(f"Timed out waiting for the release workflow to finish for {tag_name}.")


def generate_winget_manifests(version: str) -> None:
    tag = get_release_tag(version)
    temp_dir = Path(tempfile.mkdtemp(prefix="winget-pkgs-", dir=str(SESSION_ROOT)))
    print(f"Checking out your winget-pkgs fork into {temp_dir}.")
    run(["git", "clone", "https://github.com/eapolinario/winget-pkgs.git", str(temp_dir)], check=True)
    manifest_dir = temp_dir / "manifests" / "e" / "Eapolinario" / "GitOfTheseus" / version
    manifest_dir.mkdir(parents=True, exist_ok=True)

    x64_archive = f"git-of-theseus-{tag}-x86_64-pc-windows-msvc.zip"
    arm64_archive = f"git-of-theseus-{tag}-aarch64-pc-windows-msvc.zip"
    x64_url = f"https://github.com/eapolinario/git-of-theseus/releases/download/{tag}/{x64_archive}"
    arm64_url = f"https://github.com/eapolinario/git-of-theseus/releases/download/{tag}/{arm64_archive}"

    run(["gh", "release", "download", tag, "--repo", "eapolinario/git-of-theseus", "--pattern", "*x86_64-pc-windows-msvc.zip", "--dir", str(temp_dir)], check=True)
    run(["gh", "release", "download", tag, "--repo", "eapolinario/git-of-theseus", "--pattern", "*aarch64-pc-windows-msvc.zip", "--dir", str(temp_dir)], check=True)
    with (temp_dir / x64_archive).open("rb") as archive:
        x64_sha256 = hashlib.file_digest(archive, "sha256").hexdigest()
    with (temp_dir / arm64_archive).open("rb") as archive:
        arm64_sha256 = hashlib.file_digest(archive, "sha256").hexdigest()

    (manifest_dir / "Eapolinario.GitOfTheseus.yaml").write_text(
        textwrap.dedent(
            f"""\
            # yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.10.0.schema.json
            PackageIdentifier: Eapolinario.GitOfTheseus
            PackageVersion: {version}
            DefaultLocale: en-US
            ManifestType: version
            ManifestVersion: 1.10.0
            """
        )
    )
    (manifest_dir / "Eapolinario.GitOfTheseus.installer.yaml").write_text(
        textwrap.dedent(
            f"""\
            # yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.10.0.schema.json
            PackageIdentifier: Eapolinario.GitOfTheseus
            PackageVersion: {version}
            Installers:
              - Architecture: x64
                InstallerType: zip
                NestedInstallerType: portable
                InstallerUrl: {x64_url}
                InstallerSha256: {x64_sha256}
                NestedInstallerFiles:
                  - RelativeFilePath: {x64_archive.replace('.zip', '')}/git-of-theseus-analyze.exe
                    PortableCommandAlias: git-of-theseus-analyze
                  - RelativeFilePath: {x64_archive.replace('.zip', '')}/git-of-theseus-line-plot.exe
                    PortableCommandAlias: git-of-theseus-line-plot
                  - RelativeFilePath: {x64_archive.replace('.zip', '')}/git-of-theseus-stack-plot.exe
                    PortableCommandAlias: git-of-theseus-stack-plot
                  - RelativeFilePath: {x64_archive.replace('.zip', '')}/git-of-theseus-survival-plot.exe
                    PortableCommandAlias: git-of-theseus-survival-plot
              - Architecture: arm64
                InstallerType: zip
                NestedInstallerType: portable
                InstallerUrl: {arm64_url}
                InstallerSha256: {arm64_sha256}
                NestedInstallerFiles:
                  - RelativeFilePath: {arm64_archive.replace('.zip', '')}/git-of-theseus-analyze.exe
                    PortableCommandAlias: git-of-theseus-analyze
                  - RelativeFilePath: {arm64_archive.replace('.zip', '')}/git-of-theseus-line-plot.exe
                    PortableCommandAlias: git-of-theseus-line-plot
                  - RelativeFilePath: {arm64_archive.replace('.zip', '')}/git-of-theseus-stack-plot.exe
                    PortableCommandAlias: git-of-theseus-stack-plot
                  - RelativeFilePath: {arm64_archive.replace('.zip', '')}/git-of-theseus-survival-plot.exe
                    PortableCommandAlias: git-of-theseus-survival-plot
            ManifestType: installer
            ManifestVersion: 1.10.0
            """
        )
    )
    (manifest_dir / "Eapolinario.GitOfTheseus.locale.en-US.yaml").write_text(
        textwrap.dedent(
            f"""\
            # yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.10.0.schema.json
            PackageIdentifier: Eapolinario.GitOfTheseus
            PackageVersion: {version}
            PackageLocale: en-US
            Publisher: eapolinario
            PublisherUrl: https://github.com/eapolinario
            PublisherSupportUrl: https://github.com/eapolinario/git-of-theseus/issues
            PackageName: Git Of Theseus
            PackageUrl: https://github.com/eapolinario/git-of-theseus
            License: Apache-2.0
            LicenseUrl: https://github.com/eapolinario/git-of-theseus/blob/{tag}/LICENSE
            ShortDescription: Analyze how a Git repository grows over time.
            ManifestType: defaultLocale
            ManifestVersion: 1.10.0
            """
        )
    )

    if shutil.which("winget") is None:
        raise SystemExit("winget is required to validate the generated manifest locally; install it or skip the Winget step.")

    run(["winget", "validate", "--manifest", str(manifest_dir)], cwd=temp_dir)
    run(["git", "-C", str(temp_dir), "checkout", "-b", f"eapolinario-git-of-theseus-v{version}"])
    run(["git", "-C", str(temp_dir), "add", str(manifest_dir.relative_to(temp_dir))])
    run(["git", "-C", str(temp_dir), "commit", "-m", f"Add Git of Theseus {version} package manifests"])
    run(["git", "-C", str(temp_dir), "push", "-u", "origin", f"eapolinario-git-of-theseus-v{version}"])
    run([
        "gh",
        "pr",
        "create",
        "--repo",
        "microsoft/winget-pkgs",
        "--base",
        "master",
        "--head",
        f"eapolinario:eapolinario-git-of-theseus-v{version}"
        "--title",
        f"Update Git of Theseus to version {version}",
        "--body",
        "Generated from the tagged GitHub Release for Git of Theseus.",
    ])


def verify_homebrew_formula(version: str) -> None:
    run(["git", "checkout", "master"])
    run(["git", "pull", "--ff-only", "origin", "master"])
    formula_path = REPO_ROOT / "Formula" / "git-of-theseus.rb"
    if not formula_path.is_file() or f'version "{version}"' not in formula_path.read_text():
        raise SystemExit(f"The release workflow did not update the Homebrew formula to {version}.")
    print(f"Verified the Homebrew formula was updated to {version} by the release workflow.")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Orchestrate the git-of-theseus release lifecycle")
    parser.add_argument("--version", help="Explicit release version, e.g. 0.5.0")
    parser.add_argument("--skip-version-pr", action="store_true", help="Skip opening and merging the release-plz version bump PR")
    parser.add_argument("--skip-winget", action="store_true", help="Skip creating the downstream WinGet PR")
    parser.add_argument("--skip-homebrew", action="store_true", help="Skip verifying the release workflow's Homebrew formula update")
    parser.add_argument("--skip-release-wait", action="store_true", help="Do not wait for the GitHub Release to publish")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    ensure_tools()
    ensure_clean_repo()

    if not args.skip_version_pr:
        version = maybe_update_version_pr(args.version)
    else:
        version = args.version or workspace_version()

    if version.startswith("v"):
        version = version[1:]

    tag_name = tag_release(version)

    if not args.skip_release_wait:
        wait_for_release(tag_name)

    if not args.skip_winget:
        generate_winget_manifests(version)

    if not args.skip_homebrew:
        verify_homebrew_formula(version)

    print("Release orchestration complete.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
