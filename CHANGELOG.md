# Changelog

All notable changes to petekIO are recorded here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/); the project uses SemVer. The
`release` skill promotes `[Unreleased]` to a versioned block at release time.

## [Unreleased]

### Fixed
- **Small scattered point clouds retain their triangulated boundary.** Mesh
  cleanup now treats the eight-triangle island threshold as a relative speck
  filter, not a minimum surface size, so complete four- and five-point
  triangulations return `MeshShell` geometry instead of failing with a
  misleading `triangulated surface has no boundary` error. Truly degenerate
  clouds still fail loudly, and bridge/component/label behaviour is unchanged.

### Added
- **`PointSet.infer_geometry()` now returns geometry-only roles.** Regular
  points return `GridGeometry`; validated topology-bearing curvilinear points
  return `StructuredShell`; triangulated/faulted/scattered fallback returns
  `MeshShell`. Shell wrappers expose stable `kind` values and propagate project
  names as `"<name> geometry"`, while retaining components, edge, labels, and
  wireframe access. The mesh path keeps the 3.4-cell default bridge, explicit
  `None` strictness, loud warning, and `fallback="error"`. The documented
  default is now `fallback="mesh"`; legacy `fallback="tri"` remains accepted
  with a `DeprecationWarning` but no longer implies a value-bearing result.
  Values remain on the `PointSet` until explicit `to_surface`,
  `to_structured_surface`, or `to_tri_surface` conversion.
- **EarthVision grids are first-class structured project surfaces.**
  `StructuredMeshSurface.load_earthvision_grid(...)` and
  `GeoData.load_structured_surface(...)` retain every logical node, including
  null-z nodes as `NaN` with their finite XY and row/column topology. Raw
  `Project.import_data(...)` routes EarthVision grids into the unified surface
  namespace while same-stem IRAP point exports remain topology-enriched point
  sets with stable path-qualified names. Structured surfaces now round-trip in
  whole-project `.pproj` files using the existing `structured_mesh` kind, with
  no data-version bump. `PointSet.load_earthvision_grid(...)` remains as a
  deprecated finite-node compatibility view. `model_inputs()` errors rather
  than silently dropping structured horizons it cannot yet represent.

## [0.3.13] - 2026-07-13

### Added
- **Surface interpretation and hole repair.** Python `Surface` now exposes
  `smooth(radius=1)`, `dip_angle()`, `dip_azimuth()`, and
  `extrapolate(method="nearest")` (`idw` / `min_curvature` also supported).
  Dip is derived with NaN-aware central/one-sided differences transformed into
  the rotated/y-flipped world frame; flat azimuth is undefined. Extrapolation
  delegates to the shared petekTools kernels, fills only original NaNs, excludes
  infinities from controls, and preserves every original non-NaN bit. Each
  operation returns a detached, same-geometry, primary-only surface with
  appended history.
- **Typed Python `Surface` attribute assignment.** `surface.thickness = rhs`
  now delegates to `surface.set_attr("thickness", rhs)` and adds or replaces a
  copy-on-write attribute lane, readable through `surface.attr["thickness"]`.
  The right-hand side must be a `Surface` with identical complete grid geometry;
  same-shaped grids with different origins, increments, rotations, or y-flip
  are rejected. Instance/unbound operations such as `surface.thickness(...)`
  and `Surface.thickness(...)` remain callable.

### Fixed
- **Python `Surface.thickness` instance ergonomics.** Both
  `top.thickness(base, clamp_zero=True)` and the equivalent unbound
  `Surface.thickness(top, base, clamp_zero=True)` now work. Assigning a
  `thickness` attribute lane remains supported and does not shadow either call
  form; geometry validation, clamp defaults, and operation history are unchanged.
- **`infer_geometry()` now closes ordinary TriSurface fallback fringes and seams.**
  The Python fallback defaults `max_bridge` to 3.4 cells, avoiding fragmented,
  ragged boundaries when a point export does not quite close on its recovered
  lattice. Pass `max_bridge=None` for the previous strict lattice-closed result.
  Direct `to_tri_surface()` remains strict by default.

## [0.3.12] - 2026-07-10

### Added
- **Producer-side LOD for the viewer seam (display-only; geometry never
  decimated).** Three additive extensions compute exact level-of-detail
  reductions from the shell's own structure:
  - `MeshShell::wireframe_edges(stride)` / `TriSurface::wireframe_edges(stride)`
    (Python `wireframe_edges(stride=None)`): `stride=k` returns the coarse
    lattice wireframe — per fault block only the every-k-th row/column grid
    lines survive (an in-block edge `(i,j)–(i+1,j)` kept when `j % k == 0`, an
    edge `(i,j)–(i,j+1)` when `i % k == 0`), plus **all** boundary edges and
    **all** edges touching an unlabelled node (fringe/bridge) or crossing a
    block seam — so the outline and every seam stay intact at every LOD. It
    reproduces the same line set as striding a grid's lines. `None`/`1` is the
    full wireframe, byte-identical to before. Measured ≈2×/≈3.9× fewer edges at
    stride 2/4 on a 200×200 faulted lattice.
  - `Surface`/`StructuredMeshSurface`/`TriSurface` `value_layer(attr, stride)`
    (Python `value_layer(attr=None, stride=None)`): `stride=k` builds the
    trimesh on the decimated node set — per block, the `i % k == 0 && j % k == 0`
    nodes, re-triangulated as the coarse quad-split (two triangles per coarse
    cell where all four corners exist). Node values are the nodes' own values
    (no averaging — a display LOD, not a resample); `range` comes from the
    **full-resolution** lane so colours stay stable across LODs. Unlabelled
    fringe/bridge nodes are dropped at coarse LOD (re-attaching them needs a
    full re-triangulation — disproportionate for a display reduction; the
    outline is carried by the wireframe/edge). The dict shape is unchanged.
    Measured ≈4×/≈16.8× fewer triangles at stride 2/4 (2-D decimation).
  - `iso_lines(..., simplify)` (all three levels; Python
    `iso_lines(..., simplify=None)`): `simplify=tol` runs Douglas–Peucker
    (new pure kernel `algorithms::surfaces::douglas_peucker`) on each output
    polyline with a world-unit tolerance. Open-line endpoints and ring-closure
    points are preserved; a polyline never drops below 2 points (rings below 4).
    Measured ≈7×/≈39× fewer contour vertices at tol 1 m/50 m.
- **Dataset names on Python hand-backs (duck-typed viewer seam).** Objects
  resolved through a project lookup (`project.points["…/Top Dome"]`,
  `project.surfaces[...]`, `geo.points(name)`, …) now carry a read-only
  `.name` property — the lookup key's leaf, e.g. `"Top Dome"` — so downstream
  consumers (the petektools viewer legend) can show the real dataset name.
  Derived objects propagate it (`infer_geometry` → `"Top Dome geometry"`;
  `to_surface`/`to_tri_surface`/`to_structured_surface`/`to_points`/`resample`
  keep `"Top Dome"`). Anonymous/in-memory objects return `None`.
- **Discoverable `kind` labels.** `GridGeometry` (`"grid_geometry"`),
  `Surface` (`"surface"`), `PointSet` (`"point_set"`) and `PolygonSet`
  (`"polygon_set"`) now expose a `.kind` property (matching the existing
  `TriSurface`/`StructuredMeshSurface` labels), so `infer_geometry` callers
  can type-dispatch its `GridGeometry | TriSurface` result without imports.
- **`infer_geometry(fallback=...)`.** `fallback="tri"` (default) keeps the
  TriSurface fallback; `fallback="error"` raises a `ValueError` when no
  regular lattice fits the points.
- **Geometry-optional `PointSet.to_surface`.** Python
  `to_surface(geom=None, method="idw", tolerance=1e-3)` now infers the
  lattice internally when `geom` is omitted, raising a clear `ValueError`
  when the points are not lattice-regular (never gridding onto an arbitrary
  bounding lattice) and a clear `TypeError` when the `infer_geometry`
  TriSurface fallback (or any non-`GridGeometry`) is passed as `geom`.

### Fixed
- **`infer_geometry` no longer swallows the fallback's failure cause.** When
  the lattice fit fails *and* the TriSurface fallback also fails (e.g. a
  tiny/degenerate cloud), the raised `ValueError` now chains both causes
  ("no regular lattice fits … the TriSurface fallback also failed …")
  instead of reporting only the lattice-fit error.

### Changed
- **`infer_geometry`'s TriSurface fallback is now loud.** When the regular
  lattice fit fails, the default behaviour still returns the `TriSurface`
  fallback but emits a `UserWarning` naming the fit failure (silently
  swapping return types hid points-vs-surface confusion downstream).
  `max_bridge` is documented as applying only to that fallback.
  **Migration:** pass `fallback="error"` to make the failure fatal, or
  silence it with `warnings.filterwarnings("ignore", category=UserWarning)`
  if you relied on the silent fallback.

## [0.3.11] - 2026-07-10

> **Rust API note:** this release reshapes `TriSurface`/`StructuredMeshSurface`
> around shared geometry shells — `PointSet::to_tri_surface` gained a
> `max_bridge` parameter and `TriSurface::points()` now returns an owned
> `Vec<[f64; 3]>`. The **Python API is fully backward-compatible**. Rust
> consumers on `^0.3` should review before `cargo update` (petekstatic and
> peteksim do not use the changed symbols).

