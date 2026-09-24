from dataclasses import replace
from datetime import date, datetime, timezone
from io import BytesIO
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET

import matplotlib
import pytest
import yaml
from matplotlib import pyplot

from git_of_theseus.events import (
    EVENT_LINE_STYLES,
    Event,
    _provider_color,
    add_event_annotations,
    add_events_from_path,
    load_events,
)
from git_of_theseus.line_plot import line_plot
from git_of_theseus.stack_plot import stack_plot

matplotlib.use("Agg")
FIXTURES = Path(__file__).parent / "fixtures"
SVG = "{http://www.w3.org/2000/svg}"


@pytest.fixture(autouse=True)
def close_figures():
    yield
    pyplot.close("all")


def _record(**changes):
    return {
        "date": "2024-01-05",
        "provider": "Example",
        "model": "Model",
        "event": "available",
        "scope": "public",
        "label": "A model release",
        "source": "release notes",
        **changes,
    }


def _manifest(tmp_path, document):
    return _write(tmp_path / "events.yaml", yaml.safe_dump(document))


def _write(path, text):
    path.write_text(text, encoding="utf-8")
    return path


def test_load_events_accepts_quoted_and_yaml_dates(tmp_path):
    path = _write(
        tmp_path / "events.yaml",
        """
schema_version: 1
events:
  - date: "2024-01-03"
    provider: Acme
    model: Model A
    event: available
    scope: public
    label: Model A available
    source: https://example.test/a
  - date: 2024-01-05
    provider: Acme
    model: Model B
    event: announced
    scope: public
    label: Model B announced
    source: release notes
    evidence:
      confidence: high
""",
    )
    events = load_events(path)
    assert [event.date for event in events] == [date(2024, 1, 3), date(2024, 1, 5)]
    assert events[1].notes is None


def test_load_events_rejects_bad_records(tmp_path):
    path = _write(
        tmp_path / "events.yaml",
        """
events:
  - date: 2024-01-01
    provider: Acme
    model: Model A
    event: unknown
    scope: public
    label: Model A
    source: source
""",
    )
    with pytest.raises(ValueError, match="event must be one of"):
        load_events(path)


def test_annotations_draw_lines_and_labels_only_in_range():
    figure, axis = pyplot.subplots()
    axis.plot([date(2024, 1, 1), date(2024, 1, 10)], [0, 1], label="series")
    original_xlim = axis.get_xlim()
    events = [
        Event(
            date(2024, 1, 3),
            "Acme",
            "Model A",
            "available",
            "public",
            "Model A",
            "source",
        ),
        Event(
            date(2025, 1, 1),
            "Acme",
            "Model B",
            "announced",
            "public",
            "out of range",
            "source",
        ),
    ]
    add_event_annotations(axis, events)
    assert len(axis.lines) == 2
    assert len(axis.texts) == 1
    assert axis.get_xlim() == original_xlim
    assert axis.lines[-1].get_linestyle() == "-"
    assert "Provider: Acme" in [text.get_text() for text in axis.get_legend().get_texts()]
    assert axis.texts[0].get_text() == "1"
    key = [text for text in figure.texts if text.get_gid() == "event-key-1"]
    assert "2024-01-03" in key[0].get_text()
    assert "Model A" in key[0].get_text()
    pyplot.close(figure)


def test_survival_plot_rejects_events():
    from git_of_theseus.survival_plot import survival_plot

    with pytest.raises(ValueError, match="elapsed age"):
        survival_plot([], events="events.yaml")


def test_shared_manifest_dates_order_and_metadata():
    events = load_events(FIXTURES / "events.yaml")
    assert len(events) == 7
    assert [event.date for event in events] == [
        date(2023, 12, 1),
        date(2024, 1, 1),
        date(2024, 1, 5),
        date(2024, 1, 5),
        date(2024, 1, 5),
        date(2024, 1, 10),
        date(2024, 2, 1),
    ]
    assert [event.event for event in events[2:5]] == ["available", "pilot", "default"]
    assert events[2].notes == "The UTC date is January 5."
    assert events[2].label == 'Available <One> & "Two"'


