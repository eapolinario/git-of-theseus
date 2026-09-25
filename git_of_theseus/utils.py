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

import itertools, json
from pathlib import Path

import dateutil.parser
import numpy


def generate_n_colors(n):
    vs = numpy.linspace(0.4, 0.9, 6)
    colors = [(0.9, 0.4, 0.4)]

    def euclidean(a, b):
        return sum((x - y) ** 2 for x, y in zip(a, b))

    while len(colors) < n:
        new_color = max(
            itertools.product(vs, vs, vs),
            key=lambda a: min(euclidean(a, b) for b in colors),
        )
        colors.append(new_color)
    return colors


def load_curve_inputs(input_fns):
    paths = [Path(input_fns)] if isinstance(input_fns, (str, Path)) else [Path(p) for p in input_fns]
    if not paths:
        raise ValueError("at least one input file is required")

    datasets = []
    all_ts = set()
    for path in paths:
        with path.open() as f:
            data = json.load(f)
        ts = [dateutil.parser.parse(value) for value in data["ts"]]
        if len(data["y"]) != len(data["labels"]):
            raise ValueError(f"{path} has mismatched series and label counts")
        if any(len(row) != len(ts) for row in data["y"]):
            raise ValueError(f"{path} has a series with a mismatched timestamp count")
        datasets.append((path, ts, data["y"], data["labels"]))
        all_ts.update(ts)

    timestamps = sorted(all_ts)
    labels = []
    rows = []
    prefix_labels = len(paths) > 1
    for path, ts, values, source_labels in datasets:
        source = path.parent.name or path.stem
        positions = {timestamp: index for index, timestamp in enumerate(ts)}
        for label, values_for_label in zip(source_labels, values):
            row = []
            current = 0
            for timestamp in timestamps:
                if timestamp in positions:
                    current = values_for_label[positions[timestamp]]
                row.append(current)
            labels.append(f"{source}: {label}" if prefix_labels else label)
            rows.append(row)
    return {"y": numpy.array(rows), "ts": timestamps, "labels": labels}
