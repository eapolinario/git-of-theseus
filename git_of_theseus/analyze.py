# -*- coding: utf-8 -*-
#
# Copyright 2016 Erik Bernhardsson
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
# http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import argparse
import datetime
import functools
import json
import os
import warnings
from pathlib import Path
from typing import Callable, Optional

from wcmatch import fnmatch

from .git_backend import (
    BlameLine,
    CommitInfo,
    EntryInfo,
    GitBackend,
    GitPythonBackend,
    utc_strftime,
)

# Some filetypes in Pygments are not necessarily computer code, but configuration/documentation. Let's not include those.
IGNORE_PYGMENTS_FILETYPES = [
    "*.json",
    "*.md",
    "*.ps",
    "*.eps",
    "*.txt",
    "*.xml",
    "*.xsl",
    "*.rss",
    "*.xslt",
    "*.xsd",
    "*.wsdl",
    "*.wsf",
    "*.yaml",
    "*.yml",
]


@functools.lru_cache(maxsize=1)
def _default_filetypes():
    """Lazily compute the default file-type whitelist.

    Importing pygments at module load time slows interpreter startup and is
    only needed when ``analyze()`` runs.
    """
    import pygments.lexers
    fts = set()
    for _, _, filetypes, _ in pygments.lexers.get_all_lexers():
        fts.update(filetypes)
    fts.difference_update(IGNORE_PYGMENTS_FILETYPES)
    return fts


def get_top_dir(path):
    return (
        os.path.dirname(path).split("/")[0] + "/"
    )  # Git/GitPython on Windows also returns paths with '/'s


# ---------------------------------------------------------------------------
# Progress reporting
# ---------------------------------------------------------------------------

class _NullBar:
    """Drop-in replacement for tqdm when no progress reporting is wanted."""

    def __init__(self, total=None, desc="", **_kwargs):
        self.total = total
        self.n = 0
        self.desc = desc

    def update(self, n=1):
        self.n += n

    def set_description(self, desc, *_args, **_kwargs):
        self.desc = desc

    def __iter__(self):
        return iter(())

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class _CallbackBar:
    """Progress bar that forwards updates to a user-provided callback.

    The callback receives a dict ``{"phase", "n", "total", "desc"}``.
    """

    def __init__(self, callback, phase, total=None, desc="", **_kwargs):
        self._cb = callback
        self._phase = phase
        self.total = total
        self.n = 0
        self.desc = desc
        self._emit()

    def _emit(self):
        self._cb({
            "phase": self._phase,
            "n": self.n,
            "total": self.total,
            "desc": self.desc,
        })

    def update(self, n=1):
        self.n += n
        self._emit()

    def set_description(self, desc, *_args, **_kwargs):
        self.desc = desc
        self._emit()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


def _make_bar_factory(progress: Optional[Callable], quiet: bool):
    """Return a ``bar(total=..., desc=..., phase=...)`` factory.

    Priority:
    1. user-supplied ``progress`` callback,
    2. tqdm if available and not ``quiet``,
    3. silent null bar.
    """
    if progress is not None:
        def factory(total=None, desc="", phase="", **_kwargs):
            return _CallbackBar(progress, phase=phase, total=total, desc=desc)
        return factory

    if quiet:
        def factory(total=None, desc="", phase="", **_kwargs):
            return _NullBar(total=total, desc=desc)
        return factory

    try:
        from tqdm import tqdm
    except ImportError:
        def factory(total=None, desc="", phase="", **_kwargs):
            return _NullBar(total=total, desc=desc)
        return factory

    common = {"smoothing": 0.025, "dynamic_ncols": True}

    def factory(total=None, desc="", phase="", iterable=None, **kwargs):
        opts = dict(common)
        opts.update(kwargs)
        if iterable is not None:
            return tqdm(iterable, total=total, desc=desc, **opts)
        return tqdm(total=total, desc=desc, **opts)

    return factory


# ---------------------------------------------------------------------------
# Multiprocess blame driver (used only with the GitPython backend; relies on
# repo_dir + ``git`` being available, which is not the case in WASM).
# ---------------------------------------------------------------------------

