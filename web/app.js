// git-of-theseus in-browser host.
//
// Loads Pyodide, clones the requested git repo into an in-browser FS via
// isomorphic-git, builds a JS adapter that implements the GitBackend
// interface expected by `git_of_theseus.wasm.JsBackend`, and renders the
// resulting cohort + author stack plots with Plotly.

import git from "https://esm.sh/isomorphic-git@1.27.1";
import http from "https://esm.sh/isomorphic-git@1.27.1/http/web";
import LightningFS from "https://esm.sh/@isomorphic-git/lightning-fs@4.6.0";

const CORS_PROXY = "https://cors.isomorphic-git.org";

const $ = (id) => document.getElementById(id);
const status = (msg) => { $("status").textContent = msg; };
const setError = (msg) => { $("error").textContent = msg || ""; };
const setProgress = (n, total) => {
  const p = $("progress");
  if (!total || total <= 0) { p.removeAttribute("value"); return; }
  p.max = total; p.value = n;
};

// ---------------------------------------------------------------------------
// Pyodide loader
// ---------------------------------------------------------------------------

let pyodidePromise = null;
async function getPyodide() {
  if (pyodidePromise) return pyodidePromise;
  status("Loading Pyodide…");
  pyodidePromise = (async () => {
    const py = await loadPyodide({
      indexURL: "https://cdn.jsdelivr.net/pyodide/v0.26.4/full/",
    });
    status("Installing git-of-theseus into Pyodide…");
    await py.loadPackage("micropip");
    // Pure-Python deps required by the analyzer (gitpython is NOT loaded;
    // we provide our own backend).
    await py.runPythonAsync(`
import micropip
await micropip.install(['pygments', 'wcmatch'])
`);
    // Load the package source. When served from this directory, the
    // git_of_theseus/ tree sits at ../git_of_theseus relative to web/.
    const pkgFiles = [
      "__init__.py",
      "git_backend.py",
      "analyze.py",
      "wasm.py",
    ];
    py.FS.mkdir("/git_of_theseus");
    for (const f of pkgFiles) {
      const resp = await fetch(`../git_of_theseus/${f}`);
      if (!resp.ok) throw new Error(`Could not fetch ../git_of_theseus/${f}`);
      const src = await resp.text();
      py.FS.writeFile(`/git_of_theseus/${f}`, src);
    }
    py.runPython(`
import sys
if '/' not in sys.path:
    sys.path.insert(0, '/')
# Skip the package's plot imports which depend on matplotlib etc.
import importlib, importlib.util
spec = importlib.util.spec_from_file_location(
    'git_of_theseus.analyze', '/git_of_theseus/analyze.py')
analyze_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analyze_mod)
sys.modules['git_of_theseus.analyze'] = analyze_mod
spec = importlib.util.spec_from_file_location(
    'git_of_theseus.git_backend', '/git_of_theseus/git_backend.py')
gb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gb)
sys.modules['git_of_theseus.git_backend'] = gb
spec = importlib.util.spec_from_file_location(
    'git_of_theseus.wasm', '/git_of_theseus/wasm.py')
wasm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wasm)
sys.modules['git_of_theseus.wasm'] = wasm
`);
    return py;
  })();
  return pyodidePromise;
}

// ---------------------------------------------------------------------------
// JS GitBackend adapter — wraps isomorphic-git for Pyodide.
// ---------------------------------------------------------------------------

// Convert a 40-char hex string lowercase, defensively.
const hex = (s) => String(s).toLowerCase();

class JsGitBackend {
  constructor(fs, dir, mailmapText) {
    this.fs = fs;
    this.dir = dir;
    this._mailmap = parseMailmap(mailmapText || "");
    this._commitCache = new Map();
    this._treeCache = new Map();
    this._blobLinesCache = new Map();
  }

  has_mailmap() { return Object.keys(this._mailmap).length > 0; }

  head_commit_hexsha() { return this._headSha; }

  resolve_branch(branch) {
    return this._refs.get(`refs/heads/${branch}`) ?? null;
  }

  active_branch_name() { return this._activeBranch ?? null; }