@pytest.mark.parametrize(
    "value, expected",
    [
        ("2024-01-05", date(2024, 1, 5)),
        (date(2024, 1, 5), date(2024, 1, 5)),
        ("2024-01-05T12:34:56", date(2024, 1, 5)),
        ("2024-01-05T00:30:00+02:00", date(2024, 1, 4)),
        ("2024-01-05T23:30:00-02:00", date(2024, 1, 6)),
        ("2024-01-05T23:30:00.123456Z", date(2024, 1, 5)),
        ("2024-01-05T12", date(2024, 1, 5)),
        ("2024-01-05T1230", date(2024, 1, 5)),
        ("2024-01-05T123000", date(2024, 1, 5)),
        ("2024-01-05T12:30:00+02", date(2024, 1, 5)),
        ("2024-01-05T24:00:00", date(2024, 1, 6)),
        ("2024-01-05T23:30:00,123456-0200", date(2024, 1, 6)),
        (datetime(2024, 1, 5, 12, tzinfo=timezone.utc), date(2024, 1, 5)),
    ],
)
def test_dates(tmp_path, value, expected):
    path = _manifest(tmp_path, {"events": [_record(date=value)]})
    assert load_events(path)[0].date == expected


@pytest.mark.parametrize(
    "value",
    [
        None,
        "",
        True,
        123,
        "2024",
        "2024-01",
        "2024-001",
        "20240105",
        "2024-02-30",
        "0000-01-01",
        "0001-01-01T00:00:00+02:00",
        "9999-12-31T23:00:00-02:00",
        "2024-01-05T23:59:60Z",
        "2024-01-05T24:00:01",
        "2024-01-05T12:30.5",
        "2024-01-05T24:00:00.1",
    ],
)
def test_invalid_dates(tmp_path, value):
    path = _manifest(tmp_path, {"events": [_record(date=value)]})
    with pytest.raises(ValueError, match=r"events\[0\].date"):
        load_events(path)


@pytest.mark.parametrize("version", [True, 1.0, "1", None, 2])
def test_invalid_schema_versions(tmp_path, version):
    path = _manifest(tmp_path, {"schema_version": version, "events": []})
    with pytest.raises(ValueError, match="schema_version must be 1"):
        load_events(path)


@pytest.mark.parametrize(
    "document, message",
    [
        ([], "YAML mapping"),
        (None, "YAML mapping"),
        ({}, "events list"),
        ({"events": {}}, "events list"),
        ({"events": ["bad"]}, r"events\[0\] must be a YAML mapping"),
        ({"events": [{}]}, "missing required fields"),
        ({"events": [_record(provider=" ")]}, "provider must be a non-empty string"),
        ({"events": [_record(model=3)]}, "model must be a non-empty string"),
        ({"events": [_record(notes=3)]}, "notes must be a string"),
    ],
)
def test_invalid_manifest_shapes(tmp_path, document, message):
    with pytest.raises(ValueError, match=message):
        load_events(_manifest(tmp_path, document))


def test_file_errors_include_manifest_path(tmp_path):
    path = tmp_path / "missing.yaml"
    with pytest.raises(ValueError, match="cannot read events manifest"):
        load_events(path)
    _write(path, "events: [\n")
    with pytest.raises(ValueError, match="cannot parse events manifest"):
        load_events(path)
    path.write_bytes(b"\xff\xfe")
    with pytest.raises(ValueError, match="cannot read events manifest"):
        load_events(path)


@pytest.mark.parametrize("events", [[], None])
def test_no_events_preserves_plot(events):
    figure, axis = pyplot.subplots()
    axis.plot([date(2024, 1, 1), date(2024, 1, 10)], [1, 2], label="code")
    axis.legend()
    limits = axis.get_xlim(), axis.get_ylim()
    size = figure.get_size_inches().copy()
    if events is None:
        add_events_from_path(axis, None)
    else:
        add_event_annotations(axis, events)
    assert (figure.get_size_inches() == size).all()
    assert (axis.get_xlim(), axis.get_ylim()) == limits
    assert len(axis.lines) == 1
    assert not axis.texts
    assert not figure.texts


