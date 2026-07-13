# Wells

A `Well` is a surface location (`head`), a datum (`kb`), and one or more named
**bores** (sidetracks; the unnamed `""` is the main bore). Each bore owns its
trajectory, logs, and formation tops.

## Loading

`load_well` ingests a per-well directory (walked recursively, so a Petrel export
tree with separate `Paths/`/`Logs/` subdirs works) or a single file, dispatched
by extension:

- `*.wellpath` → a **positioned** trajectory, one bore per file (MD preserved
  exactly; position interpolated by minimum curvature).
- `*.las` → logs, routed to the matching bore, with mnemonic aliasing.

```python
# head/kb are optional — a .wellpath header is authoritative and fills them.
geo.load_well("15/9-A1", files="wells/15_9-A1/")

w = geo.well("15/9-A1")
w.head           # (x, y) from the wellpath header
w.kb             # kelly-bushing datum
w.crs            # CRS label (provenance only; petekIO never reprojects)
w.bores()        # e.g. ["", "A", "B", "ST2"]
```

## Geometry

```python
w.xyz(2450)          # interpolated (x, y, z) at measured depth; z = subsea TVD
w.tvd(2450)          # subsea TVD at MD
w.md_at_tvd(2190)    # inverse
```

`z` is subsea TVD (`md − kb` for a vertical hole). A standalone trajectory can be
built without a project via `petekio.Trajectory.from_stations(...)`.

## Logs

```python
bore = w.sidetrack("A")
bore.mnemonics()                 # curve names, in insertion order
bore.log_stats("PHIE").mean      # whole-bore NaN-skipping stats
```

### Calculated logs and basis alignment

After import, LAS files are no longer modelling containers. Each imported curve
is a standalone log on a bore with its own `md` and `values` arrays. Log
arithmetic is strict by default: two curves can be combined directly only when
they are on the same bore and have identical MD sampling. If sampling differs,
declare the output basis or resample an operand explicitly.

```python
logs = project.wells.logs

# Strict: PHIE and NetSand must already share MD sampling on each bore.
project.wells.assign_log("PHIE_NET", logs.PHIE * logs.NetSand)

# Output basis is PHIE; non-basis operands are resampled to PHIE.
project.wells.assign_log(
    "PHIE_NET",
    logs.PHIE * logs.NetSand,
    basis=logs.PHIE,
    interpolation="previous",
)

# Operand-local resampling wins and makes the expression self-contained.
project.wells.assign_log(
    "PHIE_NET",
    logs.PHIE * logs.NetSand.to_basis(logs.PHIE, interpolation="spline"),
)
```

Supported interpolation names are `nearest`/`closest`, `linear`,
`previous`/`ffill`, `next`/`bfill`, and `spline`/`cubic`. When the
`petektools` wheel is installed, resampling delegates to its Rust `interp1d`
kernel; `spline` means a natural cubic spline.

`assign_log` runs across all wells/bores that contain the required logs. Missing
logs are skipped and reported; basis mismatches or duplicate output names raise.
The result is a small report:

```python
result = project.wells.assign_log("BVPHI", logs.PHIE * (1 - logs.SW))
result.summary()
result.created
result.skipped
```

The calculation basis is the curve's MD vector on a bore, not the input file it
came from. Curves imported from separate files can combine directly when their
MD vectors match; curves from the same file still fail if their sampling differs
after normalization.

## Tops and zones

`load_well_tops` loads a multi-well Petrel well-tops file and routes each
`Horizon` pick to the matching well + bore. `Other`-type picks (fluid contacts —
OWC/GOC/FWL) are excluded; they aren't lithostratigraphy.

A **zone** is the interval between consecutive tops: `[top_md, next_top_md)`, the
deepest running to total depth.

```python
geo.load_well_tops("WellTops.tops")

bore.zones()                          # [(name, top_md, base_md), ...]
bore.zone_stats("PHIE")               # [(zone, Stats), ...]
bore.zone_stats("PHIE", "Top A").mean # one zone's Stats directly (None if absent)
```

`Stats` carries the average (`mean`), `sum`, count, and percentiles; zones where
the curve has no samples are omitted from `zone_stats`.

### Intersecting surfaces and storing picks

All three surface levels (`Surface`, `StructuredMeshSurface`, and `TriSurface`)
intersect the actual minimum-curvature bore trajectory:

```python
hits = bore.intersections(surface, tolerance=1e-3)  # MD ordered, pure
hit = bore.intersection(surface)                     # None, one, or loud multiple-hit error
hit.md, hit.xyz, hit.well, hit.bore, hit.surface
hit.to_dict()

bore.add_top("Top reservoir", hit)
bore.replace_top("Top reservoir", 2412.5)
bore.remove_top("Top reservoir")
bore.tops()
```

Null triangles are holes, outside trajectories are no-hit, shared triangle
edges de-duplicate, tangencies are retained, and coplanar overlap is rejected
because it has no discrete pick. `intersection` never chooses among multiple
crossings; call `intersections` and select one explicitly.

For a complete project horizon, use the aggregate report and atomic mapping:

