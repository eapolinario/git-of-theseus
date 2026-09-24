"""Load and render optional external event annotations."""

from __future__ import annotations

import hashlib
from copy import deepcopy
from io import StringIO
from os import PathLike
import re
import textwrap
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from datetime import date, datetime, timezone
from pathlib import Path
from typing import Any, Iterable

import yaml
from dateutil.parser import isoparse
from matplotlib import dates as mdates
from matplotlib.font_manager import FontProperties
from matplotlib.lines import Line2D
from matplotlib.patches import ConnectionPatch

EVENT_LINE_STYLES = {
    "announced": "--",
    "available": "-",
    "pilot": "-.",
    "default": ":",
    "retired": (0, (5, 2, 1, 2, 1, 2)),
}
PROVIDER_COLORS = (
    "#1f77b4",
    "#ff7f0e",
    "#2ca02c",
    "#d62728",
    "#9467bd",
    "#8c564b",
    "#e377c2",
    "#7f7f7f",
    "#bcbd22",
    "#17becf",
)
REQUIRED_FIELDS = ("date", "provider", "model", "event", "scope", "label", "source")
VALID_EVENTS = frozenset(EVENT_LINE_STYLES)


@dataclass(frozen=True)
class Event:
    date: date
    provider: str
    model: str
    event: str
    scope: str
    label: str
    source: str
    notes: str | None = None


def _parse_date(value: Any, index: int) -> date:
    if isinstance(value, date) and not isinstance(value, datetime):
        return value
    if not isinstance(value, datetime) and (
        not isinstance(value, str)
        or not re.match(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}(?:$|[Tt ])", value)
    ):
        raise ValueError(f"events[{index}].date must be an ISO date or timestamp")
    try:
        parsed = value if isinstance(value, datetime) else isoparse(value)
        if parsed.tzinfo is not None:
            parsed = parsed.astimezone(timezone.utc)
        return parsed.date()
    except (TypeError, ValueError, OverflowError) as exc:
        raise ValueError(
            f"events[{index}].date must be an ISO date or timestamp"
        ) from exc


def load_events(path: str | Path) -> list[Event]:
    """Load and validate an event manifest from YAML."""
    try:
        with Path(path).open(encoding="utf-8") as handle:
            document = yaml.safe_load(handle)
    except (OSError, UnicodeError) as exc:
        raise ValueError(f"cannot read events manifest {path}: {exc}") from exc
    except yaml.YAMLError as exc:
        raise ValueError(f"cannot parse events manifest {path}: {exc}") from exc
    if not isinstance(document, dict):
        raise ValueError("events manifest must contain a YAML mapping")
    schema_version = document.get("schema_version", 1)
    if type(schema_version) is not int or schema_version != 1:
        raise ValueError("events manifest schema_version must be 1")
    records = document.get("events")
    if not isinstance(records, list):
        raise ValueError("events manifest must contain an events list")

    parsed: list[Event] = []
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            raise ValueError(f"events[{index}] must be a YAML mapping")
        missing = [field for field in REQUIRED_FIELDS if field not in record]
        if missing:
            raise ValueError(
                f"events[{index}] is missing required fields: {', '.join(missing)}"
            )
        values = {field: record[field] for field in REQUIRED_FIELDS}
        for field, value in values.items():
            if field != "date" and (not isinstance(value, str) or not value.strip()):
                raise ValueError(f"events[{index}].{field} must be a non-empty string")
        if values["event"] not in VALID_EVENTS:
            allowed = ", ".join(sorted(VALID_EVENTS))
            raise ValueError(f"events[{index}].event must be one of: {allowed}")
        notes = record.get("notes")
        if notes is not None and not isinstance(notes, str):
            raise ValueError(f"events[{index}].notes must be a string when provided")
        parsed.append(
            Event(
                date=_parse_date(values["date"], index),
                provider=values["provider"],
                model=values["model"],
                event=values["event"],
                scope=values["scope"],
                label=values["label"],
                source=values["source"],
                notes=notes,
            )
        )
    return sorted(parsed, key=lambda event: event.date)


def _provider_color(provider: str) -> str:
    digest = hashlib.sha256(provider.encode("utf-8")).digest()
    return PROVIDER_COLORS[digest[0] % len(PROVIDER_COLORS)]