### Added
- **The three-level geometry-shell system.** Geometry is a flat empty shell —
  purely topological/positional, never a function of z — in three levels:
  the rigid `GridGeometry` (level 1, unchanged), the new **`StructuredShell`**
  (level 2: `(i, j)` nodes with explicit per-node XY, optional nominal
  geometry, edge polygon) and the new **`MeshShell`** (level 3: 2-D nodes,
  CCW triangles, quad-dominant wireframe, boundary edge, per-node walk
  labels). Shells are immutable and `Arc`-shared, so N properties/clones
  never repeat geometry in memory. Surfaces = shell + property lanes:
  `StructuredMeshSurface` and `TriSurface` now carry a primary value lane
  plus named **attribute lanes** (`attr`/`set_attr`/`attr_names`/
  `as_attr_surface`), mirroring `Surface` (Python `set_attr` on these two
  returns a new object).
- **Conversions across levels.** Upward is free and lossless, carrying every
  attribute 1:1 with node identity preserved: `Surface.to_structured_mesh()`,
  `Surface.to_tri_surface()`, `StructuredMeshSurface.to_tri_surface()` (and
  shell-level `GridGeometry::to_structured_shell()/to_mesh_shell()`,
  `StructuredShell::to_mesh_shell()`). Downward is a fit or a resample:
  `infer_grid(tolerance)` fits a regular `GridGeometry` (errors when the
  shell is not regular), and `resample(target, method)` grids the primary
  **and all attribute lanes** onto a target geometry through the shared
  gridding kernels.
- **Iso-lines** on all three surface levels:
  `iso_lines(interval=None, levels=None, attr=None)` — NaN-aware
  marching-triangles contour extraction with deterministic, exact
  (mesh-edge-anchored) segment chaining. Levels 1–2 quad-split each cell
  along a consistent diagonal; level 3 contours per shell triangle. Explicit
  `levels` win; `interval` aligns levels to its multiples across the value
  range. Holes break lines, never bend them.
- **Value layers**: `value_layer(attr=None)` returns the viewer's trimesh
  bundle — Python: `{"kind": "trimesh", "name", "nodes", "triangles",
  "values", "range"}` (range = finite min/max, NaN allowed in values) —
  consumed by the petektools viewer.
- **Walkability**: a lazily built corner table on `MeshShell`
  (opposite-corner + vertex→corner arrays; derived, never serialized).
  Construction asserts the mesh is edge-manifold (no undirected edge carried
  by more than two triangles).
- **`.pproj` persistence for level 2/3 surfaces**: `StructuredMeshSurface`
  and `TriSurface` gained `save`/`load` (new section kinds
  `structured_mesh` / `tri_surface`). Each section stores its shell **once**
  with N property lanes referencing it; existing `.pproj` files load
  unchanged.

- `TriSurface.wireframe_edges()` (Rust and Python): the unique triangle edges
  with interior cell diagonals removed — the quad-dominant wireframe of the
  geometry as a flat empty shell. Purely topological: a diagonal is classified
  from the topology walk's `(block, i, j)` labels and hidden only when both
  triangles of its cell survived; boundary diagonals stay, and z never enters
  the classification (how shape splits non-planar cells belongs to the surface
  layer). Consumers (e.g. the petektools 2-D viewer) can draw lattice cells as
  squares instead of triangle pairs.
- `max_bridge` (in cells) on `PointSet.to_tri_surface(max_link, max_bridge)`
  and Python `infer_geometry(..., max_bridge=None)` /
  `to_tri_surface(max_link=None, max_bridge=None)`: opt-in admits triangle
  edges the closed-lattice rules reject — the boundary fringe, fault seams,
  interior data gaps — up to that length, closing the mesh where the geometry
  does not close (smoother edges, one continuous sheet). `None` keeps the
  strict lattice-closed behaviour; must be `>= max_link` when set. The Rust
  `to_tri_surface` signature gained the second parameter.

### Changed
- Rust `TriSurface::points()` now returns `Vec<[f64; 3]>` (the shell's XY
  zipped with the primary z lane) instead of `&[[f64; 3]]`; behaviour and the
  Python surface (`points()`/`xyz()` tuples) are unchanged.

## [0.3.10] - 2026-07-10

### Fixed
- Python `PointSet.infer_geometry(...)` now returns the established unstructured
  `TriSurface` fallback when strict regular-lattice inference fails, so irregular
  and curvilinear point surfaces remain directly viewable. Successful regular
  inference still returns `GridGeometry`; the strict Rust inference API is unchanged.
  `TriSurface.xyz()` supplies the generic `view2d` point protocol.

### Changed
- The `petektools` Python package is now an optional `petekio[toolkit]` extra.
  The Rust extension still uses the petekTools crate; base Python installs keep
  the existing interpolation fallback, while viewer methods explain how to
  install the optional renderer.

### CI
- Continuous integration now builds the ABI3 wheel once and tests that artifact
  across Python 3.10–3.14, including separate base-install and optional-toolkit
  coverage. Release artifacts build alongside the unchanged Rust gates; trusted
  PyPI publishing retries transient failures safely and reports the time until
  the wheel is installable from the registry.

## [0.3.9] - 2026-07-09

### Added
- `PointSet.to_tri_surface(max_link=None)` + `TriSurface` — the triangulated fallback
  for a surface whose topology `detect_topology` cannot verify. The points are the
  vertices, unmoved; the result is one connected sheet with its boundary ring(s).
  Delaunay runs in the **normalized grid frame** (each axis divided by its own step),
  so an anisotropic cell becomes a unit square and one scalar `max_link` bounds both
  axes — in world units no such scalar exists, since past an aspect ratio of sqrt(2)
  the cell diagonal already exceeds two short-axis steps and the admissible band is
  empty. `max_link` is in cells and must lie in `(sqrt(2), 2)`: below the diagonal the
  mesh shreds, at two cells a triangle skips a node. Adjacencies the topology walk
  refused are excluded, and so is every edge between two different fault blocks — nodes
  the walk could not connect have no grid adjacency, which is what a fault *is*. Both
  blocks are kept (`TriSurface.components()` reports how many); dropping all but the
  largest would silently discard real data. On a reference fault-cut export this leaves
  2 bridging triangles of 76,262, against 84 for a bare length filter. Adds no
  dependency: `spade` was already in the tree via `geo`.
- `PointSet.detect_topology(nominal_cell=None)` — recovers `column`/`row` topology
  from bare `X Y Z` surface points **without moving a point**. It detects the grid
  azimuth from the modal nearest-neighbour step and a step **per axis** (a 50 x 25 m
  cell is ordinary), then walks the grid paths predictively, using locally measured
  step vectors so it follows curvature, shear and swell.
  Returns `(points, TopologyReport)`; the points are `None` unless the detection
  **verifies** — every distinct node labelled, no index claimed twice, no coincident
  pair with differing z. It deliberately cannot cross a fault: where nodes are snapped
  onto a fault trace or stretched across it, the neighbour relation is not determined
  by geometry, and forcing it welds fault blocks together silently. Instead the walk
  **re-seeds** wherever it stalls, labelling each fault block in its own index space:
  `report.blocks` is 1 for an uninterrupted grid and more when the surface is fault-cut,
  and `verified` requires exactly one. Spec: `surface_topology_walk_spec`.
- `StructuredMeshSurface.to_points()` — explodes the mesh back into a `PointSet` with
  its `column`/`row` topology, copying node XY/Z rather than resampling. It is the
  exact inverse of `PointSet.to_structured_surface(...)`, and a round-trip test now
  pins the pair as bit-for-bit lossless (previously true, but unenforced).
- `API.md` now declares `StructuredMeshSurface` and `PointSet::to_structured_surface`,
  which the implementation had grown without the contract.

### Fixed
- `PointSet.infer_geometry(...)` no longer returns a silently wrong geometry for a
  **curvilinear** mesh. When `column`/`row` topology is present the lattice was
  estimated from median node deltas and never checked against the nodes, so a mesh
  with varying cell size or a non-90° cell angle yielded a plausible-looking
  `GridGeometry` that no node sat on. Inference now verifies the lattice against the
  nodes and raises `GeometryInference` — pointing at `to_structured_surface(...)`,
  which represents such a mesh exactly — matching the strictness the coordinate-only
  path already enforced. Isolated off-lattice nodes (a collapsed or clipped export
  node) are still tolerated.

### Changed
- **Breaking:** `GeometryEdge` is now `{ Occupied, ConvexHull, FullRect }`, matching
  the locked `API.md` contract. The `ConcaveHull` and `Trimesh` variants are removed,
  along with the `"concave_hull"`/`"alpha"`/`"outer"`/`"default"`/`"trimesh"`/`"tin"`
  Python aliases, which now raise `ValueError`.
- **Breaking:** `edge="occupied"` now means the **outline of the occupied lattice
  nodes** — the true data footprint, tracking interior holes and a non-rectangular
  boundary — on both `infer_geometry(...)` and `to_structured_surface(...)`. It
  previously meant a bounding rectangle on the former and the occupied-cell outline
  on the latter: one name, two unrelated behaviours. Use `edge="full_rect"` for the
  bounding rectangle.
