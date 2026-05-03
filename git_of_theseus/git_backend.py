# -*- coding: utf-8 -*-
#
# Copyright 2016 Erik Bernhardsson
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
# http://www.apache.org/licenses/LICENSE-2.0
"""Abstract git backend used by :mod:`git_of_theseus.analyze`.

The default backend (:class:`GitPythonBackend`) shells out to ``git`` via
GitPython, which is what the CLI has always used. The interface is intentionally
small so it can be re-implemented on top of pure-Python or Web-based git
implementations (e.g. ``isomorphic-git`` running under Pyodide, or a libgit2
WASM build) for use in the browser, where ``subprocess`` and ``multiprocessing``
are unavailable.

Only the operations needed by ``analyze.py`` are exposed:

* iterating commits on a branch
* resolving the HEAD/branch tip
* listing tracked blobs (``path``, ``binsha``) at a commit
* getting blame results at a commit/path: a list of ``(line_count, commit_sha,
  author_name, author_email)`` tuples, one per blame hunk
* mailmap canonicalisation of an author ``(name, email)`` pair
* whether a ``.mailmap`` is present at the repo root
"""
from __future__ import annotations

import datetime
import functools
import os
from abc import ABC, abstractmethod
from pathlib import Path
from typing import Iterable, List, Optional, Tuple


class CommitInfo:
    """A lightweight, picklable view of a commit.

    Backends should return objects with at least these attributes."""

    __slots__ = ("hexsha", "binsha", "committed_date", "author_name", "author_email")

    def __init__(self, hexsha: str, binsha: bytes, committed_date: int,
                 author_name: str, author_email: str):
        self.hexsha = hexsha
        self.binsha = binsha
        self.committed_date = committed_date
        self.author_name = author_name
        self.author_email = author_email


class EntryInfo:
    """A lightweight view of a tree blob entry."""

    __slots__ = ("path", "binsha")

    def __init__(self, path: str, binsha: bytes):
        self.path = path
        self.binsha = binsha


class BlameLine:
    """A single blame hunk: ``num_lines`` lines attributed to ``commit``."""

    __slots__ = ("num_lines", "commit_hexsha", "commit_binsha",
                 "author_name", "author_email")

    def __init__(self, num_lines: int, commit_hexsha: str, commit_binsha: bytes,
                 author_name: str, author_email: str):
        self.num_lines = num_lines
        self.commit_hexsha = commit_hexsha
        self.commit_binsha = commit_binsha
        self.author_name = author_name
        self.author_email = author_email


class GitBackend(ABC):
    """Abstract git backend used by :func:`git_of_theseus.analyze.analyze`."""

    @abstractmethod
    def has_mailmap(self) -> bool:
        """Whether the repo has a ``.mailmap`` at its root."""

    @abstractmethod
    def head_commit_hexsha(self) -> str:
        """SHA of HEAD."""

    @abstractmethod
    def resolve_branch(self, branch: str) -> Optional[str]:
        """Return the tip SHA of ``branch`` or ``None`` if it does not exist."""

    @abstractmethod
    def active_branch_name(self) -> Optional[str]:
        """Active branch name, or ``None`` if HEAD is detached."""

    @abstractmethod
    def iter_commits(self, branch_or_sha: str) -> Iterable[CommitInfo]:
        """Iterate over commits reachable from ``branch_or_sha`` (newest-first)."""

    @abstractmethod
    def commit(self, hexsha: str) -> CommitInfo:
        """Look up a commit by SHA."""

    @abstractmethod
    def parents(self, hexsha: str) -> List[str]:
        """Parent SHAs of a commit, in order."""

    @abstractmethod
    def list_blobs(self, hexsha: str) -> Iterable[EntryInfo]:
        """List all blob entries reachable from the tree of ``hexsha``."""

    @abstractmethod
    def blame(self, hexsha: str, path: str,
              ignore_whitespace: bool = False) -> List[BlameLine]:
        """Return blame hunks for ``path`` at commit ``hexsha``.

        Implementations should follow renames where possible (equivalent to
        ``git blame --follow``)."""

    @abstractmethod
    def check_mailmap(self, name: str, email: str) -> Tuple[str, str]:
        """Return ``(name, email)`` after applying the repo's mailmap."""

    # Optional – backends are free to override for performance.
    def write_commit_graph(self) -> None:  # pragma: no cover - opt feature
        """Generate a commit-graph file. Default: no-op."""
        return None


