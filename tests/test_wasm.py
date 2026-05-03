"""Tests for the WASM/Pyodide entry point.

These don't actually require Pyodide – we drive the JsBackend adapter with a
plain Python object that mimics the JS adapter interface. This exercises the
full ``analyze.analyze()`` pipeline using a non-GitPython backend, which is
exactly what runs in the browser.
"""
from __future__ import annotations

import hashlib

from git_of_theseus.wasm import JsBackend, run_analysis


def _sha(s: str) -> str:
    return hashlib.sha1(s.encode("utf-8")).hexdigest()


class _FakeJsAdapter:
    """In-memory git-ish backend with two commits and one .py file.

    Layout::

        c1 (Alice, 2020-01-01) -> a.py: ["alpha"]
        c2 (Bob,   2021-01-01) -> a.py: ["alpha", "beta"]   (Bob authored line 2)
    """

    C1 = _sha("c1")
    C2 = _sha("c2")
    BLOB1 = _sha("alpha\n")
    BLOB2 = _sha("alpha\nbeta\n")

    COMMITS = {
        C1: {
            "hexsha": C1,
            "binsha_hex": C1,
            "committed_date": 1577836800,  # 2020-01-01
            "author_name": "Alice",
            "author_email": "alice@example.com",
        },
        C2: {
            "hexsha": C2,
            "binsha_hex": C2,
            "committed_date": 1609459200,  # 2021-01-01
            "author_name": "Bob",
            "author_email": "bob@example.com",
        },
    }

    PARENTS = {C1: [], C2: [C1]}
    BLOBS = {
        C1: [{"path": "a.py", "binsha_hex": BLOB1}],
        C2: [{"path": "a.py", "binsha_hex": BLOB2}],
    }
    BLAME = {
        # at C1, line "alpha" attributed to C1 (Alice)
        (C1, "a.py"): [{
            "num_lines": 1,
            "commit_hexsha": C1, "commit_binsha_hex": C1,
            "author_name": "Alice", "author_email": "alice@example.com",
        }],
        # at C2, line 1 -> C1 (Alice), line 2 -> C2 (Bob)
        (C2, "a.py"): [
            {
                "num_lines": 1,
                "commit_hexsha": C1, "commit_binsha_hex": C1,
                "author_name": "Alice", "author_email": "alice@example.com",
            },
            {
                "num_lines": 1,
                "commit_hexsha": C2, "commit_binsha_hex": C2,
                "author_name": "Bob", "author_email": "bob@example.com",
            },
        ],
    }

    # -- adapter API ------------------------------------------------------
    def has_mailmap(self):
        return False

    def head_commit_hexsha(self):
        return self.C2

    def resolve_branch(self, branch):
        return self.C2 if branch == "master" else None

    def active_branch_name(self):
        return "master"

    def iter_commits(self, branch_or_sha):
        # newest-first
        return [self.COMMITS[self.C2], self.COMMITS[self.C1]]

    def commit(self, hexsha):
        return self.COMMITS[hexsha]

    def parents(self, hexsha):
        return list(self.PARENTS[hexsha])

    def list_blobs(self, hexsha):
        return list(self.BLOBS[hexsha])

    def blame(self, hexsha, path, ignore_whitespace):
        return list(self.BLAME[(hexsha, path)])

    def check_mailmap(self, name, email):
        return [name, email]


def test_js_backend_runs_analyze_end_to_end():
    adapter = _FakeJsAdapter()
    res = run_analysis(adapter, branch="master")

    # cohort labels are sorted by the year-bucket key
    assert res["cohorts"]["labels"] == [
        "Code added in 2020",
        "Code added in 2021",
    ]
    # at C1: 1 line authored 2020 ; at C2: 1 line authored 2020 + 1 authored 2021
    assert res["cohorts"]["y"] == [[1, 1], [0, 1]]

    # authors: Alice + Bob, sorted alphabetically
    assert res["authors"]["labels"] == ["Alice", "Bob"]
    assert res["authors"]["y"] == [[1, 1], [0, 1]]

    # two timestamps, one per sampled commit, chronological
    assert len(res["cohorts"]["ts"]) == 2
    assert res["cohorts"]["ts"][0] < res["cohorts"]["ts"][1]


def test_progress_callback_emits_events():
    events = []
    run_analysis(_FakeJsAdapter(), branch="master", progress=events.append)
    phases = {e["phase"] for e in events}
    assert {"list_commits", "backtrack", "blame_commits"}.issubset(phases)


def test_jsbackend_check_mailmap_passthrough():
    backend = JsBackend(_FakeJsAdapter())
    assert backend.check_mailmap("X", "x@y.z") == ("X", "x@y.z")
