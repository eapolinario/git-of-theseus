#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import textwrap
import time
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]


def run(cmd: list[str], *, check: bool = True, capture: bool = True) -> subprocess.CompletedProcess[str]:
    print(f"+ {' '.join(cmd)}")
    if capture:
        return subprocess.run(cmd, cwd=REPO_ROOT, text=True, capture_output=True, check=check)
    return subprocess.run(cmd, cwd=REPO_ROOT, text=True, check=check)


def ensure_tools() -> None:
    missing = []
    for tool in ("git", "gh"):
        if shutil.which(tool) is None:
            missing.append(tool)
    if missing:
        raise SystemExit(f"Missing required tools: {', '.join(missing)}")

    gh_status = run(["gh", "auth", "status"], check=False)
    if gh_status.returncode != 0:
        raise SystemExit("GitHub CLI is not authenticated. Run `gh auth login` first.")


def ensure_clean_repo() -> None:
    result = run(["git", "status", "--short", "--branch"], capture=True)
    if result.returncode != 0:
        raise SystemExit(result.stderr.strip() or "git status failed")
    if "working tree clean" not in result.stdout and "Changes not staged for commit" in result.stdout:
        status = result.stdout.strip().splitlines()
        if any(line.startswith("??") or line.startswith(" M") or line.startswith("M ") for line in status):
            raise SystemExit("Repository is not clean. Commit or stash changes before running a release.")


def open_release_pr() -> str:
    print("Opening the version-bump PR via the repo workflow.")
    run(["gh", "workflow", "run", "release-plz.yml"])
    result = run(["gh", "pr", "list", "--search", "Prepare release version", "--json", "number,title,headRefName,url"], capture=True)
    prs = __import__("json").loads(result.stdout or "[]")
    if not prs:
        raise SystemExit("No release version PR was created yet.")
    latest = sorted(prs, key=lambda pr: pr["number"])[-1]
    return str(latest["number"])


def wait_for_checks(pr_number: str, timeout_minutes: int = 60) -> None:
    print(f"Waiting for PR checks to pass on #{pr_number}...")
    deadline = time.time() + timeout_minutes * 60
    while time.time() < deadline:
        checks = run([
            "gh",
            "pr",
            "checks",
            pr_number,
            "--required",
        ], check=False)
        if checks.returncode == 0 and "failing" not in checks.stdout.lower() and "pending" not in checks.stdout.lower():
            if "no checks reported" not in checks.stdout.lower():
                return
        time.sleep(30)
    raise SystemExit(f"Timed out waiting for checks to pass on PR #{pr_number}.")


def merge_pr(pr_number: str) -> None:
    print(f"Merging PR #{pr_number}.")
    run(["gh", "pr", "merge", pr_number, "--merge", "--delete-branch"])


def tag_release(version: str) -> None:
    tag_name = f"v{version}"
    print(f"Creating tag {tag_name}.")
    run(["git", "checkout", "master"])
    run(["git", "pull", "--ff-only", "origin", "master"])
    run(["git", "tag", "-a", tag_name, "-m", f"Release {tag_name}"])
    run(["git", "push", "origin", tag_name])


def wait_for_release(tag_name: str, timeout_minutes: int = 90) -> None:
    print(f"Waiting for the GitHub Release to be published for {tag_name}...")
    deadline = time.time() + timeout_minutes * 60
    while time.time() < deadline:
        run(["gh", "release", "view", tag_name, "--json", "tagName,name,isDraft,url"], check=False)
        result = run(["gh", "release", "view", tag_name, "--json", "tagName,name,isDraft,url"], check=False)
        if result.returncode == 0:
            payload = __import__("json").loads(result.stdout or "{}")
            if payload and payload.get("tagName") == tag_name and not payload.get("isDraft", True):
                return
        time.sleep(30)
    raise SystemExit(f"Timed out waiting for the GitHub Release to publish for {tag_name}.")


def print_manual_followups() -> None:
    print(textwrap.dedent("""
    Release workflow complete. Next manual checks:
      - Verify the GitHub Release assets exist.
      - Confirm Homebrew formula update has been pushed.
      - Submit the generated WinGet YAML to microsoft/winget-pkgs.
      - Run a Windows smoke test with winget install/upgrade.
    """))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Orchestrate a git-of-theseus release")
    parser.add_argument("--version", help="Release version to tag, e.g. 0.5.0")
    parser.add_argument("--skip-pr", action="store_true", help="Skip opening the version-bump PR and assume it already exists")
    parser.add_argument("--pr-number", type=str, help="Existing PR number to merge")
    parser.add_argument("--skip-release-wait", action="store_true", help="Do not wait for the release workflow to publish")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    ensure_tools()
    ensure_clean_repo()

    pr_number = args.pr_number
    if not args.skip_pr:
        pr_number = open_release_pr()
    if not pr_number:
        raise SystemExit("No the PR number is available. Provide --pr-number or run without --skip-pr.")

    if not args.skip_pr:
        wait_for_checks(pr_number)
    merge_pr(pr_number)

    version = args.version
    if version is None:
        version = "0.0.0"
    tag_release(version)

    if not args.skip_release_wait:
        wait_for_release(f"v{version}")

    print_manual_followups()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