# ---------------------------------------------------------------------------
# Default backend: GitPython (shells out to the git binary).
# ---------------------------------------------------------------------------

class GitPythonBackend(GitBackend):
    """Backend that uses GitPython, i.e. the system ``git`` binary."""

    def __init__(self, repo_dir: str):
        import git  # local import so non-CLI use can avoid the dep
        self._git = git
        self.repo_dir = repo_dir
        self.repo = git.Repo(repo_dir)

    # -- mailmap -----------------------------------------------------------
    def has_mailmap(self) -> bool:
        return (Path(self.repo_dir) / ".mailmap").exists()

    @functools.lru_cache(maxsize=None)
    def check_mailmap(self, name: str, email: str) -> Tuple[str, str]:
        pre = f"{name} <{email}>"
        mapped: str = self.repo.git.check_mailmap(pre)
        if " <" in mapped:
            mapped_name, rest = mapped.split(" <", maxsplit=1)
            mapped_email = rest.rstrip(">")
        else:
            mapped_name = mapped
            mapped_email = email
        return mapped_name, mapped_email

    # -- ref resolution ----------------------------------------------------
    def head_commit_hexsha(self) -> str:
        return self.repo.head.commit.hexsha

    def resolve_branch(self, branch: str) -> Optional[str]:
        try:
            self.repo.git.show_ref(f"refs/heads/{branch}", verify=True)
        except self._git.exc.GitCommandError:
            return None
        return self.repo.commit(branch).hexsha

    def active_branch_name(self) -> Optional[str]:
        try:
            return self.repo.active_branch.name
        except TypeError:
            return None

    # -- commit walking ----------------------------------------------------
    def _to_info(self, commit) -> CommitInfo:
        return CommitInfo(
            hexsha=commit.hexsha,
            binsha=commit.binsha,
            committed_date=commit.committed_date,
            author_name=commit.author.name,
            author_email=commit.author.email,
        )

    def iter_commits(self, branch_or_sha: str) -> Iterable[CommitInfo]:
        for c in self.repo.iter_commits(branch_or_sha):
            yield self._to_info(c)

    def commit(self, hexsha: str) -> CommitInfo:
        return self._to_info(self.repo.commit(hexsha))

    def parents(self, hexsha: str) -> List[str]:
        return [p.hexsha for p in self.repo.commit(hexsha).parents]

    # -- tree iteration ----------------------------------------------------
    def list_blobs(self, hexsha: str) -> Iterable[EntryInfo]:
        for entry in self.repo.commit(hexsha).tree.traverse():
            if entry.type == "blob":
                yield EntryInfo(path=entry.path, binsha=entry.binsha)

    # -- blame -------------------------------------------------------------
    def blame(self, hexsha: str, path: str,
              ignore_whitespace: bool = False) -> List[BlameLine]:
        kwargs = {"w": True} if ignore_whitespace else {}
        out: List[BlameLine] = []
        try:
            for old_commit, lines in self.repo.blame(hexsha, path, **kwargs):
                out.append(BlameLine(
                    num_lines=len(lines),
                    commit_hexsha=old_commit.hexsha,
                    commit_binsha=old_commit.binsha,
                    author_name=old_commit.author.name,
                    author_email=old_commit.author.email,
                ))
        except Exception:
            # Match the historical behaviour: swallow blame errors silently.
            return []
        return out

    # -- optional ----------------------------------------------------------
    def write_commit_graph(self) -> None:
        self.repo.git.execute(["git", "commit-graph", "write", "--changed-paths"])


def utc_strftime(timestamp: int, fmt: str) -> str:
    """Format a UTC timestamp using ``fmt``.

    Centralised so all backends share the same cohort key formatting.
    """
    return datetime.datetime.utcfromtimestamp(timestamp).strftime(fmt)