def _tooltip_text(event: Event) -> str:
    lines = [
        event.label,
        f"Date: {event.date.isoformat()}",
        f"Provider: {event.provider}",
        f"Model: {event.model}",
        f"Event: {event.event}",
        f"Scope: {event.scope}",
        f"Source: {event.source}",
    ]
    if event.notes:
        lines.append(f"Notes: {event.notes}")
    return "\n".join(lines)


def add_event_annotations(
    ax: Any, events: Iterable[Event], *, include_legend: bool = True
) -> dict[str, str]:
    """Add numbered markers and an event key after the plot layout is set.

    Extra space is added outside the axes, preserving the data area and limits.
    Do not run tight_layout after this function.
    Return the SVG group IDs and tooltip text for save_event_plot.
    """
    events = sorted(events, key=lambda event: event.date)
    if not events:
        return {}
    x_min, x_max = ax.get_xlim()
    visible = [
        event
        for event in events
        if x_min <= mdates.date2num(event.date) <= x_max
    ]
    if not visible:
        return {}

    if include_legend:
        handles, labels = ax.get_legend_handles_labels()
        providers = sorted({event.provider for event in visible})
        event_types = sorted({event.event for event in visible})
        handles.extend(
            Line2D([0], [0], color=_provider_color(provider), linewidth=2)
            for provider in providers
        )
        labels.extend(f"Provider: {provider}" for provider in providers)
        handles.extend(
            Line2D(
                [0],
                [0],
                color="black",
                linestyle=EVENT_LINE_STYLES[event_type],
                linewidth=1.8,
            )
            for event_type in event_types
        )
        labels.extend(f"Event: {event_type}" for event_type in event_types)
        ax.legend(handles, labels, loc=2)

    figure = ax.figure
    figure.canvas.draw()
    renderer = figure.canvas.get_renderer()
    dpi = figure.dpi
    scale = dpi / 72
    font = FontProperties(family="monospace", size=9)
    char_width = renderer.get_text_width_height_descent("M", font, False)[0]
    width, height = figure.get_size_inches() * dpi
    bounds = ax.get_window_extent().frozen()
    gap = 5 * scale
    row_height = 16 * scale

    placements = []
    lane_ends = []
    for index, event in enumerate(visible, 1):
        x = ax.transData.transform((mdates.date2num(event.date), 0))[0]
        label_width = len(str(index)) * char_width + gap * 2
        left = max(bounds.x0, min(x - label_width / 2, bounds.x1 - label_width))
        lane = next(
            (i for i, end in enumerate(lane_ends) if end + gap <= left),
            len(lane_ends),
        )
        if lane == len(lane_ends):
            lane_ends.append(0)
        lane_ends[lane] = left + label_width
        placements.append((left + label_width / 2 - x, lane))

    columns = max(1, min(3, int(bounds.width / (250 * scale))))
    column_width = bounds.width / columns
    wrap_width = max(1, int((column_width - gap * 4) / char_width))
    entries = []
    key_y = 30 * scale
    for row_start in range(0, len(visible), columns):
        row = []
        for offset, event in enumerate(visible[row_start : row_start + columns]):
            index = row_start + offset + 1
            parts = (
                f"{index}. {event.date.isoformat()} | {event.provider} | {event.event}",
                event.label,
                f"Scope: {event.scope}",
            )
            lines = [
                line
                for part in parts
                for line in textwrap.wrap(part, width=wrap_width)
            ]
            row.append((offset, index, event, lines))
        for offset, index, event, lines in row:
            entries.append((offset, index, event, "\n".join(lines), key_y))
        key_y += max(len(lines) for _, _, _, lines in row) * row_height + gap

    key_height = key_y + 15 * scale
    top_height = (len(lane_ends) + 2) * row_height
    new_height = height + top_height + key_height
    figure.set_size_inches(width / dpi, new_height / dpi, forward=True)
    ax.set_position(
        [
            bounds.x0 / width,
            (bounds.y0 + key_height) / new_height,
            bounds.width / width,
            bounds.height / new_height,
        ]
    )
    figure.text(
        bounds.x0 / width,
        (new_height - 10 * scale) / new_height,
        f"External events ({len(visible)}) - dates do not show model use or impact",
        fontsize=10,
        va="top",
        gid="event-notice",
    )
    figure.text(
        bounds.x0 / width,
        (key_height - 8 * scale) / new_height,
        "Event key",
        fontsize=10,
        weight="bold",
        va="top",
    )

    for index, (event, (offset, lane)) in enumerate(zip(visible, placements), 1):
        color = _provider_color(event.provider)
        ax.axvline(
            event.date,
            color=color,
            linestyle=EVENT_LINE_STYLES[event.event],
            linewidth=1.2,
            alpha=0.8,
            zorder=5,
            gid=f"event-line-{index}",
        )
        x = ax.transData.transform((mdates.date2num(event.date), 0))[0]
        ax.add_artist(
            ConnectionPatch(
                (mdates.date2num(event.date), 1),
                (
                    (x + offset) / width,
                    (bounds.y1 + key_height + (6 + lane * 16) * scale) / new_height,
                ),
                coordsA=ax.get_xaxis_transform(),
                coordsB=figure.transFigure,
                color=color,
                linewidth=0.6,
                clip_on=False,
                zorder=4,
                gid=f"event-connector-{index}",
            )
        )
        ax.annotate(
            str(index),
            xy=(event.date, 1),
            xycoords=("data", "axes fraction"),
            xytext=(offset / scale, 6 + lane * 16),
            textcoords="offset points",
            va="bottom",
            ha="center",
            fontproperties=font,
            color="black",
            bbox={"facecolor": "white", "edgecolor": color, "pad": 2},
            annotation_clip=False,
            zorder=6,
            gid=f"event-label-{index}",
        )

    for column, index, event, text, y in entries:
        x = bounds.x0 + column * column_width
        figure.add_artist(
            Line2D(
                [(x + gap) / width, (x + gap * 3) / width],
                [(key_height - y - gap) / new_height] * 2,
                transform=figure.transFigure,
                color=_provider_color(event.provider),
                linestyle=EVENT_LINE_STYLES[event.event],
                linewidth=1.8,
            )
        )
        figure.text(
            (x + gap * 4) / width,
            (key_height - y) / new_height,
            text,
            fontproperties=font,
            parse_math=False,
            linespacing=1.5,
            va="top",
            gid=f"event-key-{index}",
            bbox={"facecolor": "none", "edgecolor": "none", "pad": 2},
        )

    return {
        f"event-{part}-{index}": _tooltip_text(event)
        for index, event in enumerate(visible, 1)
        for part in ("line", "label", "key")
    }