- `PointSet.infer_geometry(...)` now defaults to `edge="full_rect"` (four corners of
  the bounding lattice); `to_structured_surface(...)` defaults to `edge="occupied"`,
  since a curvilinear mesh has no bounding regular lattice to report.

### Performance
- The occupied footprint no longer triangulates the point cloud. `infer_geometry`
  already derives every point's lattice index, so the occupancy is reused directly:
  the Delaunay pass and its two float-keyed hash maps are gone. On a topology-less
  250k-point grid the footprint edge went from ~1057 ms to ~42 ms (~25x), and now
  costs the same as `full_rect`.
- Updated docs for point-derived surfaces to make the distinction explicit:
  point sets render as points, inferred geometries render clipped regular grid
  lines, and structured surfaces preserve locally shifted Petrel nodes.
- Updated dependency floors to `petektools>=0.2.7,<0.3` for the viewer
  point/geometry rendering fix.

## [0.3.7] - 2026-07-08

### Added
- Added `StructuredMeshSurface` for topology-bearing point grids that need
  explicit per-node XY coordinates, including Python access to `kind`, `ncol`,
  `nrow`, `node_xy(...)`, `z(...)`, `values()`, `geometry`, `edge`, `stats()`,
  and `history()`.
- Added `edge="trimesh"` / `edge="tin"` for
  `PointSet.infer_geometry(...)`. This is now the default point edge and traces
  the exterior of the locally connected point triangulation.

### Changed
- Reworked point-derived geometry edges. `edge="occupied"` now means the tight
  grid-oriented rectangle covering all finite point XY positions;
  `edge="full_rect"` remains the inferred regular geometry rectangle; and
  `edge="convex_hull"` remains an explicit convex envelope.
- Surface-point docs now recommend the default triangulated edge for point
  footprints, while keeping `occupied` available as the compact rectangular QC
  outline.
- Updated the Python dependency floor to `petektools>=0.2.6,<0.3` so the wheel
  gets the viewer topology-grid QA improvements from petekTools 0.2.6.

## [0.3.6] - 2026-07-08

### Added
- Added calculated-log assignment across project wells with
  `project.wells.assign_log(...)`, strict same-basis arithmetic by default,
  explicit output bases through `basis=logs.<curve>`, and operand-local
  resampling through `.to_basis(..., interpolation=...)`.
- Added point and polygon column calculations. Point sets expose coordinate and
  attribute columns such as `points.z` and `points.PHIE`; polygon sets expose
  `polygons.area` and numeric attributes. Container-to-container arithmetic is
  intentionally unsupported.
- Added standardized operation history across surfaces, points, polygons, logs,
  and generated views/objects. Derived objects preserve source history and label
  secondary contributors with role prefixes such as `rhs.*`, `mask.*`, and
  `prior.*`.

### Changed
- Imported logs are now documented and treated as MD/value curves on a bore,
  independent of their source LAS file after import. Curves from separate files
  can combine directly when their MD vectors match.
- The Python wheel now depends on `petektools>=0.2.5,<0.3` so calculated-log
  resampling uses the shared Rust `interp1d` kernel, including natural-cubic
  spline support.

## [0.3.5] - 2026-07-08

### Changed
- EarthVision/Petrel point-grid loading now preserves `column` and `row`
  fields as point attributes and uses them as authoritative topology for
  `PointSet.infer_geometry(...)`. `Project.import_data(...)` now enriches
  same-stem Petrel IRAP point exports from matching EarthVision topology files
  when both are present. Standalone plain IRAP/XYZ point exports that have lost
  those fields remain strict XY-only inference and now report a clearer hint
  when duplicate nodes make exact geometry recovery impossible.
- Renamed raw project ingestion from `Project.load(...)` with `LoadSettings` to
  `Project.import_data(...)` with `ImportSettings`. `Project.load(...)` and
  `Project.save(...)` now deal only with compact `.pproj` projects.
- Added folder-aware project collections. Object names can use `/` folders
  such as `structure/top dome`; `project.surfaces` shows immediate children,
  `project.surfaces.structure` descends into the folder, `.all_names()` returns
  canonical names, and unique leaf lookup such as `project.surfaces.top_dome`
  resolves when unambiguous. Project objects can now be renamed/deleted through
  typed methods (`rename_surface`, `delete_points`, etc.) or generic
  `rename(kind, old, new)` / `delete(kind, name)`.
- Surface, polygon, point, and log readers now normalize through canonical
  internal payloads before constructing the public domain objects. `.pproj`
  persistence remains format-independent: projects save `Surface`, `PointSet`,
  `PolygonSet`, and `Well`/`Log` objects rather than source-format details.

## [0.3.4] - 2026-07-07

### Added
- Added strict regular-grid inference for point clouds:
  `PointSet::infer_geometry(tolerance)` in Rust and
  `PointSet.infer_geometry(tolerance=1e-3, edge=...)` in Python. The Python
  edge option accepts `"occupied"`, `"convex_hull"`, and `"full_rect"` and
  returns a `GridGeometry` carrying a matching `geometry.edge` polygon. Inference
  raises a loud geometry-inference error when the points are genuinely scattered,
  duplicated onto the same lattice node, or miss the detected lattice by more
  than tolerance.
- Added `GridGeometry.edge` and `Surface.edge` in Python. `surface.geometry`
  now returns a geometry whose `.edge` polygon matches `surface.edge`, so
  modelling code can use the same edge API whether geometry came from a surface
  or from point-set inference.

### Changed
- Renamed the Rust surface outline method from `Surface::boundary_polygon()` to
  `Surface::edge()` and routed model-input boundary derivation through the new
  edge method. This removes the duplicate legacy surface-boundary API name in
  favour of the shared geometry/surface edge vocabulary.

## [0.3.3] - 2026-07-07

### Added
- Added `LoadSettings` and made `petekio.Project.load(..., settings=...)` the
  canonical project-loading entry point.
- Added list-like project inventories for `project.surfaces`, `project.wells`,
  `project.wells.logs`, `project.wells.<well>.logs`, `project.tops`,
  `project.points`, and `project.polygons`; loaded asset names now prefer simple
  stems such as `Top reservoir` when unambiguous.
- Added `project.tops["well tops"]` DataFrame access for loaded top sets.

### Changed
- Moved log inventory under `project.wells.logs` and per-well log collections,
  aligning project structure with the suite API design.
- Extended inline log expression tests around filtered calls such as
  `logs.PHIE(logs.NetSand > 0.50)`.
- Updated CI and release workflows to current action versions and the
  Actions-owned release flow.
- Aligned the internal Python binding crate's self-dependency floor with the
  workspace release version.

## [0.3.2] - 2026-07-07

### Added
- Python `petekio.Project.load(path, aliases=None, crs=None, settings=None)` as
  the canonical raw-project loading facade. It wraps `GeoData` rather than
  owning duplicate data, delegates `.pproj` files to `GeoData.open`, recursively
  scans raw project directories, loads wells before Petrel tops, exposes
  inventory counts/names/skips, and keeps `crsmeta.xml` sidecars out of skipped
  records.
- Added `project.logs`, `Logs`, `LogChannel`, and `LogPredicate` for lazy,
  pandas-style well-log expressions such as `logs.PHIE(logs.NetSand > 0.50)`.
  The resolver returns positioned per-well samples and accepts serialized
  expression dictionaries so downstream modelling layers can persist or forward
  the same source description without importing petekIO internals.
- Added defensive caching for resolved log expressions. Cached results are
  copied on return so consumers can mutate their local sample lists without
  corrupting the project cache.

## [0.3.1] - 2026-07-06

### Added
- Public bounded content detector: `FormatKind` plus `detect(path)`, with
  content-first identification for CPS-3, IRAP classic/points, EarthVision, LAS,
  wellpath, Petrel tops, `crsmeta.xml`, GeoJSON, and CSV point headers. New
  tests cover extensionless and misnamed-extension files.
- `GeoData` manager dispatch now consumes `detect()` for surfaces, points,
  polygons, and well-tree file classification where practical, so content wins
  over misleading extensions. `FormatKind::Unknown` preserves the previous
  extension fallback.
- Minimal `crsmeta.xml` sidecar support for well loads: the CRS label is parsed
  and attached to `Well::crs()` (label only, no reprojection). Surface, point,
  and polygon objects still have no CRS field, so no sidecar is attached there.
- Petrel `Type=Other` well-tops picks now surface as `FluidContact` values on
  `Well`/`Sidetrack` via `contact(s)` instead of being silently dropped. Contacts
  stay separate from formation `Top`s, so they do not create zones or alter the
  `load_well_tops` assigned-top count.

### License — Apache-2.0 (`decision_license_ratified`)
- The project is licensed under **Apache-2.0**. Canonical Apache License 2.0 text
  is provided as [LICENSE](LICENSE); a [NOTICE](NOTICE) names the copyright holder.
  Cargo `[workspace.package] license` and `pyproject` `license` / classifier are
  set to Apache-2.0.

### ⚠️ BEHAVIOUR CHANGE — surface resample centralized onto the shared kernel
petekIO's `Surface::sample` / `Surface::resample` now delegate to the shared
`petektools::resample` (Bilinear) kernel — **one home** for the bilinear
resampling math (the one-resampler rule). Two visible changes:

