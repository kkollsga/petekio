"""Compact regular-surface resources and domain-owned well display clipping."""

from __future__ import annotations

import base64
import copy
import hashlib
import math
from collections.abc import Callable, Mapping
from typing import TYPE_CHECKING, Any

from ._project_view_catalog import Entry

if TYPE_CHECKING:
    from ._project import Project


class SurfaceViewResources:
    """Snapshot-local caches and wire adapters for project surface views."""

    def __init__(self, project: "Project", viewer: Callable[[], Any]) -> None:
        self.project = project
        self.viewer = viewer
        self.entries: dict[str, Entry] = {}
        self.diagnostics: list[dict[str, Any]] = []
        self.trajectory_cache: dict[str, list[tuple[float, list[float]]]] = {}
        self.overlay_cache: dict[tuple[str, str], dict[str, Any]] = {}

    def reset(
        self, entries: dict[str, Entry], diagnostics: list[dict[str, Any]]
    ) -> None:
        self.entries = entries
        self.diagnostics = diagnostics
        self.trajectory_cache.clear()
        self.overlay_cache.clear()

    @staticmethod
    def preview_stride(obj: Any, target_nodes: int = 160_000) -> int:
        count = max(1, int(obj.ncol) * int(obj.nrow))
        return max(1, math.ceil(math.sqrt(count / target_nodes)))

    @staticmethod
    def _block(raw: bytes, dtype: str, shape: list[int]) -> dict[str, Any]:
        return {
            "dtype": dtype,
            "shape": shape,
            "data": base64.b64encode(raw).decode("ascii"),
        }

    def shared_regular(
        self,
        entry: Entry,
        grid: Mapping[str, Any],
        descriptors: list[dict[str, Any]],
        detail: str | None,
    ) -> dict[str, Any]:
        ncol, nrow = map(int, grid["dimensions"])
        blocks: dict[str, dict[str, Any]] = {}

        def add_block(raw: bytes, dtype: str) -> dict[str, str]:
            digest = hashlib.sha256(raw).hexdigest()
            descriptor = self._block(raw, dtype, [ncol * nrow])
            existing = blocks.get(digest)
            if existing is not None and existing != descriptor:
                raise ValueError("shared surface block digest has conflicting types")
            blocks[digest] = descriptor
            return {"__block__": digest}

        mask = add_block(bytes(grid["mask"]), "u8")
        lanes = list(grid["lanes"])
        if len(lanes) != len(descriptors):
            raise ValueError("shared surface transport omitted a declared attribute")
        attributes = []
        for descriptor, lane in zip(descriptors, lanes):
            record = copy.deepcopy(descriptor)
            record["values"] = add_block(bytes(lane["values"]), "f32")
            record["range"] = (
                None
                if record.get("kind") == "categorical"
                else copy.deepcopy(lane.get("range"))
            )
            attributes.append(record)

        step_i = tuple(map(float, grid["step_i"]))
        step_j = tuple(map(float, grid["step_j"]))
        spacing_x = math.hypot(*step_i)
        spacing_y = math.hypot(*step_j)
        rotation = math.degrees(math.atan2(step_i[1], step_i[0]))
        yflip = step_i[0] * step_j[1] - step_i[1] * step_j[0] < 0.0
        frame = {
            "origin_x": float(grid["origin"][0]),
            "origin_y": float(grid["origin"][1]),
            "spacing_x": spacing_x,
            "spacing_y": spacing_y,
            "ncol": ncol,
            "nrow": nrow,
            "rotation_deg": rotation,
            "yflip": yflip,
            "crs": getattr(self.project, "crs", None),
            "units": getattr(self.project, "unit", None),
        }
        resource: dict[str, Any] = {
            "schema_version": 2,
            "kind": "workspace_resource",
            "item_id": entry.id,
            "view": "map",
            "blocks": blocks,
            "payload": {
                "schema_version": 4,
                "map": {
                    "surface_grid": {
                        "schema_version": 1,
                        "item_id": entry.id,
                        "frame": frame,
                        "positive": "up",
                        "mask": mask,
                        "attributes": attributes,
                        "triangle_count": int(grid["triangle_count"]),
                    }
                },
            },
        }
        if detail is not None:
            resource["detail"] = detail
        return resource

    def regular_map(self, entry: Entry, grid: Mapping[str, Any]) -> dict[str, Any]:
        payload = self.viewer().view2d_payload([], title=entry.label, lod=False)
        ncol, nrow = map(int, grid["dimensions"])
        values_raw = bytes(grid["values"])
        mask_raw = bytes(grid["value_mask"])
        values_digest = hashlib.sha256(values_raw).hexdigest()
        mask_digest = hashlib.sha256(mask_raw).hexdigest()
        regular_grid = {
            "dimensions": [ncol, nrow],
            "origin": list(grid["origin"]),
            "step_i": list(grid["step_i"]),
            "step_j": list(grid["step_j"]),
            "values": {"__block__": values_digest},
            "mask": {"__block__": mask_digest},
        }
        fill = {
            "name": str(grid["name"]),
            "display_name": entry.label,
            "range": list(grid["range"]),
            "regular_grid": regular_grid,
            "item_id": entry.id,
        }
        corners = [
            [
                regular_grid["origin"][axis]
                + i * regular_grid["step_i"][axis]
                + j * regular_grid["step_j"][axis]
                for axis in (0, 1)
            ]
            for i, j in (
                (0, 0),
                (ncol - 1, 0),
                (0, nrow - 1),
                (ncol - 1, nrow - 1),
            )
        ]
        xs = [point[0] for point in corners]
        ys = [point[1] for point in corners]
        map_bundle = payload["map"]
        map_bundle["frame"] = {
            "origin_x": min(xs),
            "origin_y": min(ys),
            "spacing_x": max(max(xs) - min(xs), 1.0),
            "spacing_y": max(max(ys) - min(ys), 1.0),
            "ncol": 2,
            "nrow": 2,
        }
        map_bundle["fills"] = [fill]
        map_bundle["items"] = [{"id": entry.id, "fill_range": [0, 1]}]
        map_bundle["blocks"] = {
            values_digest: self._block(values_raw, "f32", [ncol * nrow]),
            mask_digest: self._block(mask_raw, "u8", [ncol * nrow]),
        }
        payload["summary"].update(fills=1, triangles=int(grid["triangle_count"]))
        return payload

    def regular_scene(
        self, entry: Entry, grid: Mapping[str, Any], detail: str | None
    ) -> dict[str, Any]:
        payload = self.viewer().view3d_payload([], title=entry.label)
        ncol, nrow = map(int, grid["dimensions"])
        elevations_raw = bytes(grid["elevations"])
        values_raw = bytes(grid["values"])
        mask_raw = bytes(grid["elevation_mask"])
        mesh = {
            "name": str(grid["name"]),
            "display_name": entry.label,
            "range": list(grid["range"]),
            "values": None,
            "item_id": entry.id,
            "regular_surface": {
                "dimensions": [ncol, nrow],
                "origin": list(grid["origin"]),
                "step_i": list(grid["step_i"]),
                "step_j": list(grid["step_j"]),
                "elevations": self._block(elevations_raw, "f32", [ncol * nrow]),
                "mask": self._block(mask_raw, "u8", [ncol * nrow]),
                "values": self._block(values_raw, "f32", [ncol * nrow]),
                "elevation_range": list(grid["elevation_range"]),
                "triangle_count": int(grid["triangle_count"]),
            },
        }
        scene = payload["scene3d"]
        scene["meshes"] = [mesh]
        scene["layers"] = [
            {"kind": "surface", "name": entry.label, "item_id": entry.id}
        ]
        scene["ref_z"] = max(grid["elevation_range"])
        scene["detail"] = detail or "full"
        if int(grid["stride"]) > 1:
            scene["preview_stride"] = int(grid["stride"])
        payload["summary"].update(meshes=1, triangles=int(grid["triangle_count"]))
        return payload

    def attach_well_overlays(
        self, context: Entry, surface: Any, payload: dict[str, Any]
    ) -> None:
        overlays = []
        for bore_entry in self.entries.values():
            if bore_entry.role != "bore" or bore_entry.disabled:
                continue
            key = (context.id, bore_entry.id)
            overlay = self.overlay_cache.get(key)
            if overlay is None:
                overlay = self._surface_well_overlay(context, bore_entry, surface)
                self.overlay_cache[key] = overlay
            overlays.append(copy.deepcopy(overlay))
        if overlays:
            payload["map"]["well_overlays"] = overlays

    def _surface_well_overlay(
        self, context: Entry, bore_entry: Entry, surface: Any
    ) -> dict[str, Any]:
        well_id, bore = bore_entry.source
        well = self.project.well(well_id)
        sidetrack = None if well is None else well.sidetrack(bore)
        base = self.trajectory_samples(bore_entry.id, sidetrack)
        full_path = [point for _, point in base]
        overlay: dict[str, Any] = {
            "context_item_id": context.id,
            "well_item_id": bore_entry.id,
            "trajectory": full_path,
            "intersection": None,
            "intersections": [],
            "status": "no_hit",
        }
        if sidetrack is None:
            overlay.update(status="error", message="bore is no longer available")
            return overlay
        if not full_path:
            overlay.update(status="error", message="bore has no positioned trajectory")
            return overlay
        try:
            hits = list(sidetrack.intersections(surface))
        except Exception as exc:
            message = f"surface intersection failed: {exc}"
            overlay.update(status="error", message=message)
            self.diagnostics.append(
                {
                    "code": "well_overlay_error",
                    "severity": "warning",
                    "item_id": bore_entry.id,
                    "context_item_id": context.id,
                    "error": type(exc).__name__,
                    "message": message,
                }
            )
            return overlay
        records = []
        for hit in hits:
            try:
                md = float(hit.md)
                xyz = [float(value) for value in hit.xyz]
            except (TypeError, ValueError, OverflowError):
                continue
            if len(xyz) != 3 or not math.isfinite(md) or not all(
                math.isfinite(value) for value in xyz
            ):
                continue
            records.append({"md": md, "xyz": xyz})
        records.sort(key=lambda record: record["md"])
        if not records:
            return overlay
        selected = records[-1]
        md = selected["md"]
        xyz = selected["xyz"]
        clipped = [point for sample_md, point in base if sample_md < md]
        if not clipped or clipped[-1] != xyz:
            clipped.append(xyz)
        overlay.update(
            status="hit" if len(records) == 1 else "ambiguous",
            trajectory=clipped,
            intersection=copy.deepcopy(selected),
            intersections=records,
        )
        if len(records) > 1:
            overlay["message"] = (
                f"{len(records)} crossings; display path ends at the greatest-MD hit"
            )
        return overlay

    def trajectory_samples(
        self, item_id: str, sidetrack: Any
    ) -> list[tuple[float, list[float]]]:
        cached = self.trajectory_cache.get(item_id)
        if cached is not None:
            return cached
        rows: list[tuple[float, list[float]]] = []
        if sidetrack is not None:
            span = sidetrack.md_range()
            if span is not None:
                lo, hi = map(float, span)
                count = 1 if hi == lo else 128
                for index in range(count):
                    md = lo if count == 1 else lo + (hi - lo) * index / (count - 1)
                    point = sidetrack.xyz(md)
                    if point is not None:
                        rows.append((md, [float(value) for value in point]))
        self.trajectory_cache[item_id] = rows
        return rows


__all__ = ["SurfaceViewResources"]
