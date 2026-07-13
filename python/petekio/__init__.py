"""petekIO — the subsurface data ingestion + structure layer.

A thin Python surface over the Rust `petekio` library: surfaces (operators,
attributes, statistics, volumetrics), wells/logs/tops (the dynamic
`w.brent.ntg` access chain), points/polygons, and the `GeoData` project with a
broadcastable wells view. See https://github.com/kkollsga/petekio.
"""

from ._petekio import (
    BBox,
    FormatKind,
    GeoData,
    GridGeometry,
    IngestSpec,
    Interval,
    LogView,
    MeshShell,
    NetSettings,
    PointColumn,
    PointSet,
    PolygonColumn,
    PolygonSet,
    Sidetrack,
    Stats,
    StructuredMeshSurface,
    StructuredShell,
    Surface,
    SurfaceIntersection,
    Trajectory,
    TriSurface,
    Well,
    WellIntersectionSet,
    WellsView,
    __version__,
    canonical_mnemonic,
    detect,
)

# The standalone WellLogBundle producer (well.view() / a logs-only session). The
# Rust view() trampolines call into `_viewer`; these re-exports let a caller build
# or inspect the bundle directly. petektools.viewer is an OPTIONAL runtime
# dependency, lazily imported by LogSession.serve/save.
from ._viewer import (
    LogSession,
    build_well_log_bundle,
    encode_lane,
)
from ._project import BoundTemplate, ImportSettings, Project, WellTopSet
from ._logs import Logs, LogChannel, LogPredicate

# The pure-Python view spec value-objects (WHAT / HOW for well.view()). The
# reservoir cutoffs (NetSettings) + load-time IngestSpec live on the Rust side.
from ._specs import (
    ViewSettings,
    ViewSpec,
)

__all__ = [
    "BBox",
    "BoundTemplate",
    "FormatKind",
    "GeoData",
    "GridGeometry",
    "IngestSpec",
    "Interval",
    "LogChannel",
    "LogPredicate",
    "LogSession",
    "LogView",
    "Logs",
    "MeshShell",
    "ImportSettings",
    "NetSettings",
    "PointColumn",
    "PointSet",
    "PolygonColumn",
    "PolygonSet",
    "Project",
    "Sidetrack",
    "Stats",
    "StructuredMeshSurface",
    "StructuredShell",
    "Surface",
    "SurfaceIntersection",
    "Trajectory",
    "TriSurface",
    "ViewSettings",
    "ViewSpec",
    "Well",
    "WellIntersectionSet",
    "WellTopSet",
    "WellsView",
    "__version__",
    "build_well_log_bundle",
    "canonical_mnemonic",
    "detect",
    "encode_lane",
]