class _BlameProc:
    """Worker process that blames files."""

    def __init__(self, repo_dir, q, ret_q, run_flag, blame_kwargs,
                 commit2cohort, use_mailmap):
        import multiprocessing
        import signal

        class _Proc(multiprocessing.Process):
            def __init__(inner):
                super().__init__(daemon=True)
                inner.repo_dir = repo_dir
                inner.q = q
                inner.ret_q = ret_q
                inner.run_flag = run_flag
                inner.blame_kwargs = dict(blame_kwargs)
                inner.commit2cohort = commit2cohort
                inner.use_mailmap = use_mailmap

            def run(inner):  # pragma: no cover - subprocess
                signal.signal(signal.SIGINT, signal.SIG_IGN)
                backend = GitPythonBackend(inner.repo_dir)
                while inner.run_flag.wait():
                    entry, commit_sha = inner.q.get()
                    if not commit_sha:
                        return
                    inner.ret_q.put(
                        (entry, _file_histogram(
                            backend, entry, commit_sha,
                            inner.commit2cohort, inner.use_mailmap,
                            ignore_whitespace=inner.blame_kwargs.get("w", False),
                        ))
                    )

        self._proc = _Proc()
        self.name = self._proc.name

    def start(self):
        self._proc.start()
        self.name = self._proc.name

    def is_alive(self):
        return self._proc.is_alive()

    def join(self):
        return self._proc.join()


def _file_histogram(backend: GitBackend, path: str, commit_sha: str,
                    commit2cohort, use_mailmap, ignore_whitespace=False):
    """Build the per-file histogram of attribution counts.

    Returns a dict keyed by ``(category, value)`` tuples.
    """
    h = {}
    for hunk in backend.blame(commit_sha, path, ignore_whitespace=ignore_whitespace):
        cohort = commit2cohort.get(hunk.commit_binsha, "MISSING")
        _, ext = os.path.splitext(path)
        if use_mailmap:
            author_name, author_email = backend.check_mailmap(
                hunk.author_name, hunk.author_email,
            )
        else:
            author_name, author_email = hunk.author_name, hunk.author_email
        keys = [
            ("cohort", cohort),
            ("ext", ext),
            ("author", author_name),
            ("dir", get_top_dir(path)),
            ("domain", author_email.split("@")[-1]),
        ]
        if hunk.commit_binsha in commit2cohort:
            keys.append(("sha", hunk.commit_hexsha))
        for key in keys:
            h[key] = h.get(key, 0) + hunk.num_lines
    return h


class _BlameDriver:
    """Multi-process driver – original CLI behaviour."""

    def __init__(self, repo_dir, proc_count, last_file_y, cur_y, blame_kwargs,
                 commit2cohort, use_mailmap, quiet):
        import multiprocessing
        self.repo_dir = repo_dir
        self.proc_count = proc_count
        self.q = multiprocessing.Queue()
        self.ret_q = multiprocessing.Queue()
        self.run_flag = multiprocessing.Event()
        self.run_flag.set()
        self.last_file_y = last_file_y
        self.cur_y = cur_y
        self.blame_kwargs = blame_kwargs
        self.commit2cohort = commit2cohort
        self.use_mailmap = use_mailmap
        self.quiet = quiet
        self.proc_pool = []
        self.spawn_process(self.proc_count)

    def spawn_process(self, spawn_only=False):
        n = self.proc_count - len(self.proc_pool)
        if n == 0:
            return
        if n < 0:
            return None if spawn_only else self._despawn_process(-n)
        if not self.quiet:
            print("\n\nStarting up processes: ", end="")
        for i in range(n):
            self.proc_pool.append(
                _BlameProc(
                    self.repo_dir,
                    self.q,
                    self.ret_q,
                    self.run_flag,
                    self.blame_kwargs,
                    self.commit2cohort,
                    self.use_mailmap,
                )
            )
            self.proc_pool[-1].start()
            if not self.quiet:
                print(
                    ("" if i == 0 else ", ") + self.proc_pool[-1].name,
                    end="\n" if i == n - 1 else "",
                )

    def _despawn_process(self, n):
        for _ in range(n):
            self.q.put((None, None))
        print("\n")
        while True:
            print("\rShutting down processes: ", end="")
            killed = 0
            for idx, proc in enumerate(self.proc_pool):
                if not proc.is_alive():
                    print(
                        ("" if killed == 0 else ", ") + proc.name,
                        end="\n" if killed == n - 1 else "",
                    )
                    killed += 1
            if killed >= n:
                for proc in self.proc_pool:
                    if not proc.is_alive():
                        proc.join()
                self.proc_pool = [p for p in self.proc_pool if p.is_alive()]
                return

    def fetch(self, commit, check_entries, bar):
        self.spawn_process()
        processed = 0
        total = len(check_entries)
        for entry in check_entries:
            self.q.put((entry.path, commit.hexsha))
        while processed < total:
            path, file_y = self.ret_q.get()
            for key_tuple, locs in file_y.items():
                self.cur_y[key_tuple] = self.cur_y.get(key_tuple, 0) + locs
            self.last_file_y[path] = file_y
            processed += 1
            self.run_flag.wait()
            bar.update()
        return self.cur_y

    def pause(self):
        self.run_flag.clear()

    def resume(self):
        self.run_flag.set()


