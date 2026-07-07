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
attributes), or RMS/IRAP plain `X Y Z`.

```python
pts = geo.load_points("picks", "picks.geojson")
pts.bbox
geom = pts.infer_geometry(edge="convex_hull")  # strict; raises if points are not grid-like
grid = pts.to_surface(grid_geom)               # or grid scattered points onto an explicit model grid
```

Use `infer_geometry()` only when the points are expected to be a regular grid
export. For genuinely scattered picks or irregular vendor exports, choose the
model/template `GridGeometry` explicitly and call `to_surface(...)`.

## Polygons

A `PolygonSet` is one or more rings. Load GeoJSON or CPS-3 lines; use it to clip.

```python
poly = geo.load_polygons("outline", "outline.geojson")
poly.rings                          # the constituent rings
inside = pts.clip(poly)             # keep points inside the polygon
```

See the [API reference](../api/reference.md) for the full surface/points/polygon
surfaces.