```python
result = project.wells.intersection(surface)
result.summary()                 # hits / skipped / failed
project.well_tops["Reservoir/Top"] = result
project.well_tops["Reservoir/Top"].rows  # well, bore, md, xyz
del project.well_tops["Reservoir/Top"]
```

Outside/no-hit skips are accepted; any failure blocks assignment. The complete
right-hand side is validated first (same project, full wells view, one hit per
bore, matching MD/XYZ), then the horizon is replaced and stale picks removed.
`project.tops` remains the source-table view from raw imports;
`project.well_tops` is reconstructed from `.pproj` Well records.

### A tidy table across bores — `zone_table`

For a per-`zone × bore` table, `zone_table` returns a ready
[pandas](https://pandas.pydata.org/) DataFrame — no manual loop/pivot/reorder:

```python
t = w.zone_table("PHIE", stats=("mean", "p50", "p90"))   # tidy: zone, bore, mean, p50, p90
w.zone_table("PHIE", pivot=True, decimals=3)             # wide: zone index × bore columns, rounded
w.zone_table("PHIE", aggregate=True)                     # grouped: pooled "all" row first, then per bore
w.zone_table("PHIE", zones=["Upper Sand", "Mid Sand"])  # keep only these zones
geo.wells.zone_table("PHIE")                             # multi-well; bore = "<well> <sidetrack>"
```

`stats` are `Stats` attribute names (default `["mean"]`), plus **`samples`**
(sample count) and **`gross`** (the zone's MD thickness). `zones=` keeps only the
named zones (case-insensitive); `decimals=N` rounds. `zone` is an ordered
Categorical in lithostratigraphic order, so it survives `pivot`/`groupby`;
zero-thickness / no-sample cells drop out unless `include_empty=True`.

| Argument | Effect |
| --- | --- |
| `stats=(...)` | which `Stats` attrs (+ `samples`, `gross`) become columns |
| `zones=[...]` | keep only these zones |
| `pivot=True` | wide: `zone` index × `bore` columns (multi-stat → MultiIndex) |
| `aggregate=True` | grouped by zone, pooled **all** row first; `(zone, bore)` index |
| `weighted=False` | plain sample mean instead of thickness-weighted |
| `decimals=N` | round the stat values |
| `include_empty=True` | keep zero-thickness / no-sample cells |

Averages are **thickness-weighted by default** — each sample is weighted by the
MD span it represents, so a finely-sampled log doesn't outweigh a coarse one over
the same interval (uniform sampling is a no-op). Pass `weighted=False` for the
plain sample mean. `aggregate=True` adds a pooled **all** row per zone (the
thickness-weighted average across bores). Needs pandas —
`pip install petekio[pandas]`.

## Lithostratigraphic ordering

Zones are returned in **true stratigraphic order**, not merely measured-depth
order. At `load_well_tops` time petekIO reads *every* well in the tops file and
merges their relative orderings into one field-wide column:

- A marker that is strictly shallower than another in *any* well establishes that
  order.
- A marker that pinches out (zero thickness — coincident MD) in one well carries
  no ordering information there, but is ordered by a well that **develops** it.
- Ties no well resolves fall back to file order, then insertion — best-effort,
  never failing.

```python
geo.strat_order      # the merged column, e.g. ["Top A", "Sand A", "Mid", ...]
```

The merge changes only the *order* zones are presented in — each zone's
`[top_md, base)` geometry is untouched.

!!! example "Why field-wide"
    A sand that is present (thick) in one well but pinched out (zero thickness)
    in another can't be ordered from the pinched-out well alone — its MD ties the
    marker above it. Reading the whole field lets the well that develops the sand
    supply the order the others lack.

### Manual ordering hints

Some markers are coincident in **every** well — two stacked lobes, say — so no
depth data can order them. A **hint** resolves the stalemate. It is honoured
*only* where the data leaves the pair unordered: any real MD relationship always
wins, so a hint can never override geology.

```python
geo = petekio.GeoData(unit="m")
geo.strat_hint("Upper Sand < Lower Sand")          # "A < B" = A above B
geo.strat_hint(above="Upper Sand", below="Lower Sand")  # the explicit form
geo.load_well_tops("WellTops.tops")                # hints apply at load time
```

`A < B` reads "A above B", `A > B` reads "A below B". Names may be partial
(resolved at load: exact → `… top` → unique substring; an ambiguous or unmatched
token raises). When two coincident tops are *stacked* — one sits in the interval
the other bounds — the stratigraphically **lowest** of the cluster owns the
interval its samples fall in, so per-zone stats stay correct after a re-order.

## A worked example

The [`well_example.ipynb`](https://github.com/kkollsga/petekio/blob/main/examples/well_example.ipynb)
notebook runs this whole path end-to-end on a small synthetic field — load →
`strat_order` → `zone_table` (tidy / pivot / aggregate / thickness-weighted) →
`strat_hint` — and is generated with the bundled `synthgen` helper.

See the [API reference](../api/reference.md#well) for the full `Well` /
`Sidetrack` surface.
