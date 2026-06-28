"""petekIO — the subsurface data ingestion + structure layer.

A thin Python surface over the Rust `petekio` library: surfaces (operators,
attributes, statistics, volumetrics), wells/logs/tops (the dynamic
`w.brent.ntg` access chain), points/polygons, and the `GeoData` project with a
broadcastable wells view. See https://github.com/kkollsga/petekio.
"""

from ._petekio import (
    BBox,
    GeoData,
    GridGeometry,
    PointSet,
    PolygonSet,
    Stats,
    Surface,
    __version__,
)

__all__ = [
    "BBox",
    "GeoData",
    "GridGeometry",
    "PointSet",
    "PolygonSet",
    "Stats",
    "Surface",
    "__version__",
]