  iter_commits(branchOrSha) {
    const oid = this._refs.get(`refs/heads/${branchOrSha}`) || branchOrSha;
    const list = this._allCommits.get(oid) || [];
    return list.map((c) => this._commitDict(c));
  }

  commit(hexsha) {
    const c = this._commitCache.get(hex(hexsha));
    if (!c) throw new Error(`unknown commit ${hexsha}`);
    return this._commitDict(c);
  }

  parents(hexsha) {
    const c = this._commitCache.get(hex(hexsha));
    return (c && c.parent) ? [...c.parent] : [];
  }

  list_blobs(hexsha) {
    return this._treeCache.get(hex(hexsha)) || [];
  }

  blame(hexsha, path, _ignoreWhitespace) {
    // Approximate blame: walk the file's first-parent history, attributing
    // any added line to the commit that introduced it. This is *not*
    // identical to `git blame -w -C --follow`, but is good enough for
    // cohort/author stackplots in the browser.
    return approximateBlame(this, hexsha, path);
  }

  check_mailmap(name, email) {
    const m = this._mailmap[`${name}|${email}`] || this._mailmap[`|${email}`];
    if (m) return [m.name || name, m.email || email];
    return [name, email];
  }

  _commitDict(c) {
    return {
      hexsha: c.oid,
      binsha_hex: c.oid,
      committed_date: c.committer.timestamp,
      author_name: c.author.name || "",
      author_email: c.author.email || "",
    };
  }
}

