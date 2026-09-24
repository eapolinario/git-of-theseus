from datetime import date

import matplotlib
import pytest
from matplotlib import pyplot

from git_of_theseus.events import Event, add_event_annotations, load_events

matplotlib.use("Agg")


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
    pyplot.close(figure)


def test_survival_plot_rejects_events():
    from git_of_theseus.survival_plot import survival_plot

    with pytest.raises(ValueError, match="elapsed age"):
        survival_plot([], events="events.yaml")