- **NaN-corner policy CHANGED.** petekIO previously **hard-holed on ANY**
  undefined corner (returned `None`/`NaN` if any of the four surrounding nodes
  was `NaN`). The kernel's policy now applies: if the **NEAREST** of the four
  corners is undefined the result is `None`/`NaN`; **otherwise** it is the
  weighted mean over the **finite** corners with the weights **renormalized** (a
  `NaN` far corner is dropped, not treated as zero). This fills the
  finite-supported fringe around a hole instead of eroding it. *Impact
  assessment:* no committed test/fixture exercises a NaN-fringe resample and
  identity resample stays bit-exact, so no real-format golden is affected; the
  change is pinned by the reframed `sample_nan_corner_policy` unit test.
- **`Surface::resample` now returns `Result<Surface>` (was infallible
  `-> Surface`).** It raises the new **`GeoError::Unsupported`** on a **rotated**
  source OR target geometry (`rotation_deg != 0`), because the shared kernel is
  **axis-aligned-only** — a loud typed error instead of a silently-untested
  answer, pending the suite-wide grid-rotation work (`task_suite_grid_rotation`).
  `yflip` is fully supported. **Real impact:** rotated IRAP/RMS surfaces (the
  committed `simple.irap` fixture is rotated 30°) can no longer be resampled
  until the kernel gains rotation support; **point sampling stays exact under
  rotation** (a single world→index map) and `Surface::sample` remains infallible.
  Python `Surface.resample` raises `ValueError` on a rotated geometry.

### Centralization — units & stats delegate to the family kernels (behaviour-neutral)
- `Unit::metres_per_unit` (and thus `area_to_m2`'s squared factor) is
  single-homed on `petektools::units::FT_TO_M`; the unweighted type-7 percentile
  (`Stats::percentile` unweighted) delegates to `petektools::stats::percentile`.
- **Population std** (`n` denominator) and the **cumulative-weight (nearest-rank)
  weighted percentile** deliberately **stay local**, with documented
  convention-difference notes — they are genuinely different algorithms from
  petektools' sample `std_dev` / interpolating `weighted_percentile`.

### Build — pre-release family path dependency
- `petektools` is pinned to the sibling working tree (`path = "../petekTools"`),
  matching the rest of the suite, to kill the dual-`petektools` lockfile
  type-identity trap. **The publish batch swaps all family deps back to registry
  versions together** — petekIO must not publish with a path dep.

### Spec-pattern surface — NetSettings / IngestSpec / ViewSpec / ViewSettings (additive; every existing test stays green)
The petek family **house spec pattern** applied to petekIO's Python surface —
declarative, frozen value-objects (each: `to_dict`/`from_dict` with a `"spec"`
type tag, value equality, `.replace(**overrides)` derivation, a domain-table
`repr`), applied at explicit moments. Locked by a parametrized conformance
battery (`test_spec_conformance.py`, testing-doctrine R7).
- **`NetSettings(phi_min=, sw_max=, vsh_max=)`** — φ/Sw/Vsh reservoir cutoffs
  (wraps core `Cutoffs`). Accepted by `net_zone_stats(cut=)`, `zone_table(cut=)`
  (new per-cell net-conditioning; `phi`/`sw`/`vsh` name the curves), and
  `ViewSpec(cutoff=)`. The existing scalar kwargs stay as per-call overrides on
  top of a `cut`.
- **`IngestSpec(aliases=, strat_hints=, unit=)`** — declarative load-time
  canonicalization, applied per-call at `load_well(ingest=)` /
  `load_well_tops(ingest=)`. **Kills the order-dependent sticky state:** the
  `load_well(aliases=)` kwarg and `GeoData.strat_hint(...)` now emit a
  `DeprecationWarning` (removal in a future minor) — they construct/mutate the
  same state the spec carries declaratively. `unit` is a loud guard (errors if it
  disagrees with the project unit).
- **`ViewSpec(curves=, tops=, flatten_default=, flags=, cutoff=)` +
  `ViewSettings(serve=, save=)`** — collapse the four hand-synced `view()`
  signatures (`Well.view`, `WellsView.view`, `_viewer.render`,
  `build_well_log_bundle`). All accept `spec=`/`settings=`; the legacy per-call
  kwargs remain, but passing a spec **and** its legacy kwargs is a loud error
  (spec XOR kwargs).
- Core mirrors (Rust): `Cutoffs` and `NameMap` gain `Serialize`/`Deserialize` +
  `Display` (`Cutoffs` also already `PartialEq`); a new `StratHints` value type
  (serde + `PartialEq` + `Display`); `GeoData::load_well_with` (per-call,
  non-sticky aliases) and `GeoData::add_strat_hints`; `build_zone_table` gains an
  optional `net: Option<NetCond>` arg (`None` → bit-identical to before).

### Performance & boundary hardening (behaviour-neutral; every existing test stays green)
- **The GIL is released around compute-heavy / IO calls** (`py.detach`): min-curvature
  gridding (`PointSet.to_surface`), `Surface.resample` / `volume_between` /
  save+load IO, the `.pproj` `save`/`open`/`inspect`/`split`/`export`/`merge`,
  every `PointSet`/`PolygonSet`/`Surface` loader, and the `zone_table` /
  `net_zone_stats` / `well.view()` crunch — so other Python threads make progress
  while petekIO computes. Regression-guarded by a GIL-release smoke test.
- **A `Surface` handed back from a project (`GeoData.surface()`/`surfaces()`/
  `load_surface()`) is now a cheap view — no per-access deep copy** of the grid +
  every attribute layer. Mutation (`set_attr`) is copy-on-write: a view detaches to
  an owned copy first, so a handed-back surface still never writes back into the
  project (identical observable semantics, minus the copy on read).
- **Petrophysics conditioning hot path ~5× faster** (5.56 ms → 1.07 ms on a
  40-curve × 4000-sample field, `cargo bench --bench petro_hotpath`): the three
  `net_zone_stats` cutoff curves are resampled onto each zone's MD grid via a
  single O(n+k) merge-walk instead of a per-sample O(k·n) `at_md` sweep.

### Added
- `PointSet::coords(&self) -> &[[f64; 3]]` (Rust) — the public read side of
  `from_coords`: the raw `[x, y, z]` points in load order (`NaN` carried
  through). Additive (open/closed); lets a consumer grid the scatter itself
  rather than only via `to_surface`. Round-trip test (`from_coords` → `coords`)
  added.
- `LogView::resample_onto(targets)` (Rust) — resample onto arbitrary ascending
  targets via a merge-walk; bit-identical to mapping `at_md`.
- Python `LogView.at_md_many(depths)` and `LogView.values_md()` — batched
  accessors that resolve the well→bore→log chain once for a loop instead of
  re-walking it per call.
- Core kernels `algorithms::wells::dz_weights` and the
  `analysis::well_tables` crunch (`build_zone_table` / `net_zone_samples` /
  `gather_raw_logs`), lifted out of the Python bindings so each formula has one
  home in core (the binding is now a thin marshaller). Outputs are bit-identical.

### Changed (BREAKING behaviour — target 0.3.0)
- The 0.3.0 breaking window bundles this z-convention change with the
  family-wide **SI/metric standard** (petekSuite `decision_si_units_standard`):
  metres, negative-down elevation as the default z, imperial opt-in only. petekIO
  stays metric-native — the readers and Python surface introduce no imperial
  defaults.
- **One z convention across the library: `xyz()` now returns negative-down
  elevation.** `Trajectory::xyz` / `Sidetrack::xyz` / `Well::xyz` — and
  `WellCurveInput.xyz` in `model_inputs()` — previously returned `z` as
  **positive-down TVDSS**, while `Surface` values are **negative-down
  elevation**: the two disagreed in sign, a silent hazard when positioning
  curves against horizons (weakness W2). Position `z` is now **negative-down
  elevation everywhere**, matching `Surface` z (a point below the datum has
  negative z), so consumers no longer hand-negate. `tvd()` / `md_at_tvd()` are
  **unchanged** — they remain the domain-natural positive-down TVDSS
  (`tvd(md) == -xyz(md).z`); `net_pay` (TVD-based) is unaffected. Persisted
  `.pproj` trajectories are byte-identical (internal storage stays positive-down;
  only the public accessor sign changed). **Consumers reading `xyz().z` or
  `WellCurveInput.xyz[..][2]` must drop any manual sign flip.**
- **`SummaryInputs` is now metric — the last mixed-convention surface in the
  contract is gone.** The model-ready DTO carried imperial, positive-down
  scalars; they are now base-SI:
  - `reservoir_area_acres` → **`area_m2`** (m², base SI — keeps the whole DTO in
    metres/m² and matches the consumer's internal `area_m2`; conversion factor
    `metres_per_unit()²`, i.e. 0.3048² from a feet project).
  - `net_pay_ft` → **`net_pay_m`** (m; factor `metres_per_unit()`, 0.3048 from
    feet). The per-well samples are converted **before** characterising, so the
    `Uncertain` Normal is built natively in metres — an exact location-scale
    rescale (location and scale both scale by the factor; shape preserved).
  - `owc_ft`/`goc_ft` → **`owc_depth_m`/`goc_depth_m`**, **positive-down depth in
    metres**. These are *depths*, not elevations: deeper = larger, matching the
    consumer (petekStatic) `Contact.depth_m` datum. Geometry z stays negative-down
    elevation (`xyz()`, `Surface`); scalar contacts are depths, named `_depth_m`
    so the sign is unambiguous.
  - Backing helper `Unit::area_to_acres` → **`Unit::area_to_m2`**.

### Added
- **Standalone `well.view()` — the WellLogBundle producer + a logs-only viewer
  session** (petekio's slice of the well-correlation seam,
  `petekSuite/dev-docs/designs/well-log-bundle-seam.md`; wire format codified in
  `petektools/viewer/SCHEMA.md`).
  - `Well.view(curves=, tops=, flatten_default=, phie_cutoff=0.08, flags=, serve=True,
    save=)` and `WellsView.view(...)` — build a `WellLogBundle` (kind
    `"wells_logs"`, `schema_version` 4) straight from a well's own logs +
    trajectory (no model) and hand it to the viewer unit: a JSON header + v3 f32
    base64 lane blocks (`md_m`/`tvd_m`/curve `values`, `NaN` = `0x7FC00000`), per
    curve `range`/`cutoff` (on effective porosity)/`codes` (flag strips). `tops`
    is opt-in (`True`/a name list) — a standalone bundle carries `tops[]`/`zones[]`
    only when asked and **never** `ties` (model context only). Returns a
    `LogSession` mirroring the viewer's ergonomics — `.serve()` (non-blocking
    local server) / `.save(path)` (one self-contained HTML file) — both delegated
    to `petektools.viewer`, an **optional runtime dependency** imported lazily
    with a helpful error if absent. TVD is trajectory TVDSS where a survey exists,
    else the vertical assumption `md - kb`.
  - `petekio.canonical_mnemonic(raw)` — the family curve-name authority exposed to
    Python (case-insensitive, vintage-tag stripped); the producer canonicalizes
    every mnemonic through it.
  - `petekio.build_well_log_bundle(...)` / `petekio.encode_lane(...)` — the pure
    Python producer + lane encoder (a documented seam-twin of the viewer wire
    format; petekio duplicates the small schema rather than importing petektools,
    per the family coupling rule), exposed for direct bundle construction + tests.