class _SerialBlameDriver:
    """Single-process blame driver. Used in WASM (no multiprocessing)."""

    def __init__(self, backend, last_file_y, cur_y, blame_kwargs,
                 commit2cohort, use_mailmap):
        self.backend = backend
        self.last_file_y = last_file_y
        self.cur_y = cur_y
        self.blame_kwargs = blame_kwargs
        self.commit2cohort = commit2cohort
        self.use_mailmap = use_mailmap
        # API compatibility with _BlameDriver:
        self.proc_pool = []

    def fetch(self, commit, check_entries, bar):
        ignore_ws = bool(self.blame_kwargs.get("w", False))
        for entry in check_entries:
            file_y = _file_histogram(
                self.backend, entry.path, commit.hexsha,
                self.commit2cohort, self.use_mailmap,
                ignore_whitespace=ignore_ws,
            )
            for key_tuple, locs in file_y.items():
                self.cur_y[key_tuple] = self.cur_y.get(key_tuple, 0) + locs
            self.last_file_y[entry.path] = file_y
            bar.update()
        return self.cur_y

    def pause(self):  # pragma: no cover - serial driver has no pausing
        pass

    def resume(self):  # pragma: no cover
        pass


# ---------------------------------------------------------------------------
# Backwards-compat re-exports (for tests / external callers).
# ---------------------------------------------------------------------------

@functools.lru_cache(maxsize=None)
def get_mailmap_author_name_email(repo, author_name, author_email):
    """Legacy helper kept for backwards compatibility with existing callers
    and tests. New code should use :meth:`GitBackend.check_mailmap`.
    """
    pre_mailmap_author_email = f"{author_name} <{author_email}>"
    mail_mapped_author_email: str = repo.git.check_mailmap(pre_mailmap_author_email)
    if " <" in mail_mapped_author_email:
        mailmap_name, rest = mail_mapped_author_email.split(" <", maxsplit=1)
        mailmap_email = rest.rstrip(">")
    else:
        mailmap_name = mail_mapped_author_email
        mailmap_email = author_email
    return mailmap_name, mailmap_email


# ---------------------------------------------------------------------------
# Main analyze() entry point.
# ---------------------------------------------------------------------------

