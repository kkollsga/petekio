# Projects & persistence

A `GeoData` project saves to a single **`.pproj`** file — an efficient, structured
container (not human-readable) that is easy to **split, merge, and tag-filter**
for sharing with teammates.

```python
geo.set_owner("kk")
geo.set_tags(["cerisa", "gate-0"])          # project-level tags
geo.save("field.pproj")                     # atomic whole-project write

geo = petekio.GeoData.open("field.pproj")   # materialize the project
```

## What's inside

One file: a magic + version header, a **JSON manifest** (owner, tags, unit,
timestamps, and a section index), then one `zstd`-compressed section per
element. The manifest is self-describing, so you can list a project without
loading any data:

```python
info = petekio.GeoData.inspect("field.pproj")   # a dict — no element decode
info["owner"], info["tags"], info["unit"]
info["elements"]                                 # [(kind, name), ...]
```

## Share a subset — split / merge / tag-filter

Sections are position-independent, so these copy blobs **byte-for-byte** (no
re-encode):

```python
geo.set_element_tags("15/9-A1", ["cerisa"])          # tag an element
geo.save("field.pproj")

petekio.GeoData.export("field.pproj", "share.pproj", ["cerisa"])  # tagged subset → one file
petekio.GeoData.split("field.pproj", "wells.pproj", ["15/9-A1"])  # by element name
petekio.GeoData.merge("a.pproj", "b.pproj", "both.pproj")         # union (b wins on clash)
```

## Per-element files & human-readable export

Each element can be saved on its own, and exported to an interchange format:

```python
# (Rust) surface.save("top.pproj"); Surface::load("top.pproj")
points.export_geojson("picks.geojson")   # or export_csv(...)
polygons.export_geojson("outline.geojson")
```

## Model sidecar (for downstream tools)

A modelling tool can store its model into the same project as opaque
`model/*` sections — bytes petekIO frames and hands back untouched, each with
its own version. petekIO never parses them, and they survive split/merge/export
byte-for-byte:

```python
geo.put_model_section("model/cerisa/props", ["cerisa"], 1, payload_bytes)
version, data = geo.model_section("model/cerisa/props")
geo.model_section_names()
```

## Versioning

Two tiers: a hard **format** version (a newer file is refused with a clear
message) and a soft **`data_version`** for the element schema (older files route
through a migration; the manifest itself evolves via additive fields). The
`.pproj` format version is independent of petekIO's crate version.