- **Python petrophysics access** (weaknesses W3/W4/W10).
  - `Sidetrack.log(mnemonic) -> LogView` — per-sample `values()`/`md()`/`at_md()`
    on a **named bore** (previously only aggregate stats were reachable; the raw
    curve lived on the sidetrack, unreachable from Python).
  - `Sidetrack.net_zone_stats(value, phi=, sw=, vsh=, phi_min=, sw_max=, vsh_max=,
    geomean=)` — net-conditioned per-zone aggregation: keeps only samples passing
    the φ/Sw(/Vsh) cutoffs (φ/Sw/Vsh sampled onto `value`'s MDs), then aggregates
    — `[(zone, Stats)]` (net arithmetic mean, e.g. NTG-conditioned φ/Sw) or
    `[(zone, float)]` net geometric mean (`geomean=True`, e.g. permeability).
  - `Stats::geomean(values)` (Rust) + `Stats.geomean([...])` / `LogView.geomean()`
    (Python) — geometric mean of positive values.
  - `PointSet.z_stats()` — stats over the z coordinate (horizon depth range),
    which `stats("z")` could not reach.
  - **In-memory constructors**, Python-exposed: `PointSet::from_coords` /
    `PointSet.from_xyz(x, y, z)` and `PolygonSet::from_rings` /
    `PolygonSet.from_rings(rings)` — build point/polygon sets from coordinate
    arrays without a file.
- **Opt-in curve-mnemonic canonicalization at load** (weakness W9). New
  `GeoData::set_curve_aliases(NameMap)` and Python
  `load_well(..., aliases={"PHIE_2025": "PHIE"})`: each loaded log's mnemonic is
  mapped to canonical — the user alias map first (the choices the table can't
  guess), then the built-in table + vintage `_YYYY` strip — so `log("PHIE")`
  resolves a `PHIE_2025` curve and the model contract's canonical PHIE/SW is
  produced. `aliases={}` (empty map) gives pure auto-canonicalization. Off by
  default: without aliases, raw mnemonics are preserved unchanged.
- **Real-format readers: CPS-3 grid, CPS-3 lines, EarthVision grid, and
  `.IrapClassicPoints` dispatch** (weaknesses W6/W7/W8, and the read side of W5).
  - `Surface::load_cps3_grid` — CPS-3 regular grid (`.CPS3grid`): `FS*` header
    (`FSLIMI`/`FSNROW`/`FSXINC`/`FSASCI`) + row-major z, `1.0E+30`-family null →
    `NaN`, with a **documented north→south node ordering** (row 0 = `ymax` edge;
    mapped to `GridGeometry` `yori=ymax, yflip=true`).
  - `PolygonSet::load_cps3_lines` — CPS-3 polyline/polygon (`.CPS3lines`): `FF*`
    header + `->`-separated polyline blocks → one ring per block (structure
    outlines, fault polygons, model edge).
  - `PointSet::load_earthvision_grid` — EarthVision grid ASCII
    (`.EarthVisionGrid`): scattered `x y z` with a directive header; null nodes
    dropped.
  - `GeoData::load_surface`/`load_points`/`load_polygons` now dispatch the new
    extensions (`.CPS3grid` / `.EarthVisionGrid`+`.IrapClassicPoints` /
    `.CPS3lines`); full Python classmethods + `GeoData` dispatch.
- **LAS 3.0 (delimited) reading.** A LAS 3.0 file (`VERS 3.0`, often `DLM COMMA`
  with `~Log_Definition`/`~Log_Data` sections) previously loaded as a well with
  **zero curves, silently** (`las_rs` 0.2 reads only 1.2/2.0). The LAS reader now
  sniffs `~Version` `VERS`: < 3.0 defers to `las_rs` (unchanged); at 3.0 a small
  **contained internal parser** reads the delimited layout (space/comma/tab,
  NULL → `NaN`) behind the same `Log::load_las`/`load_las_all` API, so
  `load_well` ingests LAS 3.0 core curves. An unsupported 3.0 variant (wrapped
  data, or no curve/data section) returns a typed `GeoError::Format` naming the
  reason instead of dropping the curves silently.
- **`GeoError::Format`** — a typed error for "wrong/unexpected file format" (an
  EarthVision grid handed to the IRAP-points reader; an unsupported LAS 3.0
  variant), naming the detected/declared format so a caller can route.

### Fixed
- **CPS-3 grid reader Y-orientation — surfaces no longer ingest upside-down**
  (`task_petekio_cps3_yflip`; **BREAKING behaviour change, loud**). `load_cps3_grid`
  (and `GeoData::load_surface` on `.cps3grid`) mapped the row-major z stream with
  the first data row at the **north** edge (`yori = ymax, yflip = true`). The
  correct convention — matching the IRAP-classic baseline and the Golden Software
  CPS-3 definition, whose values run bottom-up — takes the first data row as the
  **south** edge (`yori = ymin, yflip = false`). The old behaviour ingested a
  CPS-3 grid **Y-flipped** relative to the IRAP copy of the same surface: a deep
  dome came back as its mirror image. **Impact:** anyone who ingested `.CPS3grid`
  surfaces before 0.3.0 got them mirrored north↔south; re-ingest, and drop any
  downstream orientation work-around (e.g. a Y-flip auto-corrected against an
  outline centroid). No CPS-3 header field encodes the Y direction, so the
  south-origin convention is assumed and documented in `io/cps3.rs`. Regression
  cover: `tests/grid_orientation.rs` asserts an **asymmetric** synthetic surface
  ingests node-for-node identically as IRAP and CPS-3 (plus an EarthVision
  orientation golden).
- **Multi-sidetrack wells now surface every bore — no more empty well downstream**
  (R-a; **behaviour change, loud**). The universal NCS case is one well id with
  several `.wellpath` bores (`99/9-1` A/B/ST2, each its own comp-log). Those route
  to named sidetracks while the main bore stays empty, and the well-level
  accessors only covered the main-or-single-trajectory case — so a multi-bore well
  exposed **no** picks/trajectory/logs through the top level, and `model_inputs()`
  handed the consumer an empty well. Fixes:
  - **`model_inputs()` is bore-aware.** It now emits **one positioned
    `WellCurveInput` set per bore** — positioned by that bore's own trajectory,
    keyed by the bore-qualified `well_id` (`"99/9-1 A"`; the id alone for the main
    bore) — and folds each bore's net-pay petrophysics separately. Each bore is an
    independent positioned "well" to the geomodel. (A single-bore well is
    unchanged: `well_id` = the id.)
  - **Honest top-level semantics — silent-empty is gone.** New
    `Well::set_default_bore(label)` / `default_bore()` / `clear_default_bore()` and
    `is_multibore()` select the bore the delegating accessors resolve through (the
    single-trajectory rule still applies when only one bore has a trajectory). On a
    multi-bore well **with no default selected**, the **Python** accessors
    (`Well.xyz`/`tvd`/`md_at_tvd`/`top`/`log`) now **raise `ValueError`** naming the
    bores and pointing at `.sidetrack(name)` / `.set_default_bore(name)`, instead
    of returning silent empties. (Rust keeps the `Option` return — a multi-bore
    well with no default resolves through the empty main bore → `None`.)
  - **Per-bore access is first-class + complete.** `Sidetrack` gains `mnemonics()`
    (joining the existing `xyz`/`tvd`/`md_at_tvd`/`log`/`logs`/`top`/`zones`/
    `zone_stats`); Python `Sidetrack` already exposes the per-bore petro surface
    (`log`/`net_zone_stats`/`zone_stats`/`md_range`). `Well::bores()` (labels),
    `Well::bore_id(label)`, and `GeoData::well_mut(id)` are added; Python `Well`
    gains `is_multibore`/`default_bore`/`set_default_bore`.
  - **Tops still route to the right bore when several are present** — the
    bore-suffix matching (`"99/9-1 A"` → bore `A`) holds under multiple bores
    (covered by a test).
- **The tops CSV reader decodes Latin-1, so a Norwegian marker name no longer
  aborts the load.** `Top::load_csv` (the well-level `.csv` tops path) fed the
  file to `csv::Reader::from_path`'s strict-UTF-8 decoder, so a real Petrel/RMS
  export carrying a Latin-1/Windows-1252 byte (e.g. `"Blåbær"`, `0xE5`) failed to
  parse. It now reads the bytes and decodes them through the same `decode_latin1`
  path every other reader uses (IRAP/CPS-3/LAS/wellpath/Petrel-tops), before
  parsing — so `Å`/`å`/`æ`/`ø` land as proper Unicode. (The Petrel well-tops
  reader already decoded Latin-1; this closes the same gap on the CSV path.)
- **A single-trajectory well positions its logs/tops through that one path,
  regardless of bore naming.** A deviated single-sidetrack well (one `.wellpath`
  + one comp-log + tops, e.g. NCS `99/9-1 A`) previously mis-attached: the
  trajectory landed on a bore named after the wellpath stem while the LAS/tops
  defaulted to the *main* bore, so `Well::xyz`/`log`/`top` (and hence
  `WellCurveInput.xyz` in `model_inputs()`) could not position the curves —
  positions came back `NaN`. Two coordinated fixes: (1) a **single** `.wellpath`
  is now the well's one *main* bore `""` as documented (it was mislabelled with
  the full filename stem), so logs/tops co-locate with the trajectory; and (2)
  well-level resolution follows the **single-trajectory rule** — when exactly one
  bore carries a trajectory, `Well`'s accessors resolve through it even if it is
  a named bore. Multi-bore wells are unchanged: they resolve through the main
  bore and select a bore explicitly.