def analyze(
    repo_dir=None,
    cohortfm="%Y",
    interval=7 * 24 * 60 * 60,
    ignore=None,
    only=None,
    outdir=".",
    branch="master",
    all_filetypes=False,
    ignore_whitespace=False,
    procs=2,
    quiet=False,
    opt=False,
    *,
    backend: Optional[GitBackend] = None,
    progress: Optional[Callable] = None,
    write_outputs: bool = True,
):
    """Run the cohort/author/etc. analysis on a repository.

    Parameters
    ----------
    repo_dir, cohortfm, interval, ignore, only, outdir, branch, all_filetypes,
    ignore_whitespace, procs, quiet, opt:
        Same meaning as on the command line. ``procs <= 0`` switches to a
        serial in-process blame driver (required for WASM/Pyodide).
    backend:
        Optional pre-built :class:`GitBackend`. When omitted, a
        :class:`GitPythonBackend` is created from ``repo_dir``.
    progress:
        Optional callback ``progress(event)`` where ``event`` is a dict with
        ``phase``, ``n``, ``total``, ``desc``. When provided, ``tqdm`` is
        bypassed entirely – useful for browser UIs.
    write_outputs:
        When ``True`` (default), write ``cohorts.json``/``authors.json``/etc.
        to ``outdir``. When ``False``, return the structured results instead
        of writing files.

    Returns
    -------
    dict
        ``{"cohorts": ..., "authors": ..., "exts": ..., "dirs": ...,
        "domains": ..., "survival": ...}`` when ``write_outputs`` is False;
        otherwise ``None`` for backwards compatibility.
    """
    ignore = list(ignore) if ignore else []
    only = list(only) if only else []

    if backend is None:
        if repo_dir is None:
            raise ValueError("Either `repo_dir` or `backend` must be provided")
        backend = GitPythonBackend(repo_dir)

    use_mailmap = backend.has_mailmap()
    blame_kwargs = {"w": True} if ignore_whitespace else {}

    bar_factory = _make_bar_factory(progress, quiet)

    if outdir and write_outputs and not os.path.exists(outdir):
        os.makedirs(outdir)

    # Resolve branch -------------------------------------------------------
    resolved_branch = backend.resolve_branch(branch)
    if resolved_branch is None:
        active = backend.active_branch_name()
        if active is None:
            active = backend.head_commit_hexsha()
            fallback_desc = "HEAD commit '{:s}'".format(active)
        else:
            fallback_desc = "default branch '{:s}'".format(active)
        warnings.warn(
            "Requested branch: '{:s}' does not exist. Falling back to {:s}".format(
                branch, fallback_desc,
            )
        )
        branch = active

    # Optional commit-graph optimisation (ignored by non-GitPython backends).
    if opt:
        if not quiet and progress is None:
            print(
                "Generating git commit-graph... If you wish, this file is "
                "deletable later at .git/objects/info"
            )
        try:
            backend.write_commit_graph()
        except Exception:
            pass

    # ------------------------------------------------------------------
    # Phase 1: enumerate all commits, build cohort + author key sets.
    # ------------------------------------------------------------------
    master_commits = []
    commit2cohort = {}
    curve_key_tuples = set()

    desc = "{:<55s}".format("Listing all commits")
    bar = bar_factory(desc=desc, phase="list_commits")
    for commit in backend.iter_commits(branch):
        cohort = utc_strftime(commit.committed_date, cohortfm)
        commit2cohort[commit.binsha] = cohort
        curve_key_tuples.add(("cohort", cohort))
        if use_mailmap:
            author_name, author_email = backend.check_mailmap(
                commit.author_name, commit.author_email,
            )
        else:
            author_name, author_email = commit.author_name, commit.author_email
        curve_key_tuples.add(("author", author_name))
        curve_key_tuples.add(("domain", author_email.split("@")[-1]))
        bar.update()

    # ------------------------------------------------------------------
    # Phase 2: backtrack along first-parent at fixed intervals.
    # ------------------------------------------------------------------
    desc = "{:<55s}".format("Backtracking the master branch")
    bar = bar_factory(desc=desc, phase="backtrack")
    head_sha = backend.resolve_branch(branch) or backend.head_commit_hexsha()
    sha = head_sha
    last_date = None
    while True:
        commit = backend.commit(sha)
        if last_date is None or commit.committed_date < last_date - interval:
            master_commits.append(commit)
            last_date = commit.committed_date
        bar.update()
        parents = backend.parents(sha)
        if not parents:
            break
        sha = parents[0]

    # ------------------------------------------------------------------
    # Phase 3: discover entries at each sampled commit.
    # ------------------------------------------------------------------
    if ignore and not only:
        only = ["**"]  # original "stupid fix"
    def_ft_str = "+({:s})".format("|".join(_default_filetypes()))
    path_match_str = "{:s}|!+({:s})".format("|".join(only), "|".join(ignore))
    path_match_zero = len(only) == 0 and len(ignore) == 0
    ok_entry_paths = {}
    all_entries = []

    def entry_path_ok(path):
        if path not in ok_entry_paths:
            ok_entry_paths[path] = (
                all_filetypes
                or fnmatch.fnmatch(
                    os.path.split(path)[-1], def_ft_str, flags=fnmatch.EXTMATCH,
                )
            ) and (
                path_match_zero
                or fnmatch.fnmatch(
                    path,
                    path_match_str,
                    flags=fnmatch.NEGATE | fnmatch.EXTMATCH | fnmatch.SPLIT,
                )
            )
        return ok_entry_paths[path]

    master_commits = master_commits[::-1]  # chronological ascending
    entries_total = 0
    desc = "{:<55s}".format("Discovering entries & caching filenames")
    cbar = bar_factory(total=len(master_commits), desc=desc, phase="discover_commits")
    ebar = bar_factory(
        desc="{:<55s}".format("Entries Discovered"),
        phase="discover_entries",
    )
    for i, commit in enumerate(master_commits):
        for entry in backend.list_blobs(commit.hexsha):
            if not entry_path_ok(entry.path):
                continue
            entries_total += 1
            _, ext = os.path.splitext(entry.path)
            curve_key_tuples.add(("ext", ext))
            curve_key_tuples.add(("dir", get_top_dir(entry.path)))
            ebar.update()
            all_entries.append(_AppendStub(i, entry))  # placeholder
        cbar.update()

    # Convert per-commit entry list back into a list-of-lists indexed by i.
    per_commit_entries = [[] for _ in master_commits]
    for stub in all_entries:
        per_commit_entries[stub.i].append(stub.entry)
    all_entries = per_commit_entries

    del ok_entry_paths

    # ------------------------------------------------------------------
    # Phase 4: blame each sampled commit.
    # ------------------------------------------------------------------
    curves = {}
    ts = []
    last_file_y = {}
    cur_y = {}

    use_serial = procs is None or procs <= 0 or repo_dir is None
    if use_serial:
        blamer = _SerialBlameDriver(
            backend, last_file_y, cur_y, blame_kwargs, commit2cohort, use_mailmap,
        )
    else:
        blamer = _BlameDriver(
            repo_dir, procs, last_file_y, cur_y, blame_kwargs,
            commit2cohort, use_mailmap, quiet,
        )

    commit_history = {}
    last_file_hash = {}

    # Allow the (multiprocess) CLI to be paused and process count changed.
    sigint_installed = False
    if not quiet and not use_serial:
        import signal as _signal

        def handler(a, b):
            try:
                blamer.pause()
                print("\n\nProcess paused")
                x = int(input(
                    "0. Exit\n1. Continue\n2. Modify process count\n"
                    "Select an option: "
                ))
                if x == 1:
                    return blamer.resume()
                elif x == 2:
                    x = int(input(
                        "\n\nCurrent Processes: {:d}\nNew Setting: ".format(
                            blamer.proc_count,
                        )
                    ))
                    if x > 0:
                        blamer.proc_count = x
                        blamer.spawn_process(spawn_only=True)
                    return blamer.resume()
                os._exit(1)
            except Exception:
                pass
            handler(None, None)

        _signal.signal(_signal.SIGINT, handler)
        sigint_installed = True

    desc = "{:<55s}".format(
        "Analyzing commit history with {:d} processes".format(
            1 if use_serial else procs,
        )
    )
    bar = bar_factory(
        total=entries_total,
        desc="{:<55s}".format("Entries Processed"),
        phase="blame_entries",
    )
    cbar = bar_factory(
        total=len(master_commits), desc=desc, phase="blame_commits",
    )
    for commit in master_commits:
        t = datetime.datetime.utcfromtimestamp(commit.committed_date)
        ts.append(t)

        # Fast diff against previous iteration.
        entries = all_entries.pop(0)
        check_entries = []
        cur_file_hash = {}
        for entry in entries:
            cur_file_hash[entry.path] = entry.binsha
            if entry.path in last_file_hash:
                if last_file_hash[entry.path] != entry.binsha:
                    for key_tuple, count in last_file_y[entry.path].items():
                        cur_y[key_tuple] -= count
                    check_entries.append(entry)
                else:
                    bar.update()
                del last_file_hash[entry.path]
            else:
                check_entries.append(entry)
        for deleted_path in last_file_hash.keys():
            for key_tuple, count in last_file_y[deleted_path].items():
                cur_y[key_tuple] -= count
        last_file_hash = cur_file_hash

        blamer.fetch(commit, check_entries, bar)
        if not use_serial:
            cbar.set_description(
                "{:<55s}".format(
                    "Analyzing commit history with {:d} processes".format(
                        len(blamer.proc_pool),
                    )
                ),
                False,
            )
        cbar.update()

        for key_tuple, count in cur_y.items():
            key_category, key = key_tuple
            if key_category == "sha":
                commit_history.setdefault(key, []).append(
                    (commit.committed_date, count),
                )

        for key_tuple in curve_key_tuples:
            curves.setdefault(key_tuple, []).append(cur_y.get(key_tuple, 0))

    if sigint_installed:
        import signal as _signal
        _signal.signal(_signal.SIGINT, _signal.default_int_handler)

    # ------------------------------------------------------------------
    # Phase 5: emit JSON outputs.
    # ------------------------------------------------------------------
    def _build(key_type, label_fmt=lambda x: x):
        key_items = sorted(k for t, k in curve_key_tuples if t == key_type)
        return {
            "y": [curves[(key_type, k)] for k in key_items],
            "ts": [t.isoformat() for t in ts],
            "labels": [label_fmt(k) for k in key_items],
        }

    results = {
        "cohorts": _build("cohort", lambda c: "Code added in %s" % c),
        "exts": _build("ext"),
        "authors": _build("author"),
        "dirs": _build("dir"),
        "domains": _build("domain"),
        "survival": commit_history,
    }

    if not write_outputs:
        return results

    def _dump(name, data):
        fn = os.path.join(outdir, name)
        if not quiet and progress is None:
            print("Writing data to %s" % fn)
        with open(fn, "w") as f:
            json.dump(data, f)

    _dump("cohorts.json", results["cohorts"])
    _dump("exts.json", results["exts"])
    _dump("authors.json", results["authors"])
    _dump("dirs.json", results["dirs"])
    _dump("domains.json", results["domains"])
    _dump("survival.json", results["survival"])
    return None


