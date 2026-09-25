import json
from pathlib import Path

from matplotlib import pyplot

from git_of_theseus.line_plot import line_plot
from git_of_theseus.stack_plot import stack_plot
from git_of_theseus.utils import load_curve_inputs


def _write_curve(path: Path, timestamps, labels, values):
    path.write_text(
        json.dumps({"ts": timestamps, "labels": labels, "y": values}),
        encoding="utf-8",
    )


def test_load_curve_inputs_aligns_repositories(tmp_path):
    first = tmp_path / "first" / "cohorts.json"
    second = tmp_path / "second" / "cohorts.json"
    first.parent.mkdir()
    second.parent.mkdir()
    _write_curve(
        first,
        ["2020-01-01T00:00:00", "2020-01-03T00:00:00"],
        ["Code added in 2020"],
        [[2, 4]],
    )
    _write_curve(
        second,
        ["2020-01-02T00:00:00", "2020-01-03T00:00:00"],
        ["Code added in 2020"],
        [[3, 5]],
    )

    data = load_curve_inputs([first, second])

    assert [timestamp.isoformat() for timestamp in data["ts"]] == [
        "2020-01-01T00:00:00",
        "2020-01-02T00:00:00",
        "2020-01-03T00:00:00",
    ]
    assert data["labels"] == [
        "first: Code added in 2020",
        "second: Code added in 2020",
    ]
    assert data["y"].tolist() == [[2, 2, 4], [0, 3, 5]]


def test_plot_multiple_repositories(tmp_path):
    first = tmp_path / "first" / "authors.json"
    second = tmp_path / "second" / "authors.json"
    first.parent.mkdir()
    second.parent.mkdir()
    _write_curve(
        first,
        ["2020-01-01T00:00:00", "2020-01-02T00:00:00"],
        ["Alice"],
        [[2, 4]],
    )
    _write_curve(
        second,
        ["2020-01-01T00:00:00", "2020-01-02T00:00:00"],
        ["Bob"],
        [[3, 5]],
    )

    line_output = tmp_path / "line.png"
    stack_output = tmp_path / "stack.png"
    line_plot([first, second], outfile=line_output)
    stack_plot([first, second], outfile=stack_output)

    assert line_output.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    assert stack_output.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    pyplot.close("all")