- **Well-name matching tolerates the family's naming variants.** Petrel well-tops
  distribution (`load_well_tops`) now folds separators (`/`, `-`, space → `_`)
  and case before matching the record's `Well` field to a loaded well id, so a
  pick keyed `99_9-1_A` reaches the well loaded as `99/9-1 A` (previously dropped
  silently). The bore suffix after the id is still returned in its original case
  so it keys the case-sensitive sidetrack label.
- **`load_irap_points` sniffs the header and rejects foreign formats.** An
  EarthVision grid (and CPS-3 / LAS) handed to the IRAP-points reader previously
  mis-parsed its numeric-looking header into a wrong-sized point set with no
  error (weakness W5). The reader now scans a header window and returns a typed
  `GeoError::Format` naming the detected format (EarthVision grid / CPS-3 / LAS)
  so the caller routes to the right reader; a plain `X Y Z` file is unaffected.
- **CSV readers surface I/O failures as `GeoError::Io`, not stringified `Parse`.**
  `PointSet::load_csv` and the tops CSV reader now route an underlying I/O error
  (missing file, mid-read failure) through `GeoError::Io(#[source])` so
  `err.source()` chains reach the origin `std::io::Error` (a caller can match
  `io::ErrorKind::NotFound`); genuine CSV *format* errors stay `Parse`.

### Changed
- **Depend on the published `petektools` crate.** The local `path` dependency on
  petekTools is now a versioned crates.io dependency (`petektools = "0.1"`),
  making petekIO publishable again. No source or behaviour change — the same
  kernels + container the path dep provided, now consumed from the registry.
- **`.pproj` generic container lifted to petekTools.** The domain-agnostic
  framing (file magic + JSON header + `zstd`-compressed section blobs +
  byte-lossless split/merge) now lives in `petektools::container`; petekIO
  re-exports it and keeps its GeoData element DTOs (Surface/Well/PointSet/
  PolygonSet + the `model/*` opaque sidecar) layered on top. **No format or API
  change** — the on-disk `.pproj` layout is identical and existing files still
  round-trip. Internal only (adds a `petektools` dependency; `zstd` is no longer
  a direct dependency).
- **Gridding migrated to the shared petekTools kernels.** Both the cold path
  (`PointSet::to_surface`, Nearest/IDW/min-curvature) and the warm-start path
  (`PointSet::regrid_min_curvature`) now delegate to `petektools::grid` /
  `petektools::grid_min_curvature_seeded`; the local `core/gridding.rs` kernel is
  deleted. `GridGeometry` maps 1:1 onto `petektools::Lattice` at the seam.
  **No public-API or numerical change** — the kernels were lifted from petekIO
  0.2.0 and held at behaviour parity (Briggs ω=1.5 / TOL=1e-6, IDW p=2); the
  golden warm-start suite is unchanged and green. Internal only.

## [0.2.8] - 2026-07-01

### Added
- **Project persistence — a single `.pproj` file.** Save/load a whole `GeoData`
  project to one structured, efficient file (magic + JSON manifest +
  `zstd(bincode)` sections): `GeoData::save/open`, `inspect` (manifest only —
  list without decoding), and per-element `Surface/Well/PointSet/PolygonSet`
  `save`/`load`. The file is **splittable / mergeable / tag-filterable** for
  team sharing — `split(names)`, `merge(a,b)`, `export(tags)` copy sections
  byte-for-byte (no re-encode). Project metadata: `owner`, project + per-element
  `tags`, timestamps. petekSim's model persists as reserved **`model/*` opaque
  sections** (`put_model_section` / `model_section` / `model_section_names` —
  bytes petekIO never parses, each with its own version). Two-tier versioning
  (hard magic + `data_version` gate + serde-default manifest); atomic writes;
  unknown/`model/*` kinds skipped on load (forward-compatible); NaN preserved.
  Human-readable export: `PointSet::export_geojson/export_csv`,
  `PolygonSet::export_geojson`. Full Python bindings. (petekSim-signed-off format.)
- **`zone_table` views, aggregation, and thickness-weighting** (Python, `Well` +
  `GeoData.wells`):
  - `pivot=True` → wide: `zone` index × `bore` columns (single stat flat; several
    → MultiIndex `(stat, bore)`); `zone` keeps lithostratigraphic order.
  - `aggregate=True` → grouped by zone with a pooled **all** row first, then the
    per-bore rows, indexed by `(zone, bore)`. Mutually exclusive with `pivot`.
  - `weighted=True` (**default**) thickness-weights every average — per-bore and
    the pooled aggregate — by each sample's MD span, so a finely-sampled log no
    longer outweighs a coarse one over the same interval. Uniform sampling is a
    no-op; `weighted=False` restores the plain sample mean. (`sum` then becomes
    `Σ(dz·value)` — the thickness-integrated quantity, e.g. Σφ·dz.)
  - `stats=` also accepts **`samples`** (sample count) and **`gross`** (zone MD
    thickness; its aggregate is the mean across bores).
  - `zones=[...]` keeps only the named zones (case-insensitive, lithostrat order
    preserved); unknown names contribute no rows.
  - `decimals=N` rounds the stat values. Default `pivot=False` is the tidy frame.

## [0.2.7] - 2026-06-30

### Added
- **`zone_table()`** (Python) — a first-class tidy per-`zone × bore` table for a
  curve, on `Well` and `GeoData.wells` (`WellsView`). Returns a `pandas.DataFrame`
  with columns `zone`, `bore`, then one per requested stat (`stats=` are `Stats`
  attribute names — mean/sum/count/min/max/std/p10/p50/p90; default `["mean"]`).
  `zone` is an ordered Categorical in lithostratigraphic order, so it survives
  `pivot`/`groupby` with no manual reindex; zero-thickness / no-sample cells are
  dropped unless `include_empty=True`. At the well-set level `bore` identifies
  well + sidetrack. pandas is an **optional** extra (`pip install petekio[pandas]`),
  imported lazily — the base wheel stays dependency-free.

## [0.2.6] - 2026-06-30

### Added
- **Manual lithostratigraphic hints.** `GeoData::add_strat_hint(above, below)` and
  the shorthand `strat_hint("A < B")` (`A < B` = A above B, `A > B` = A below B;
  Python `geo.strat_hint("A < B")` or `geo.strat_hint(above=, below=)`) let you
  resolve orderings the data can't — pairs coincident in *every* well. A hint is
  applied **only where the data leaves the pair unordered**; any strict MD
  relationship always wins, so a hint can never override geology. Tokens may be
  partial names (resolved at `load_well_tops`: exact → `… top` → unique
  substring; ambiguous/unmatched errors).

## [0.2.5] - 2026-06-30

### Changed
- **Coincident-tops interval assignment follows lithostratigraphy.** When
  several tops share a measured depth (a zero-thickness cluster — e.g. a
  formation top stacked with sand members), the interval down to the next
  distinct-MD pick is now assigned to the cluster's **stratigraphically lowest**
  member (per the loaded `strat_order`), instead of the arbitrary
  insertion-order pick. So the marker immediately above the developed interval
  carries it — e.g. a `Base Shale top` coincident with the Upper Sand group correctly
  owns the reservoir interval beneath it. Geometry for distinct-MD tops is
  unchanged; only zero-thickness ties are affected, and only when a column is
  loaded.

