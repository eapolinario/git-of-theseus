# `git-of-theseus` &mdash; in‑browser

A static page that runs the **code-by-cohort** and **code-by-author** analyses
entirely in the browser, then renders the same kind of stack plots that the CLI
produces. No server side; everything runs in your tab.

## How it works

```
┌──────────────────────────┐    ┌────────────────────────────────────┐
│ index.html  +  app.js    │    │ Pyodide (CPython compiled to WASM) │
│                          │    │                                    │
│  • isomorphic-git clones │    │  • git_of_theseus.analyze.analyze  │
│    the repo into         │    │    runs in --procs 0 (serial) mode │
│    LightningFS           │    │  • progress callback → JS UI       │
│  • walks first-parent    │ ─▶ │  • git operations are delegated to │
│    history, samples at   │    │    a JS-supplied GitBackend        │
│    the requested         │    │    (git_of_theseus.wasm.JsBackend) │
│    interval              │    │                                    │
│  • approximate blame     │    │  • returns cohorts.json /          │
│    (LCS line tracking)   │    │    authors.json data structures    │
│                          │    └─────────────────┬──────────────────┘
│  • Plotly renders the    │                      │
│    cohort + author       │ ◀────────────────────┘
│    stack charts          │
└──────────────────────────┘
```

The Python `analyze()` function is the same one used by the CLI &mdash; it has been
refactored so the git layer (`GitBackend` interface in
`git_of_theseus/git_backend.py`) and the progress reporting are pluggable. In
the browser we plug in:

* a JS git backend backed by [isomorphic-git](https://isomorphic-git.org/), and
* a JS progress callback that updates the page.

Multiprocessing is not available in WASM; pass `procs=0` (or use
`run_analysis()` from `git_of_theseus.wasm`) to take the in-process serial
blame path.

## Running locally

The page is fully static. Serve the **repository root** (not `web/`) so that
`web/app.js` can `fetch('../git_of_theseus/...')` to load the package source
into Pyodide:

```bash
# from the repo root
python -m http.server 8000
# then open http://localhost:8000/web/
```

Type a Git URL, pick a branch / interval, click **Run analysis**.

## Limitations

* **CORS.** Cloning a public Git repo from a static page requires a CORS proxy.
  This page uses `https://cors.isomorphic-git.org` &mdash; the proxy run by the
  isomorphic-git project. For production use, **host your own** proxy
  (`@isomorphic-git/cors-proxy`).
* **Blame fidelity.** This page implements a first-parent, LCS-based blame
  approximation; it does **not** match `git blame -w -C --follow` exactly,
  especially on repos with heavy renames or copies. If you need parity with
  the CLI, install the package locally:

  ```bash
  pip install 'git-of-theseus[plot]'
  git-of-theseus-analyze /path/to/repo
  git-of-theseus-stack-plot cohorts.json
  git-of-theseus-stack-plot authors.json
  ```

  A future iteration can ship `libgit2` compiled to WASM and call its
  proper blame through the same `JsBackend` interface, with no Python
  changes required.
* **Memory & runtime.** Browsers typically allow 2&ndash;4 GB per tab. Big
  repos / small intervals will run out of memory or take a very long time.
  Increase the sampling interval (`Sampling interval (days)`) to bound the
  amount of work.
* **Pyodide bundle size.** ~10&nbsp;MB compressed for the runtime; first
  load is slow.

## Files

| File | Role |
|------|------|
| `index.html` | UI (form + progress + chart containers) |
| `app.js` | Pyodide loader, isomorphic-git clone, JS `GitBackend` adapter, blame approximation, Plotly rendering |
| (no build step) | All deps come from CDNs (`esm.sh`, `cdn.jsdelivr.net`, `cdn.plot.ly`) |

## How to swap in a real `libgit2` WASM blame

1. Compile `libgit2` (or `gix`) to WASM and load it from `app.js`.
2. Replace `JsGitBackend.blame()` with a call into the WASM module. The
   shape it must return is documented in
   [`git_of_theseus/wasm.py`](../git_of_theseus/wasm.py).
3. Drop the `precomputeBlame()` step &mdash; you can call libgit2 synchronously
   per `(commit, path)` from inside the analyze loop.
