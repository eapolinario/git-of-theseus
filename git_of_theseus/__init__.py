from git_of_theseus.analyze import analyze, analyze_cmdline

# The plotting modules depend on matplotlib/numpy/scipy/python-dateutil, which
# are an optional `[plot]` extra. Import them lazily so that the core package
# (used e.g. from Pyodide / WASM in the browser) does not require them.
try:
    from git_of_theseus.survival_plot import survival_plot, survival_plot_cmdline
    from git_of_theseus.stack_plot import stack_plot, stack_plot_cmdline
    from git_of_theseus.line_plot import line_plot, line_plot_cmdline
except ImportError:  # pragma: no cover - exercised only without plot extras
    def _missing_plot_extras(*_args, **_kwargs):
        raise ImportError(
            "git-of-theseus plotting requires the optional 'plot' extras. "
            "Install with: pip install 'git-of-theseus[plot]'"
        )

    survival_plot = survival_plot_cmdline = _missing_plot_extras
    stack_plot = stack_plot_cmdline = _missing_plot_extras
    line_plot = line_plot_cmdline = _missing_plot_extras