def test_range_boundaries_styles_and_legend():
    figure, axis = pyplot.subplots()
    axis.plot([date(2024, 1, 1), date(2024, 1, 10)], [0, 100], label="code")
    axis.set_xlim(date(2024, 1, 1), date(2024, 1, 10))
    axis.set_ylim(0, 100)
    axis.legend()
    limits = axis.get_xlim(), axis.get_ylim()
    events = load_events(FIXTURES / "events.yaml")
    add_event_annotations(axis, reversed(events))
    assert (axis.get_xlim(), axis.get_ylim()) == limits
    assert len(axis.lines) == 6
    assert len(axis.texts) == 5
    assert [line.get_xdata()[0] for line in axis.lines[1:]] == [
        event.date for event in events[1:-1]
    ]
    visible = sorted(reversed(events[1:-1]), key=lambda event: event.date)
    for line, event in zip(axis.lines[1:], visible):
        assert line.get_color() == _provider_color(event.provider)
        assert line.is_dashed() == (event.event != "available")
    texts = " ".join(text.get_text() for text in figure.texts)
    assert "Excluded" not in texts
    assert "Restricted test group" in texts
    assert "dates do not show model use or impact" in texts
    legend = [text.get_text() for text in axis.get_legend().get_texts()]
    assert "code" in legend
    assert "Provider: Outside" not in legend
    assert all(f"Event: {kind}" in legend for kind in EVENT_LINE_STYLES)


def test_no_in_range_events_preserves_size_and_legend():
    figure, axis = pyplot.subplots()
    axis.plot([date(2025, 1, 1), date(2025, 1, 10)], [1, 2], label="code")
    legend = axis.legend()
    size = figure.get_size_inches().copy()
    add_event_annotations(axis, load_events(FIXTURES / "events.yaml"))
    assert (figure.get_size_inches() == size).all()
    assert axis.get_legend() is legend
    assert not figure.texts


def test_can_omit_event_legend():
    figure, axis = pyplot.subplots()
    axis.plot([date(2024, 1, 1), date(2024, 1, 10)], [1, 2], label="code")
    legend = axis.legend()
    add_event_annotations(axis, load_events(FIXTURES / "events.yaml"), include_legend=False)
    assert axis.get_legend() is legend


@pytest.mark.parametrize("count", [85, 100])
def test_dense_labels_do_not_overlap_or_leave_figure(count):
    figure, axis = pyplot.subplots(figsize=(16, 12), dpi=120)
    axis.plot([date(2024, 1, 1), date(2024, 1, 10)], [1, 2])
    axis.set_xlim(date(2024, 1, 1), date(2024, 1, 10))
    figure.tight_layout()
    event = Event(
        date(2024, 1, 1), "Example", "Model", "pilot", "Restricted test", "", "source"
    )
    events = [
        replace(
            event,
            date=date(2024, 1, 1 + (index % 10)),
            label=f"Event {index} with $literal$ text " + "x" * (index % 75),
        )
        for index in range(count)
    ]
    add_event_annotations(axis, events)
    figure.canvas.draw()
    renderer = figure.canvas.get_renderer()
    markers = [text.get_bbox_patch().get_window_extent(renderer) for text in axis.texts]
    keys = [
        text.get_window_extent(renderer)
        for text in figure.texts
        if str(text.get_gid()).startswith("event-key-")
    ]
    assert len(markers) == len(keys) == count
    assert all(
        not text.get_parse_math()
        for text in figure.texts
        if str(text.get_gid()).startswith("event-key-")
    )
    connectors = [
        patch
        for patch in axis.patches
        if str(patch.get_gid()).startswith("event-connector-")
    ]
    assert len(connectors) == count
    assert max(patch.get_zorder() for patch in connectors) < min(
        text.get_zorder() for text in axis.texts
    )
    for boxes in (markers, keys):
        assert not any(a.overlaps(b) for i, a in enumerate(boxes) for b in boxes[i + 1 :])
        assert all(
            figure.bbox.contains(b.x0, b.y0) and figure.bbox.contains(b.x1, b.y1)
            for b in boxes
        )


def test_provider_colors_match_rust():
    providers = ["anthropic", "openai", "deepseek", "kimi"]
    assert {name: _provider_color(name) for name in providers} == {
        "anthropic": "#17becf",
        "openai": "#8c564b",
        "deepseek": "#ff7f0e",
        "kimi": "#bcbd22",
    }