class _AppendStub:
    """Tiny holder used while flattening per-commit entries."""

    __slots__ = ("i", "entry")

    def __init__(self, i, entry):
        self.i = i
        self.entry = entry


# ---------------------------------------------------------------------------
# CLI entry point – behaviour preserved.
# ---------------------------------------------------------------------------

def analyze_cmdline():
    parser = argparse.ArgumentParser(description="Analyze git repo")
    parser.add_argument(
        "--cohortfm",
        default="%Y",
        type=str,
        help='A Python datetime format string such as "%%Y" for creating cohorts (default: %(default)s)',
    )
    parser.add_argument(
        "--interval",
        default=7 * 24 * 60 * 60,
        type=int,
        help="Min difference between commits to analyze (default: %(default)ss)",
    )
    parser.add_argument(
        "--ignore",
        default=[],
        action="append",
        help="File patterns that should be ignored (can provide multiple, will each subtract independently). Uses glob syntax and generally needs to be shell escaped. For instance, to ignore a subdirectory `foo/`, run `git-of-theseus . --ignore 'foo/**'`.",
    )
    parser.add_argument(
        "--only",
        default=[],
        action="append",
        help="File patterns that can match. Multiple can be provided. If at least one is provided, every file has to match at least one. Uses glob syntax and typically has to be shell escaped. In order to analytize a subdirectory `bar/`, run `git-of-theseus . --only 'bar/**'`",
    )
    parser.add_argument(
        "--outdir",
        default=".",
        help="Output directory to store results (default: %(default)s)",
    )
    parser.add_argument(
        "--branch",
        default="master",
        type=str,
        help="Branch to track (default: %(default)s)",
    )
    parser.add_argument(
        "--ignore-whitespace",
        default=False,
        action="store_true",
        help="Ignore whitespace changes when running git blame.",
    )
    parser.add_argument(
        "--all-filetypes",
        action="store_true",
        help="Include all files (if not set then will only analyze the default Pygments-detected filetypes)",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        help="Disable all console output (default: %(default)s)",
    )
    parser.add_argument(
        "--procs",
        default=os.cpu_count(),
        type=int,
        help="Number of processes to use. Use 0 to run a single in-process serial blame loop (required when running under WASM/Pyodide). There is a point of diminishing returns, and RAM may become an issue on large repos (default: %(default)s)",
    )
    parser.add_argument(
        "--opt",
        action="store_true",
        help="Generates git commit-graph; Improves performance at the cost of some (~80KB/kCommit) disk space (default: %(default)s)",
    )
    parser.add_argument("repo_dir")
    kwargs = vars(parser.parse_args())

    try:
        analyze(**kwargs)
    except KeyboardInterrupt:
        exit(1)


if __name__ == "__main__":
    analyze_cmdline()