## [0.2.4] - 2026-06-30

### Added
- **Global lithostratigraphic ordering.** `GeoData::load_well_tops` now derives a
  field-wide stratigraphic column across *every* well in the tops file (not just
  loaded ones): a marker that pinches out (zero thickness) in one well is ordered
  by a well that develops it. `zones()` / `zone_stats()` return zones in this
  order (zone geometry unchanged — only presentation order follows the column);
  `GeoData::strat_order()` exposes it. New pure kernel
  `algorithms::wells::merge_strat_order`.
- Python: `geo.strat_order` (the lithostratigraphic column) and a single-zone
  `bore.zone_stats(mnemonic, zone)` → one `Stats` (or `None`) instead of needing
  `dict(...)[name]`. The no-`zone` form is unchanged (returns the list).

### Changed
- Python `GeoData.load_well`: `head`/`kb` are now **optional**
  (`load_well(id, files=...)`). With a `.wellpath` present its header is
  authoritative and fills them; without one they default to `(0, 0)` / `0`.
  Backward-compatible (existing positional/keyword calls unchanged).

## [0.2.3] - 2026-06-30

### Added
- **Python multi-bore well surface** — `GeoData.load_well_tops(path)`; `Well.crs`
  / `Well.bores()` / `Well.sidetrack(label)` / `Well.sidetracks()`; and a
  `Sidetrack` binding with `mnemonics`/`log_stats`/`zones`/`zone_stats`/`tvd`/
  `xyz`/`md_range`. Python can now reach per-bore logs + per-zone stats (the data
  lives on the named bores, not the main bore).

### Fixed
- Petrel well-tops: capture the `Type` column; `GeoData::load_well_tops` now
  ingests only `Horizon` picks and skips `Other` (fluid contacts OWC/GOC/FWL),
  so derived zones are purely lithostratigraphic (contacts no longer split zones).

### Changed
- `GeoData::load_well` now walks a directory **recursively** (handles a Petrel
  export tree with separate `Paths/`/`Logs/` subdirs, not just a flat folder) and,
  when filenames carry the well id, **ingests only that well's files** (skips
  others sharing the tree). Flat per-well folders with generic filenames are
  unchanged. A LAS that fails to parse is now **skipped, not fatal**.

### Fixed
- LAS reader: fall back to the **first curve as the depth index** when the index
  mnemonic isn't the standard `DEPT` (e.g. Petrel core logs name it `DEPTH`).
- LAS/Petrel readers (`wellpath`, `petrel_tops`): **decode Latin-1/Windows-1252**
  exports (each byte → its Unicode code point) instead of erroring on non-UTF-8 —
  Norwegian names (`ø`/`å`/`æ`) now decode correctly rather than to `�`.

## [0.2.2] - 2026-06-29

### Added
- `analysis::normalize::canonical_mnemonic_with` + `NameMap::get` — resolve LAS
  mnemonics against a user alias map first (for the choices the table can't guess,
  e.g. `NTG_PhieLam` vs `NTG_VShale` → `NTG`), then the built-in table.

- **`.wellpath` ingest + multi-sidetrack wells** — `GeoData::load_well` reads
  Petrel `.wellpath` traces: one **bore per file** (labelled by filename stem),
  each a **positioned** trajectory (`TrajectoryInput::PositionedSurvey`, MD
  preserved, subsea `z = TVD − kb`); logs route to the matching bore. The
  wellhead XY / KB / **CRS** from the header are authoritative (`Well::crs`/
  `set_crs`; CRS recorded, never reprojected). No-`.wellpath` wells keep the
  synthesized-vertical behaviour.
- **Petrel well-tops ingest** — `GeoData::load_well_tops(path)` reads a multi-well
  `# Petrel well tops` export (quoted Surface/Well, `-999` nulls) and routes each
  pick to the matching loaded well + bore (`"99/9-1 B"` → well `99/9-1`, bore `B`).
- **Core data tagging** — `LogKind` (`Log` / `Core`) on `Log` (`kind()` /
  `with_kind`); `load_well` tags curves from `*core*.las` files as `Core` so
  consumers can include/exclude core in per-zone aggregation.
- **Per-zone aggregation** — `Sidetrack`/`Well` `zones()` (every formation zone
  as an `Interval`) and `zone_stats(mnemonic)` → per-zone `Stats` (average via
  `mean`, `sum`, percentiles). Broadcastable across a project's wells.

### Changed
- `canonical_mnemonic` now strips a trailing vintage tag (`PHIE_2025` → `PHIE`)
  and keeps **effective vs total water saturation distinct** (`SWT` no longer
  collapses to `SW`); unknown mnemonics pass through vintage-stripped (original case).

### Added
- `Trajectory::from_input` is now public — build a positioned path from a survey
  (`TrajectoryInput`) standalone, without a `Well`/`GeoData`.
- Python `Trajectory` binding: `Trajectory.from_stations([(md, inc, azi), …],
  head=(x, y), kb=…)` plus `xyz` / `tvd` / `md_at_tvd` / `md_range` — directional
  surveys can now be built and queried directly from Python.

### Added
- `PolygonSet::rings()` — exterior ring vertices per polygon (`[x, y, z]`, z=0,
  closed) + Python binding, so consumers can read the boundary outline geometry
  (not just `area`/`bbox`/`contains`).

### Changed
- New **`algorithms/`** layer: pure, type-light numeric kernels grouped by
  discipline (`algorithms::wells` — minimum-curvature survey: `tangent`,
  `dogleg`, `ratio_factor`, `arc_point`, `survey_positions`), with analytic QC
  tests. `Trajectory` now delegates to it (no behaviour change); the ratio-factor
  formula has a single home. See `SPEC.md` §9 (the algorithm discipline).

### Fixed
- `Trajectory::xyz`/`tvd` now interpolate along the **minimum-curvature arc**
  between survey stations (slerp of station tangents + partial dogleg) instead of
  straight-lining between station nodes. Mid-station TVD was previously off by up
  to ~40 m in build sections; it now matches an independent survey reference to
  <0.05 m. (`Xyz` paths still use straight-line interpolation.)

## [0.2.1] - 2026-06-29

### Added
- **Model-ready inputs contract (GATE-0)** — `GeoData::model_inputs() -> Result<ModelInputs>`,
  the locked seam consumers (petekSim) build to. Uncertainty/provenance
  vocabulary (`Uncertain`, `Distribution`, `Provenance`) and the contract types
  (`ModelInputs`/`SummaryInputs`/`SpatialInputs`/`HorizonInput`/`WellCurveInput`).
- `Uncertain` constructors: `hard`/`defaulted`/`assumed` (deterministic),
  `uniform`/`triangular`/`normal`/`lognormal` (characterised), `from_stats`, and
  the `with_provenance` builder.
- `Unit::area_to_acres` — planar area (m²/ft²) → acres, backing
  `reservoir_area_acres`.
- `Well::logs()`/`Well::mnemonics()` and `Sidetrack::logs()` — enumerate every
  curve on a bore (previously curves were only fetchable by mnemonic).
- `analysis::normalize` — input canonicalisation: `canonical_mnemonic` (LAS
  mnemonic alias table, unknowns pass through), `parse_length_unit`,
  `is_percent_unit`, `harmonise_fraction` (percent→fraction), `harmonise_length`,
  and `NameMap` (case-insensitive formation/well alias → canonical, identity for
  unknowns).
- `analysis::validate` — physical validity ranges per canonical mnemonic
  (`validity_range`, `in_range`) and `mask_out_of_range`, which rejects
  out-of-range samples to `NaN` (the undefined convention) and reports the count.
- `analysis::interpret` — petrophysical interpretation (petekIO owns net_pay):
  `Cutoffs` (φ/Sw/Vsh, defaulted 0.08/0.5/0.5), `net_flags` (per-sample
  reservoir/pay flag), `net_pay` (Σ Voronoi thickness over net samples, TVD
  depth), `net_to_gross`, and `leverett_j`.
- `analysis::characterise` — fit an `Uncertain` from a sample set:
  `DistributionShape` (Normal / Triangular = P10/P50/P90 / LogNormal) and
  `characterise`, collapsing to Deterministic below two defined values.
- `Surface::smooth` (NaN-aware moving-average, preserves the defined mask),
  `Surface::boundary_polygon` (convex hull of defined nodes), and
  `PointSet::regrid_min_curvature` (warm-started incremental min-curvature
  re-grid on a prior surface's lattice, honouring control points as hard
  constraints).
- **`GeoData::model_inputs()` implemented** — assembles the full
  normalize→validate→interpret→characterise pipeline across a project into the
  `ModelInputs` contract (summary scalars as `Uncertain`, horizons, canonical
  well curves, boundary). `GeoData::surfaces_named`/`polygons_named` accessors;
  `Surface` and `PolygonSet` are now `Clone`.
