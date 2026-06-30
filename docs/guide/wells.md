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
w.md_at_tvd(2300)    # inverse
```

`z` is subsea TVD (`md − kb` for a vertical hole). A standalone trajectory can be
built without a project via `petekio.Trajectory.from_stations(...)`.

## Logs

```python
bore = w.sidetrack("A")
bore.mnemonics()                 # curve names, in insertion order
bore.log_stats("PHIE").mean      # whole-bore NaN-skipping stats
```

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

### A tidy table across bores — `zone_table`

For a per-`zone × bore` table, `zone_table` returns a ready
[pandas](https://pandas.pydata.org/) DataFrame — no manual loop/pivot/reorder:

```python
t = w.zone_table("PHIE", stats=("mean", "p50", "p90"))   # columns: zone, bore, mean, p50, p90
t.pivot(index="zone", columns="bore", values="mean")     # zone keeps lithostratigraphic order
geo.wells.zone_table("PHIE")                              # multi-well; bore = "<well> <sidetrack>"
```

`stats` are `Stats` attribute names (default `["mean"]`). `zone` is an ordered
Categorical in lithostratigraphic order, so it survives `pivot`/`groupby`;
zero-thickness / no-sample cells drop out unless `include_empty=True`. Needs
pandas — `pip install petekio[pandas]`.

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

See the [API reference](../api/reference.md#well) for the full `Well` /
`Sidetrack` surface.