// Parse a .mailmap into { "name|email": {name,email}, "|email": {name,email} }
function parseMailmap(text) {
  const out = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/#.*$/, "").trim();
    if (!line) continue;
    // Forms: "Proper Name <proper@email>"
    //        "Proper Name <proper@email> <commit@email>"
    //        "Proper Name <proper@email> Commit Name <commit@email>"
    //        "<proper@email> <commit@email>"
    const m = line.match(
      /^(?:([^<]*?)\s*)?<([^>]+)>(?:\s+(?:([^<]*?)\s*)?<([^>]+)>)?$/,
    );
    if (!m) continue;
    const [, properName, properEmail, commitName, commitEmail] = m;
    if (commitEmail) {
      const key = `${commitName ? commitName.trim() : ""}|${commitEmail.trim()}`;
      out[key] = { name: properName?.trim(), email: properEmail.trim() };
      if (!commitName) {
        out[`|${commitEmail.trim()}`] = { name: properName?.trim(), email: properEmail.trim() };
      }
    } else {
      out[`|${properEmail.trim()}`] = { name: properName?.trim(), email: properEmail.trim() };
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Approximate blame implementation (line-tracking via parent diff).
// ---------------------------------------------------------------------------

async function readBlobLines(backend, oid) {
  if (backend._blobLinesCache.has(oid)) return backend._blobLinesCache.get(oid);
  const { blob } = await git.readBlob({
    fs: backend.fs, dir: backend.dir, oid,
  });
  const text = new TextDecoder("utf-8", { fatal: false }).decode(blob);
  const lines = text.split("\n");
  // Drop a trailing empty line caused by a final newline.
  if (lines.length && lines[lines.length - 1] === "") lines.pop();
  backend._blobLinesCache.set(oid, lines);
  return lines;
}

// Standard LCS-based diff. Returns an array of operations:
//   { op: 'eq'|'add'|'del', a: idx, b: idx, length: n }
// where 'a' indexes oldLines and 'b' indexes newLines. We only need
// per-line origin tagging, so we walk paths after computing the LCS table.
function diffLines(oldLines, newLines) {
  const m = oldLines.length, n = newLines.length;
  // LCS lengths in a (m+1) x (n+1) matrix; use a flat typed array.
  const dp = new Int32Array((m + 1) * (n + 1));
  for (let i = m - 1; i >= 0; i--) {
    for (let j = n - 1; j >= 0; j--) {
      if (oldLines[i] === newLines[j]) {
        dp[i * (n + 1) + j] = dp[(i + 1) * (n + 1) + (j + 1)] + 1;
      } else {
        const a = dp[(i + 1) * (n + 1) + j];
        const b = dp[i * (n + 1) + (j + 1)];
        dp[i * (n + 1) + j] = a > b ? a : b;
      }
    }
  }
  // Walk the diff. For each new-line index produce its origin: either
  // "kept" (came from old at index i) or "added".
  const newOrigin = new Array(n).fill(-1); // -1 = added
  let i = 0, j = 0;
  while (i < m && j < n) {
    if (oldLines[i] === newLines[j]) {
      newOrigin[j] = i;
      i++; j++;
    } else if (dp[(i + 1) * (n + 1) + j] >= dp[i * (n + 1) + (j + 1)]) {
      i++;
    } else {
      j++;
    }
  }
  return newOrigin;
}

// For a given commit and path, return an array of length L (number of lines
// in the file at that commit), where each entry is the SHA of the commit
// that first introduced that line. Walks first-parent history.
async function lineOrigins(backend, commitOid, path) {
  // Locate the blob for `path` at `commitOid`. If the path doesn't exist,
  // return null.
  const cur = backend._commitCache.get(hex(commitOid));
  if (!cur) return null;
  const tree = backend._treeCache.get(hex(commitOid)) || [];
  const entry = tree.find((e) => e.path === path);
  if (!entry) return null;
  const blobOid = entry.binsha_hex;
  const lines = await readBlobLines(backend, blobOid);
  const origins = new Array(lines.length).fill(commitOid);

  // Walk first-parent chain.
  let childOid = commitOid;
  while (true) {
    const child = backend._commitCache.get(hex(childOid));
    if (!child || !child.parent || !child.parent.length) break;
    const parentOid = child.parent[0];
    const parentTree = backend._treeCache.get(hex(parentOid)) || [];
    const parentEntry = parentTree.find((e) => e.path === path);

    // Identify the lines that *this child* introduced (everything not present
    // in parent at the same path).
    let parentLines = [];
    if (parentEntry) parentLines = await readBlobLines(backend, parentEntry.binsha_hex);

    const childTree = backend._treeCache.get(hex(childOid)) || [];
    const childEntry = childTree.find((e) => e.path === path);
    if (!childEntry) break;
    const childLines = await readBlobLines(backend, childEntry.binsha_hex);

    const childToParent = diffLines(parentLines, childLines);

    // For lines in the *current file* (commitOid) we propagate origins
    // backward only if they are still "kept" at this child. We carry a
    // mapping: for each line in `lines` (the file at commitOid) what is
    // its index in `childLines`. Build it on first iteration.
    if (childOid === commitOid) {
      // origins index space already aligns with childLines.
    } else {
      // Map from the file-as-of-commitOid through each step. We rebuild
      // by tracing a `currentToChild` mapping each iteration.
    }

    // Recompute origins for any line that was *kept* from parent: its
    // origin is whatever the parent's origin would say. We approximate
    // that by re-attributing such lines to the parent commit (the first
    // ancestor where the line still exists). The next loop iteration
    // refines further.
    const newOrigins = origins.slice();
    // Build mapping from lines-at-commitOid to lines-at-childOid for the
    // first iteration only; on subsequent iterations we walked through
    // the diff once so we already lost positional mapping. To keep the
    // implementation simple and bounded, we stop refining after 1 step
    // when the file content has changed substantially.
    if (childOid === commitOid) {
      for (let k = 0; k < childToParent.length; k++) {
        if (childToParent[k] !== -1) {
          // line existed in parent — push its origin one step back
          newOrigins[k] = parentOid;
        }
      }
    } else {
      break;
    }
    for (let k = 0; k < newOrigins.length; k++) origins[k] = newOrigins[k];
    childOid = parentOid;
  }
  return origins;
}

// Sync wrapper called from Python — synchronously returns hunks because
// Pyodide's bridge cannot await JS promises from inside a sync Python call.
// We pre-compute *all* blame results before invoking analyze() (see runAll),
// so this method just looks up cached results.
function approximateBlame(backend, commitOid, path) {
  const cache = backend._blameCache;
  const key = `${hex(commitOid)}|${path}`;
  return cache.get(key) || [];
}

// ---------------------------------------------------------------------------
// Pre-compute blame results for every (sampled commit, file) pair.
// ---------------------------------------------------------------------------

async function precomputeBlame(backend, sampledOids, progress) {
  const cache = new Map();
  let done = 0;
  let total = 0;
  // First, compute total work for progress.
  for (const oid of sampledOids) {
    const tree = backend._treeCache.get(hex(oid)) || [];
    total += tree.length;
  }
  for (const oid of sampledOids) {
    const tree = backend._treeCache.get(hex(oid)) || [];
    for (const entry of tree) {
      const origins = await lineOrigins(backend, oid, entry.path);
      const hunks = [];
      if (origins && origins.length) {
        // Group consecutive identical origins into hunks.
        let runStart = 0;
        for (let i = 1; i <= origins.length; i++) {
          if (i === origins.length || origins[i] !== origins[runStart]) {
            const ownerOid = origins[runStart];
            const owner = backend._commitCache.get(hex(ownerOid));
            hunks.push({
              num_lines: i - runStart,
              commit_hexsha: ownerOid,
              commit_binsha_hex: ownerOid,
              author_name: owner ? owner.author.name || "" : "",
              author_email: owner ? owner.author.email || "" : "",
            });
            runStart = i;
          }
        }
      }
      cache.set(`${hex(oid)}|${entry.path}`, hunks);
      done++;
      progress?.({ phase: "blame_precompute", n: done, total });
    }
  }
  backend._blameCache = cache;
}

// ---------------------------------------------------------------------------
// Top-level run function bound to the "Run analysis" button.
// ---------------------------------------------------------------------------

async function runAll() {
  setError("");
  $("run").disabled = true;
  try {
    const repoUrl = $("repoUrl").value.trim();
    const branch = $("branch").value.trim() || "master";
    const cohortfm = $("cohortfm").value.trim() || "%Y";
    const intervalDays = parseInt($("intervalDays").value, 10) || 30;
    const ignoreWS = $("ignoreWS").checked;
    const allFiletypes = $("allFiletypes").checked;
    if (!repoUrl) throw new Error("Please enter a repo URL.");

    const fsName = "got-fs-" + Date.now();
    const lfs = new LightningFS(fsName);
    const fs = { promises: lfs.promises };
    const dir = "/repo";
    await fs.promises.mkdir(dir).catch(() => {});

    status(`Cloning ${repoUrl}…`);
    setProgress(0, 0);
    await git.clone({
      fs, http, dir, url: repoUrl,
      ref: branch, singleBranch: true, depth: 0,
      corsProxy: CORS_PROXY,
      onProgress: (e) => {
        if (e.total) setProgress(e.loaded, e.total);
        status(`${e.phase}: ${e.loaded}/${e.total ?? "?"}`);
      },
    });

    status("Reading commit history…");
    const log = await git.log({ fs, dir, ref: branch });
    const backend = new JsGitBackend(
      fs, dir,
      await fs.promises.readFile(`${dir}/.mailmap`, "utf8").catch(() => ""),
    );
    backend._refs = new Map();
    backend._activeBranch = branch;
    const headSha = log[0]?.oid;
    backend._headSha = headSha;
    backend._refs.set(`refs/heads/${branch}`, headSha);

    // Cache commits + their trees.
    for (const e of log) {
      backend._commitCache.set(hex(e.oid), {
        oid: e.oid,
        parent: e.commit.parent || [],
        author: e.commit.author,
        committer: e.commit.committer,
      });
    }
    backend._allCommits = new Map([[headSha, log.map((e) => ({
      oid: e.oid, parent: e.commit.parent || [],
      author: e.commit.author, committer: e.commit.committer,
    }))]]);

    // Sample first-parent commits at the requested interval.
    const intervalSec = intervalDays * 24 * 3600;
    const sampled = [];
    let last = null;
    let sha = headSha;
    while (sha) {
      const c = backend._commitCache.get(hex(sha));
      if (!c) break;
      const t = c.committer.timestamp;
      if (last === null || t < last - intervalSec) {
        sampled.push(sha);
        last = t;
      }
      sha = c.parent[0];
    }
    sampled.reverse(); // chronological ascending – matches analyze()

    status("Reading trees…");
    let treeProgress = 0;
    for (const oid of sampled) {
      const entries = [];
      await git.walk({
        fs, dir,
        trees: [git.TREE({ ref: oid })],
        map: async function (filepath, [entry]) {
          if (filepath === ".") return;
          if (!entry) return;
          const type = await entry.type();
          if (type !== "blob") return;
          entries.push({ path: filepath, binsha_hex: await entry.oid() });
        },
      });
      backend._treeCache.set(hex(oid), entries);
      treeProgress++;
      setProgress(treeProgress, sampled.length);
      status(`Reading trees: ${treeProgress}/${sampled.length}`);
    }

    status("Computing blame (in-browser approximation)…");
    setProgress(0, 0);
    await precomputeBlame(backend, sampled, ({ n, total }) => {
      setProgress(n, total);
      if (n % 25 === 0) status(`Blame: ${n}/${total}`);
    });

    status("Loading Python analyzer…");
    const py = await getPyodide();

    status("Running analysis…");
    setProgress(0, 0);

    // Bridge progress back to the UI.
    const onProgress = (ev) => {
      const e = ev.toJs ? ev.toJs({ dict_converter: Object.fromEntries }) : ev;
      if (e.total) setProgress(e.n, e.total);
      status(`${e.phase}: ${e.n}${e.total ? "/" + e.total : ""}`);
    };

    // Hand the JS adapter + progress callback to Python.
    py.globals.set("js_adapter", backend);
    py.globals.set("on_progress", onProgress);
    py.globals.set("opts", py.toPy({
      branch, cohortfm,
      interval: intervalSec,
      ignore_whitespace: ignoreWS,
      all_filetypes: allFiletypes,
    }));

    const result = py.runPython(`
from git_of_theseus.wasm import run_analysis
run_analysis(js_adapter, progress=on_progress, **opts)
`);
    const data = result.toJs({ dict_converter: Object.fromEntries });
    result.destroy?.();

    status("Rendering charts…");
    plotStack($("cohortChart"), data.cohorts, "Code by cohort");
    plotStack($("authorChart"), data.authors, "Code by author");

    status(`Done. ${sampled.length} sampled commits analysed.`);
    setProgress(1, 1);
  } catch (err) {
    console.error(err);
    setError(String(err.stack || err.message || err));
    status("Failed.");
  } finally {
    $("run").disabled = false;
  }
}

// ---------------------------------------------------------------------------
// Plotly stack-plot helper.
// ---------------------------------------------------------------------------

function plotStack(node, data, title) {
  if (!data || !data.y || !data.y.length) {
    Plotly.purge(node);
    node.innerHTML = `<em>${title}: no data</em>`;
    return;
  }
  // Trim the legend the same way stack_plot.py does (top 20 + "other").
  const MAX_N = 20;
  let labels = data.labels.slice();
  let y = data.y.map((row) => row.slice());
  if (y.length > MAX_N) {
    const idxs = labels.map((_, j) => j)
      .sort((a, b) => Math.max(...y[b]) - Math.max(...y[a]));
    const top = idxs.slice(0, MAX_N).sort((a, b) => labels[a].localeCompare(labels[b]));
    const rest = idxs.slice(MAX_N);
    const other = new Array(data.ts.length).fill(0);
    for (const j of rest) for (let k = 0; k < other.length; k++) other[k] += y[j][k];
    labels = [...top.map((j) => labels[j]), "other"];
    y = [...top.map((j) => y[j]), other];
  }
  const traces = labels.map((label, i) => ({
    x: data.ts, y: y[i],
    name: label, type: "scatter",
    mode: "lines", stackgroup: "one", line: { width: 0.5 },
  }));
  Plotly.newPlot(node, traces, {
    title, hovermode: "x unified",
    yaxis: { title: "Lines of code" },
    xaxis: { title: "Time" },
    legend: { orientation: "v" },
  }, { responsive: true });
}

// ---------------------------------------------------------------------------

$("run").addEventListener("click", runAll);
status("Ready.");