- `WellCurveInput.xyz` — each curve sample positioned to world `(x, y, z=TVD)`
  via the trajectory, so consumers can upscale logs onto grid cells without
  touching positioning (which is petekio's responsibility).

## [0.2.0] - 2026-06-28

### Added
- Python wheel (`py`, PyO3): grew the `petekio` bindings from the early
  Surface+Stats layer to mirror `API.md` §"Python (PyO3) surface".
  - `Surface`: `load_irap_classic`/`constant`/`save_irap_classic`, `sample`,
    `resample`, element-wise math (`ln`/`log10`/`exp`/`sqrt`/`abs`/`powf`/
    `clamp_min`/`clamp`), named surface↔surface forms (`plus`/`minus`/`times`/
    `divided_by`) and the `+ - * /` operator overloads (scalar **and**
    surface↔surface, raising on geometry mismatch; reflected scalar forms),
    `thickness` (staticmethod), `stats`/`area_below`/`area_above`/
    `volume_between`/`hypsometry`, attribute access via `surface.attr["name"]`
    /`surface.attr(name)` (promotes to a `Surface`) + `attr_names`/`set_attr`,
    and `geometry`/`ncol`/`nrow`/`rotation_deg`/`bbox` getters.
  - `GridGeometry` (constructable) and `BBox` value types; `Stats` fields stay
    read-only attributes with `percentile`.
  - Numpy/`ndarray` exposure is out of scope (no numpy dependency): attribute
    layers are returned as promoted `Surface`s, never raw arrays.
  - `PointSet`: `load_csv`/`load_geojson`/`load_irap_points` classmethods,
    `len`, `attr` (→ `list[float]`), `stats`, `bbox`, `nearest`, and
    `to_surface(geom, method)` with `method` a string (`"nearest"`/`"idw"`/
    `"min_curvature"`, IDW default).
  - `PolygonSet`: `load_geojson`/`load_irap_polygons`/`load_shapefile`,
    `contains`, `area`, `bbox`, `clip`.
  - `GeoData(unit="ft"|"m")`: `load_surface` (→ owned `Surface`),
    `load_points`/`load_polygons` (→ views), named getters
    `surface`/`points`/`polygons` (miss → `None`), `surfaces()`, and the `unit`
    getter. Points/polygons hand back lightweight views that re-resolve into
    the project; surfaces are deep-cloned owned copies.
  - `Well`/`Interval`/`LogView`: `GeoData.load_well`/`well`/`wells` plus
    `well.xyz`/`tvd`/`md_at_tvd`, `well.top(name)` → `Interval`,
    `well.log(mnemonic)` → `LogView`; `Interval.top_md`/`base_md`/`name`/
    `thickness_md`/`log`; `LogView.stats`/`values`/`md`/`at_md`/`len`. The
    headline dynamic chain: `w.brent` → `Interval` and `w.brent.ntg` /
    `w.brent.phie.mean` → `Stats` via `__getattr__` (unknown names fall back to
    `AttributeError`). Wells are views into the project.
  - `WellsView` broadcast (`geo.wells`): `filter(predicate)` (a Python callable
    over `Well`), `tops(name)`, `iter()`/`len`, and `__getattr__` broadcast —
    `geo.wells.tops("Brent").ntg` (or `geo.wells.brent.ntg`) yields a per-well
    `list[Stats]`.
- `GeoData` (`manager`): the load-once project substrate. Named, insertion-
  ordered collections under one `Unit`; `new`, fluent loaders returning `&T`
  (`load_surface` IRAP classic; `load_points`/`load_polygons` extension-
  dispatched over the supported formats; `load_well` from a directory or single
  file — `*.las` → logs, tops `*.csv` (`name`,`md`) → tops on the main bore, with
  a vertical trajectory synthesized from the log MD span); named getters
  `surface`/`well`/`points`/`polygons` (miss → `None`); `surfaces()` iterator and
  `wells() -> WellsView`.
- `WellsView<'a>` (`manager`): a lightweight, broadcastable borrow over a
  project's wells (no cloning) — `filter(pred)`, `iter()`, and `tops(name)`
  (narrow to wells carrying that marker), plus `len`/`is_empty`. The substrate
  behind the per-well `Stats` broadcast.
- `PointSet` (`core`): scattered N×3 points with named `f64` attribute columns.
  Loaders `load_csv` (named X/Y/Z columns; other numeric columns → attributes)
  and `load_irap_points` (RMS plain `X Y Z`); ops `len`/`is_empty`/`filter`/
  `attr`/`stats`/`bbox`/`nearest` (rstar R*-tree, areal). Gridding
  `to_surface(geom, GridMethod)` with `GridMethod::{Nearest, InverseDistance,
  MinimumCurvature}` — Nearest + IDW (p=2, exact at data) are full; minimum
  curvature is a biharmonic (∇⁴z=0) SOR relaxation anchored at data nodes
  (interior 13-point stencil, near-edge 5-point harmonic fallback). New deps:
  `geo`, `rstar`.
- `PolygonSet` (`core`): polygon rings backed by `geo` predicates. Loader
  `load_irap_polygons` (RMS rings split on the `999.0` sentinel); ops
  `contains(x,y)` (point-in-polygon, **boundary-exclusive** per `geo`),
  `area()` (Σ `unsigned_area`), `bbox()`, and `clip(&Surface)` (masks nodes
  outside all polygons → `NaN`).
- Vector IO (`io`): GeoJSON + ESRI shapefile via `geozero` 0.15 —
  `PointSet::load_geojson` (a streaming `FeatureProcessor` carries each
  feature's **numeric** `properties{}` into attribute columns, NaN-filling the
  schemaless union; strings dropped), `PolygonSet::load_geojson`, and
  `PolygonSet::load_shapefile`. New dep: `geozero`
  (`with-geo`/`with-geojson`/`with-wkt`/`with-shp`).
- Well logs (`core`): `Log` (MD-indexed curve, `new`/`len`/`view`) and
  `LogView<'a>` — a borrowed-or-owned (`Cow`) window with `stats`,
  `stats_weighted` (element-wise PV-weighting), `filter`, `at_md` (linear
  interpolation), `resample(step)`, `values`/`md`. NaN = undefined throughout.
- Tops + intervals (`core`): `Top` (name + MD) and `Interval<'a>` (`log` clips a
  curve to `[top_md, base_md)`, `thickness_md`). Wired into `Sidetrack`
  (`add_log`/`add_tops`/`top`/`log`) and `Well` (delegating `top`/`log` to the
  main bore): tops sort by MD, the interval base is the next top's MD or total
  depth, enabling the `well.top("Brent")?.log("NTG")?.stats()` chain.
- IO (`io`): LAS reader via `las_rs` 0.2 (`Log::load_las` / `load_las_all`,
  NULL→NaN, shared index/MD curve) and a headered tops CSV reader via `csv`
  (`Top::load_csv`, name/MD columns by header). New deps: `las_rs`, `csv`.
- Well geometry (`core`): `Station` and the `TrajectoryInput` survey variants
  (`Xyz`/`MdIncAzi`/`Stations`/`Hold`/`Steer`), normalized to a positioned
  `Trajectory` via the **minimum-curvature** method (dogleg β + ratio-factor with
  a β→0 Taylor guard). `Trajectory` exposes `xyz`/`tvd`/`md_at_tvd`/`md_range`
  with linear interpolation and shallowest-crossing TVD inversion.
- `Well` → `Sidetrack` hierarchy (`core`): `Well::new`/`sidetrack`/
  `sidetrack_mut` (lazy bore creation)/`main`/`sidetracks`; `Sidetrack`
  `add_trajectory` (newest → active)/`set_active`/`active`/`trajectories`.
  `Well` and `Sidetrack` delegate `xyz`/`tvd`/`md_at_tvd` to the main/active
  trajectory. (Tops/logs deferred to Phase 4.)

## [0.1.0] - 2026-06-28

### Added
- Project scaffolding: single layered `petekio` crate
  (`foundation → io → core → analysis → manager → py`), MSRV 1.88, `py`/`f32`
  feature placeholders.
- `foundation` layer: `GeoError`/`Result`, `Unit` (+conversions), `Point3`,
  `BBox`, `GridGeometry` (rotation + `yflip`, with `node_xy`/`xy_to_ij`/`bbox`),
  and NaN-skipping `Stats` (`of`/`weighted`/`percentile`).
- `Surface` (`core`): construction (`new`/`constant`), attribute layers
  (`attr`/`set_attr`/`attr_names`/`as_attr_surface`), and **IRAP-classic** read/
  write (`load_irap_classic`/`save_irap_classic`).
- `Surface::sample` (strict NaN-aware bilinear) + `Surface::resample` onto a
  target geometry.
- `Surface` math (immutable, NaN-propagating): element-wise `ln`/`log10`/`exp`/
  `sqrt`/`abs`/`powf`/`clamp_min`/`clamp`; surface↔surface `plus`/`minus`/
  `times`/`divided_by` + `thickness`; operator overloads (scalar `+ - * /` →
  `Surface`, surface `+ - * /` → `Result<Surface>`).
- `Surface` statistics/volumetrics: `stats`, `area_below`/`area_above`,
  `volume_between`, and the `hypsometry` (area-vs-depth) curve.
- **Python bindings** (the `petekio` wheel, via PyO3/abi3 + maturin): a thin
  layer exposing `Surface` (`load_irap_classic`/`save_irap_classic`/`sample`/
  `area_below`/`area_above`/`stats`, `ncol`/`nrow`/`rotation_deg`) and `Stats`.