@pytest.mark.parametrize("plot", [line_plot, stack_plot])
@pytest.mark.parametrize("normalize", [False, True])
@pytest.mark.parametrize("extension", ["png", "svg"])
def test_plot_api_with_events(tmp_path, plot, normalize, extension):
    output = tmp_path / f"plot.{extension}"
    plot(
        FIXTURES / "curve.json",
        outfile=output,
        normalize=normalize,
        max_n=1,
        events=FIXTURES / "events.yaml",
    )
    content = output.read_bytes()
    assert len(content) > 1000
    if extension == "png":
        assert content.startswith(b"\x89PNG\r\n\x1a\n")
    else:
        assert b'id="event-key-5"' in content
        assert b'id="event-key-6"' not in content
        root = ET.fromstring(content)
        assert len(root.findall(f".//{SVG}title")) == 15
        for number in range(1, 6):
            titles = []
            for part in ("line", "label", "key"):
                group = root.find(f".//{SVG}g[@id='event-{part}-{number}']")
                assert group is not None
                title = group[0]
                assert title.tag == f"{SVG}title"
                assert group.get("aria-labelledby") == title.get("id")
                assert group.get("cursor") == "help"
                titles.append(title.text)
            assert titles[0] == titles[1] == titles[2]
            assert "Excluded" not in titles[0]
        title = root.find(f".//{SVG}g[@id='event-label-2']/{SVG}title")
        assert title.text == (
            'Available <One> & "Two"\nDate: 2024-01-05\nProvider: Example A\n'
            "Model: Model One\nEvent: available\nScope: Public test preview\n"
            "Source: release notes\nNotes: The UTC date is January 5."
        )
        assert "Notes:" not in root.find(
            f".//{SVG}g[@id='event-label-1']/{SVG}title"
        ).text
    if normalize:
        assert pyplot.gca().get_ylim() == (0.0, 100.0)


@pytest.mark.parametrize("module", ["line_plot", "stack_plot"])
def test_cli_renders_events(tmp_path, module):
    output = tmp_path / "plot.svg"
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            f"git_of_theseus.{module}",
            str(FIXTURES / "curve.json"),
            "--events",
            str(FIXTURES / "events.yaml"),
            "--outfile",
            str(output),
            "--normalize",
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    assert 'id="event-line-5"' in output.read_text(encoding="utf-8")


@pytest.mark.parametrize("module", ["line_plot", "stack_plot", "survival_plot"])
def test_cli_rejects_invalid_events_without_output(tmp_path, module):
    output = tmp_path / "plot.png"
    output.write_bytes(b"existing output")
    manifest = _write(tmp_path / "bad.yaml", "events: [")
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            f"git_of_theseus.{module}",
            str(FIXTURES / "curve.json"),
            "--events",
            str(manifest),
            "--outfile",
            str(output),
        ],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 2
    assert "error:" in result.stderr
    assert "Traceback" not in result.stderr
    expected = "elapsed age" if module == "survival_plot" else "cannot parse events manifest"
    assert expected in result.stderr
    assert output.read_bytes() == b"existing output"


@pytest.mark.parametrize("font_type", ["path", "none"])
def test_svg_tooltips_escape_all_metadata_and_have_hover_areas(tmp_path, font_type):
    source = 'https://example.invalid/?a=1&b="<tag>"'
    notes = '<script>not executable</script>\nKeep "quotes" & Unicode: \u00e9.'
    manifest = _manifest(tmp_path, {"events": [_record(source=source, notes=notes)]})
    output = tmp_path / "events.SVG"
    with matplotlib.rc_context({"svg.fonttype": font_type}):
        line_plot(FIXTURES / "curve.json", events=manifest, outfile=output)
    root = ET.parse(output).getroot()
    assert not root.findall(f".//{SVG}script")
    for title in root.findall(f".//{SVG}title"):
        assert f"Source: {source}" in title.text
        assert f"Notes: {notes}" in title.text
    line = root.find(f".//{SVG}g[@id='event-line-1']")
    assert "stroke-width: 8" in line[-1].get("style")
    assert "pointer-events: stroke" in line[-1].get("style")
    key = root.find(f".//{SVG}g[@id='event-key-1']")
    assert key.get("pointer-events") == "all"
    assert key.find(f".//{SVG}path[@style='fill: none']") is not None


def test_invalid_tooltip_text_does_not_replace_svg(tmp_path):
    manifest = _manifest(tmp_path, {"events": [_record(notes="invalid \x00 text")]})
    output = _write(tmp_path / "out.svg", "existing output")
    with pytest.raises(ValueError, match="invalid XML character"):
        line_plot(FIXTURES / "curve.json", events=manifest, outfile=output)
    assert output.read_text() == "existing output"


def test_file_like_png_output_still_works():
    output = BytesIO()
    stack_plot(FIXTURES / "curve.json", events=FIXTURES / "events.yaml", outfile=output)
    assert output.getvalue().startswith(b"\x89PNG\r\n\x1a\n")
