# Surfaces, points & polygons

## Surfaces

A `Surface` is a regular grid with a `GridGeometry` (origin, increments, rows ×
cols, rotation). Load the IRAP-classic ASCII format, then sample, resample, do
arithmetic, statistics, and volumetrics.

```python
top = geo.load_surface("top_res", "surfaces/top_res.irap")

top.geometry                 # GridGeometry (xori/yori, xinc/yinc, ncol/nrow, ...)
top.edge                     # PolygonSet around the defined surface nodes
top.geometry.edge            # same edge carried by the returned geometry
top.bbox                     # BBox of the grid
top.sample(x, y)             # bilinear sample at a world coordinate
top.stats.mean               # NaN-skipping statistics
top.area_below(2400)         # planimetric area below a depth (volumetrics)
```

`f64::NAN` marks undefined cells: arithmetic propagates NaN, statistics skip it.

### Operators and resampling

```python
diff = top - other_surface       # elementwise (geometries must match)
scaled = top * 1.05              # scalar ops
ongrid = top.resample(grid_geom) # bilinear onto another GridGeometry
```

Operations return **new** surfaces; the stored surface is unchanged.

### Attributes, interpretation, and hole repair

```python
thick = top.thickness(base, clamp_zero=True)  # base - top; unbound form also works
top.thickness = thick                         # typed attribute assignment
top.attr["thickness"]                         # promote the lane as a Surface

smoothed = top.smooth(radius=1)               # preserves the original NaN mask
dip = top.dip_angle()                         # degrees from horizontal
azimuth = top.dip_azimuth()                   # down-dip clockwise from North
filled = top.extrapolate("nearest")            # also "idw" / "min_curvature"
```

An assigned lane must be another `Surface` with identical complete geometry.
Assignment is copy-on-write for project views and does not shadow the instance
`top.thickness(base)` method. Interpretation and extrapolation return detached,
same-geometry surfaces containing only the derived primary lane. Extrapolation
changes only original NaNs and uses finite nodes as controls.

## Points

A `PointSet` is scattered `(x, y, z)` with optional per-point attributes. Load
GeoJSON, CSV (with `x`/`y`/`z` columns; other numeric columns become
attributes), the deprecated finite-node view of EarthVision/Petrel grids, or
RMS/IRAP plain `X Y Z`.

```python
pts = geo.load_points("picks", "picks.geojson")
pts.bbox
geom = pts.infer_geometry()                    # GridGeometry | StructuredShell | MeshShell
grid = pts.to_surface(grid_geom)               # or grid scattered points onto an explicit model grid
mesh = pts.to_structured_surface()             # topology-bearing points, explicit shifted XY nodes
```

### Point attribute calculations

Point sets behave like spatial tables. The container itself does not do
object-to-object arithmetic: `points_a + points_b` is intentionally unsupported.
Instead, calculate on coordinate/attribute columns inside one point set and
assign the result back as a named attribute.

```python
pts.z_shifted = pts.z + 2.0
pts.depth_plus_y = pts.z + pts.y
pts.phie_net = pts.PHIE * pts.NetSand

pts.attr("phie_net")       # list[float]
pts.attr_names()           # numeric attribute columns
```

Use an explicit future attach/resample operation to bring values from another
point set onto this one before doing column math. That keeps matching tolerance,
nearest-neighbour/interpolation policy, and missing-data handling visible.

Use `infer_geometry()` only when the points are expected to be a truly regular
affine grid. EarthVision/Petrel exports that carry `column` and `row` fields can
also be promoted with `to_structured_surface(...)`; that keeps the logical
row/column topology while preserving each node's actual XY coordinate. This is
the right home for Petrel surfaces that are locally shifted around faults.
A mesh whose nodes do not sit on any regular lattice — varying cell size, a cell
angle away from 90° — is **curvilinear**. `infer_geometry(...)` refuses to invent a
regular lattice and returns geometry only: `StructuredShell` when explicit
topology validates, otherwise a fault-aware `MeshShell`. The MeshShell fallback
bridges short open fringes and seams up to `3.4` cells by default; pass
`max_bridge=None` for strict lattice-closed triangulation. `fallback="error"`
raises; deprecated `fallback="tri"` aliases the default `"mesh"` spelling.
Direct `to_tri_surface()` remains strict and value-bearing; similarly,
`to_structured_surface(...)` explicitly attaches values to the exact row/column
representation.

`edge="full_rect"` is the default point footprint: the four corners of the
inferred lattice. It over-claims whenever the data does not fill its bounding
lattice. `edge="occupied"` is the true footprint — the outline of the nodes that
carry data, following interior holes and a non-rectangular boundary — and is the
default for `to_structured_surface(...)`. `edge="convex_hull"` is intentionally
broader for envelope/QC comparison.

During `Project.import_data(...)`, an EarthVision grid is loaded canonically as
a `StructuredMeshSurface` under `project.surfaces`; null nodes retain XY and
become `NaN` values. Same-stem Petrel IRAP point exports remain separate point
sets and are enriched from the EarthVision topology. Both retain stable,
path-qualified names when they coexist.
Standalone plain IRAP/XYZ point exports must infer from XY only, so they cannot
recover exact grid topology once those fields are lost. For genuinely scattered
picks or irregular vendor exports, choose the model/template `GridGeometry`
explicitly and call `to_surface(...)`.

`StructuredMeshSurface` is intentionally not a regular `Surface`: it has
`kind == "structured_mesh"`, `ncol`, `nrow`, `node_xy(i, j)`, `z(i, j)`,
`values()`, `edge`, `nominal_geometry`, `bbox()`, `stats()`, and `history()`.
Use `nominal_geometry` only as approximate metadata; the explicit node XY arrays
are canonical.

```python
mesh = petekio.StructuredMeshSurface.load_earthvision_grid("top.EarthVisionGrid")
mesh = geo.load_structured_surface("top", "top.EarthVisionGrid")
```

## Polygons

A `PolygonSet` is one or more rings. Load GeoJSON or CPS-3 lines; use it to clip.

```python
poly = geo.load_polygons("outline", "outline.geojson")
poly.rings                          # the constituent rings
inside = pts.clip(poly)             # keep points inside the polygon
```

### Polygon attribute calculations

Polygons follow the same rule: no `polygons_a + polygons_b`. Calculate inside one
set, using per-polygon numeric attributes and the derived per-polygon area
column.

```python
poly.ntg = [0.65, 0.72]          # one value per polygon
poly.net_area = poly.area * poly.ntg

poly.area.values()               # per-polygon areas
poly.area()                      # compatibility: total area
poly.total_area()                # explicit total area
```

See the [API reference](../api/reference.md) for the full surface/points/polygon
surfaces.
