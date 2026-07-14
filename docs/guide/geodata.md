# GeoData — the project substrate

`GeoData` is petekIO's load-once project: named surfaces, wells, points, and
polygons under one declared length unit. You load everything into it once, then
read from it — operations broadcast across the collection rather than looping
item by item.

```python
import petekio

geo = petekio.GeoData(unit="m")

geo.load_surface("top_res", "surfaces/top_res.irap")
geo.load_structured_surface("faulted_top", "surfaces/faulted_top.EarthVisionGrid")
geo.load_well("15/9-A1", files="wells/15_9-A1/")
geo.load_well_tops("WellTops.tops")
geo.load_points("picks", "picks.geojson")
geo.load_polygons("outline", "outline.geojson")
```

## Named access

Each collection is reachable by name:

```python
top = geo.surface("top_res")      # project-backed Surface view
faulted = geo.surface("faulted_top")  # project-backed StructuredMeshSurface view
w   = geo.well("15/9-A1")         # a view into the project
pts = geo.points("picks")
```

Surfaces, wells, points, and polygons are lightweight **views** that re-resolve
into the project's collections by name. Regular and structured surfaces share
one Python namespace and name-uniqueness domain.

## Broadcasting across wells

`geo.wells` is a broadcastable, filterable view over every well:

```python
wells = geo.wells
len(wells)
deep = wells.filter(lambda w: (w.sidetrack("A").md_range() or (0, 0))[1] > 2500)

# After narrowing to a top, an attribute access resolves a log to per-well Stats:
ntg = wells.tops("Top A").ntg     # list[Stats], one per well carrying that top
```

Views are **read-only filtered subsets** — narrowing never mutates the project.

## Immutability

Operations return *new* objects; mutation is explicit (`set_*`). A resample or
arithmetic op on a surface yields a new surface; the project's stored objects are
unchanged unless you replace them.

See the [API reference](../api/reference.md#geodata) for the full `GeoData`
surface.
