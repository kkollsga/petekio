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

## Points

A `PointSet` is scattered `(x, y, z)` with optional per-point attributes. Load
GeoJSON, CSV (with `x`/`y`/`z` columns; other numeric columns become
attributes), EarthVision/Petrel point grids, or RMS/IRAP plain `X Y Z`.

```python
pts = geo.load_points("picks", "picks.geojson")
pts.bbox
geom = pts.infer_geometry(edge="convex_hull")  # strict; raises if points are not grid-like
grid = pts.to_surface(grid_geom)               # or grid scattered points onto an explicit model grid
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

Use `infer_geometry()` only when the points are expected to be a regular grid
export. EarthVision/Petrel exports that carry `column` and `row` fields use that
topology directly. During `Project.import_data(...)`, same-stem Petrel IRAP point
exports are enriched from matching EarthVision topology files when both are
present. Standalone plain IRAP/XYZ point exports must infer from XY only, so
they cannot recover exact grid topology once those fields are lost. For
genuinely scattered picks or irregular vendor exports, choose the model/template
`GridGeometry` explicitly and call `to_surface(...)`.

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