def save_event_plot(
    figure: Any, path: str | Path, tooltips: dict[str, str]
) -> None:
    """Save the plot, with native browser tooltips in annotated SVG output."""
    if (
        not tooltips
        or not isinstance(path, (str, PathLike))
        or Path(path).suffix.lower() != ".svg"
    ):
        figure.savefig(path)
        return

    for group_id, text in tooltips.items():
        if re.search(r"[\x00-\x08\x0b\x0c\x0e-\x1f\ud800-\udfff\ufffe\uffff]", text):
            raise ValueError(f"SVG tooltip {group_id!r} contains an invalid XML character")

    svg = StringIO()
    figure.savefig(svg, format="svg")
    root, elements = ET.XMLID(svg.getvalue())
    namespace = "http://www.w3.org/2000/svg"
    for group_id, text in tooltips.items():
        group = elements.get(group_id)
        if group is None:
            raise ValueError(f"SVG event group {group_id!r} is missing")
        title_id = f"tooltip-{group_id}"
        title = ET.Element(f"{{{namespace}}}title", {"id": title_id})
        title.text = text
        group.insert(0, title)
        group.set("aria-labelledby", title_id)
        group.set("cursor", "help")
        group.set("pointer-events", "all")
        if group_id.startswith("event-line-"):
            line = group.find(f"{{{namespace}}}path")
            if line is None:
                raise ValueError(f"SVG event line {group_id!r} has no path")
            target = deepcopy(line)
            target.attrib.pop("id", None)
            target.set(
                "style",
                "fill: none; stroke: transparent; stroke-width: 8; pointer-events: stroke",
            )
            group.append(target)

    ET.register_namespace("", namespace)
    ET.register_namespace("xlink", "http://www.w3.org/1999/xlink")
    ET.ElementTree(root).write(path, encoding="utf-8", xml_declaration=True)


def add_events_from_path(ax: Any, path: str | Path | None) -> dict[str, str]:
    if path is not None:
        return add_event_annotations(ax, load_events(path))
    return {}
