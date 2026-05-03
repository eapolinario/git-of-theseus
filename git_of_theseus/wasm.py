# -*- coding: utf-8 -*-
#
# Copyright 2016 Erik Bernhardsson
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
# http://www.apache.org/licenses/LICENSE-2.0
"""Pyodide / WASM entry point for ``git_of_theseus``.

This module is meant to be loaded inside Pyodide and called from the JS
host. It adapts a plain JS object (e.g. one that wraps ``isomorphic-git``
or a ``libgit2`` WASM build) to :class:`git_of_theseus.git_backend.GitBackend`
and then runs :func:`git_of_theseus.analyze.analyze` in serial mode.

The JS adapter object is expected to expose these methods (all synchronous –
the JS host is responsible for awaiting any async calls before passing the
adapter to Python). All return shapes are plain dicts/lists so they
translate cleanly across the Pyodide bridge.

* ``has_mailmap() -> bool``
* ``head_commit_hexsha() -> str``
* ``resolve_branch(branch: str) -> str | None``
* ``active_branch_name() -> str | None``
* ``iter_commits(branch_or_sha: str) -> list[CommitDict]``
* ``commit(hexsha: str) -> CommitDict``
* ``parents(hexsha: str) -> list[str]``
* ``list_blobs(hexsha: str) -> list[{path, binsha_hex}]``
* ``blame(hexsha, path, ignore_whitespace) -> list[BlameDict]``
* ``check_mailmap(name, email) -> [str, str]``

Where ``CommitDict`` is::

    {hexsha, binsha_hex, committed_date, author_name, author_email}

and ``BlameDict`` is::

    {num_lines, commit_hexsha, commit_binsha_hex, author_name, author_email}

``binsha_hex`` is the lowercase hex representation of a 20-byte SHA-1.
"""
from __future__ import annotations

from typing import Iterable, List, Optional, Tuple

from .analyze import analyze
from .git_backend import BlameLine, CommitInfo, EntryInfo, GitBackend


def _hex_to_bin(hexsha: str) -> bytes:
    return bytes.fromhex(hexsha)


def _to_py(value):
    """Best-effort conversion of a Pyodide JsProxy to a Python value.

    When this module is imported outside of Pyodide the import of
    ``pyodide.ffi`` will fail – we degrade to identity in that case.
    """
    try:
        from pyodide.ffi import JsProxy  # type: ignore
    except Exception:  # pragma: no cover - non-Pyodide
        return value
    if isinstance(value, JsProxy):
        return value.to_py()
    return value


class JsBackend(GitBackend):
    """Adapter from a JS object (Pyodide ``JsProxy``) to :class:`GitBackend`."""

    def __init__(self, js_adapter):
        self._js = js_adapter

    # -- helpers -----------------------------------------------------------
    def _commit_from_dict(self, d) -> CommitInfo:
        d = _to_py(d)
        return CommitInfo(
            hexsha=d["hexsha"],
            binsha=_hex_to_bin(d["binsha_hex"]),
            committed_date=int(d["committed_date"]),
            author_name=d.get("author_name") or "",
            author_email=d.get("author_email") or "",
        )

    # -- backend impl ------------------------------------------------------
    def has_mailmap(self) -> bool:
        return bool(self._js.has_mailmap())

    def head_commit_hexsha(self) -> str:
        return str(self._js.head_commit_hexsha())

    def resolve_branch(self, branch: str) -> Optional[str]:
        v = self._js.resolve_branch(branch)
        if v is None:
            return None
        return str(v)

    def active_branch_name(self) -> Optional[str]:
        v = self._js.active_branch_name()
        if v is None:
            return None
        return str(v)

    def iter_commits(self, branch_or_sha: str) -> Iterable[CommitInfo]:
        for raw in _to_py(self._js.iter_commits(branch_or_sha)):
            yield self._commit_from_dict(raw)

    def commit(self, hexsha: str) -> CommitInfo:
        return self._commit_from_dict(self._js.commit(hexsha))

    def parents(self, hexsha: str) -> List[str]:
        return [str(p) for p in _to_py(self._js.parents(hexsha))]

    def list_blobs(self, hexsha: str) -> Iterable[EntryInfo]:
        for raw in _to_py(self._js.list_blobs(hexsha)):
            yield EntryInfo(path=raw["path"], binsha=_hex_to_bin(raw["binsha_hex"]))

    def blame(self, hexsha: str, path: str,
              ignore_whitespace: bool = False) -> List[BlameLine]:
        out: List[BlameLine] = []
        try:
            raw = _to_py(self._js.blame(hexsha, path, ignore_whitespace))
        except Exception:
            return []
        if not raw:
            return []
        for hunk in raw:
            out.append(BlameLine(
                num_lines=int(hunk["num_lines"]),
                commit_hexsha=hunk["commit_hexsha"],
                commit_binsha=_hex_to_bin(hunk["commit_binsha_hex"]),
                author_name=hunk.get("author_name") or "",
                author_email=hunk.get("author_email") or "",
            ))
        return out

    def check_mailmap(self, name: str, email: str) -> Tuple[str, str]:
        mapped = _to_py(self._js.check_mailmap(name, email))
        return str(mapped[0]), str(mapped[1])


def run_analysis(js_adapter, *, branch="master", cohortfm="%Y",
                 interval=7 * 24 * 60 * 60, ignore=None, only=None,
                 all_filetypes=False, ignore_whitespace=False,
                 progress=None):
    """Run cohort/author analysis using a JS-supplied git backend.

    Returns a dict with ``cohorts``, ``authors``, ``exts``, ``dirs``,
    ``domains``, ``survival`` keys. The ``cohorts`` and ``authors`` entries
    are what the browser UI plots as stack charts.
    """
    backend = JsBackend(js_adapter)
    return analyze(
        repo_dir=None,
        backend=backend,
        branch=branch,
        cohortfm=cohortfm,
        interval=interval,
        ignore=list(ignore) if ignore else [],
        only=list(only) if only else [],
        all_filetypes=all_filetypes,
        ignore_whitespace=ignore_whitespace,
        procs=0,                # serial mode – no multiprocessing in WASM
        quiet=True,
        progress=progress,
        write_outputs=False,
    )
