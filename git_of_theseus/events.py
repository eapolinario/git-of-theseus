"""Load and render optional external event annotations."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from datetime import date, datetime, timezone
from pathlib import Path
from typing import Any, Iterable

import yaml
from dateutil.parser import isoparse
from matplotlib import dates as mdates
from matplotlib.lines import Line2D

EVENT_LINE_STYLES = {
    "announced": "--",
    "available": "-",
    "pilot": "-.",
    "default": ":",
    "retired": (0, (5, 2, 1, 2)),
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
    if isinstance(value, datetime):
        parsed = value
        if parsed.tzinfo is not None:
            parsed = parsed.astimezone(timezone.utc)
        return parsed.date()
    if isinstance(value, date):
        return value
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"events[{index}].date must be an ISO date or timestamp")
    try:
        parsed = isoparse(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(
            f"events[{index}].date must be an ISO date or timestamp"
        ) from exc
    if parsed.tzinfo is not None:
        parsed = parsed.astimezone(timezone.utc)
    return parsed.date()


def load_events(path: str | Path) -> list[Event]:
    """Load and validate an event manifest from YAML."""
    try:
        with Path(path).open(encoding="utf-8") as handle:
            document = yaml.safe_load(handle)
    except OSError as exc:
        raise ValueError(f"cannot read events manifest {path}: {exc}") from exc
    if not isinstance(document, dict):
        raise ValueError("events manifest must contain a YAML mapping")
    schema_version = document.get("schema_version", 1)
    if schema_version != 1:
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


def add_event_annotations(
    ax: Any, events: Iterable[Event], *, include_legend: bool = True
) -> None:
    """Add in-range event lines and labels to a calendar-time matplotlib axis."""
    events = sorted(events, key=lambda event: event.date)
    if not events:
        return
    x_min, x_max = ax.get_xlim()
    visible = [
        event
        for event in events
        if x_min <= mdates.date2num(event.date) <= x_max
    ]
    if not visible:
        return

    previous_x = None
    level = 0
    for event in visible:
        x = mdates.date2num(event.date)
        if previous_x is not None and abs(x - previous_x) < max((x_max - x_min) * 0.04, 1):
            level = (level + 1) % 3
        else:
            level = 0
        previous_x = x
        color = _provider_color(event.provider)
        ax.axvline(
            event.date,
            color=color,
            linestyle=EVENT_LINE_STYLES[event.event],
            linewidth=1.8,
            alpha=0.9,
            zorder=5,
        )
        ax.annotate(
            event.label,
            xy=(event.date, 1),
            xycoords=("data", "axes fraction"),
            xytext=(0, -6 - level * 14),
            textcoords="offset points",
            rotation=90,
            va="top",
            ha="right",
            color=color,
            fontsize=9,
            clip_on=True,
        )

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


def add_events_from_path(ax: Any, path: str | Path | None) -> None:
    if path is not None:
        add_event_annotations(ax, load_events(path))
