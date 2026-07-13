"""Catalog primitives shared by the lazy project-workspace provider."""

from __future__ import annotations

import copy
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import quote


VIEWS = frozenset({"map", "scene3d", "wells", "sections", "volume", "charts"})
ROLE_ALIASES = {
    "surface": {"surface"},
    "surfaces": {"surface"},
    "structure": {"surface"},
    "structures": {"surface"},
    "point": {"point"},
    "points": {"point"},
    "polygon": {"polygon"},
    "polygons": {"polygon"},
    "well": {"bore"},
    "wells": {"bore"},
    "bore": {"bore"},
    "bores": {"bore"},
    "well_top": {"well_top"},
    "well_tops": {"well_top"},
    "tops": {"well_top"},
    "source_top": {"source_top"},
    "source_tops": {"source_top"},
    "template": {"template"},
    "templates": {"template"},
    "asset": {"asset"},
    "assets": {"asset"},
}
ROOTS = (
    ("surface", "Surfaces"),
    ("point", "Points"),
    ("polygon", "Polygons"),
    ("bore", "Wells"),
    ("well_top", "Well tops"),
    ("source_top", "Imported top tables"),
    ("template", "Templates"),
    ("asset", "Assets"),
)


def segment(value: str) -> str:
    return quote(value, safe="-._~")


def typed_id(role: str, path: tuple[str, ...] | list[str]) -> str:
    return role + ":" + "/".join(segment(part) for part in path)


def bore_id(well: str, bore: str) -> str:
    # NUL cannot be a project name, so its percent form is an unambiguous
    # stable sentinel for the otherwise empty main-bore label.
    return typed_id("well", (well,)) + "/bore:" + (segment(bore) if bore else "%00")


def label(value: str) -> str:
    text = value.replace("_", " ").strip()
    return text[:1].upper() + text[1:] if text else value


@dataclass
class Entry:
    id: str
    label: str
    role: str
    path: tuple[str, ...]
    source: tuple[str, ...]
    views: dict[str, dict[str, Any]]
    visible: dict[str, bool]
    lane_attrs: dict[str, str | None] = field(default_factory=dict)
    disabled: bool = False
    reason: str | None = None
    diagnostic: dict[str, Any] | None = None

    def leaf(self) -> dict[str, Any]:
        value: dict[str, Any] = {
            "id": self.id,
            "label": self.label,
            "role": self.role,
            "views": copy.deepcopy(self.views),
            "visible": dict(self.visible),
        }
        if self.disabled:
            value.update(
                disabled=True, reason=self.reason, diagnostic=self.diagnostic
            )
        return value


@dataclass
class Folder:
    id: str
    label: str
    children: list[Any] = field(default_factory=list)
    folders: dict[str, "Folder"] = field(default_factory=dict)

    def insert(self, entry: Entry) -> None:
        here = self
        for index, part in enumerate(entry.path[:-1]):
            child = here.folders.get(part)
            if child is None:
                child = Folder(
                    typed_id("folder", (entry.role, *entry.path[: index + 1])), part
                )
                here.folders[part] = child
                here.children.append(child)
            here = child
        here.children.append(entry)

    def to_dict(self) -> dict[str, Any]:
        children = [
            child.to_dict() if isinstance(child, Folder) else child.leaf()
            for child in self.children
        ]
        return {"id": self.id, "label": self.label, "children": children}


__all__ = [
    "Entry",
    "Folder",
    "ROLE_ALIASES",
    "ROOTS",
    "VIEWS",
    "bore_id",
    "label",
    "typed_id",
]
