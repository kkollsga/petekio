# Project workspace

`Project.view()` opens a searchable folder-aware Map / 3-D / Wells workspace.
It is the petekIO-native form of `petektools.view(project)`.

```python
import petekio

project = petekio.Project.load("field.pproj")
session = project.view()
```

The first selected surface and all bore geometries are initially visible.
Points, polygons, other surfaces, well tops, templates, and opaque assets stay
in the tree but start off. Opening reads metadata only: no surface value layer,
trajectory, or log is gathered until its item is enabled.

## Select folders and attributes

```python
session = project.view(
    selection={
        "surfaces": ["Interpretation/Reservoir/"],
        "points": ["QC/Picks"],
        "wells": True,
    },
    visible={"map": ["surface:Interpretation/Reservoir/Top%20A"]},
    property={"surface:Interpretation/Reservoir/Top%20A": "thickness"},
)
```

Folder selectors end in `/` and expand in collection order. Full canonical
paths are safe when different folders contain the same leaf name; ambiguous
leaf-only selectors fail with guidance. Surface depth and named attributes are
lanes of one surface item, so switching `thickness` loads that lane once.

Regular surfaces stay compact: Map receives the affine grid plus typed values
and mask, not expanded nodes and triangles. The 3-D resource opens with a
bounded preview spanning the complete surface footprint and refines to full
detail without changing the camera. A named attribute colours the surface but
the primary depth lane still defines elevations and geometry holes.

Selected well paths on a surface Map end at their first exact MD-ordered
surface crossing. The final display point is the intersection's exact MD/XYZ;
a no-hit well keeps its complete trajectory. Multiple crossings are reported
on the overlay while the display uses the first one. This is presentation only:
`bore.intersection(surface)` still raises on ambiguity, and callers use
`bore.intersections(surface)` to choose explicitly.

## Correlation logs and templates

```python
session = project.view(
    logs=petekio.ViewSpec(curves=("PHIE", "SW"), tops=True),
    template="qc/reservoir",
)
```

Without `logs=ViewSpec(...)`, each bore whose metadata advertises curves gets a
lazy Wells resource using all of its curves and tops. The resource starts hidden,
and catalog construction calls only `mnemonics()`; no log samples are gathered
until the bore is enabled in Wells. Passing `logs=ViewSpec(...)` explicitly
filters curves, tops, cutoffs, and flags. A template may be passed with either
the automatic curves or an explicit spec. Multi-bore wells use independent IDs
such as `well:A-1/bore:ST2`; no default bore is required or mutated. Unknown assets
remain preserved and appear disabled with a diagnostic.

## Inspect, refresh, and save

```python
session = project.view(settings=petekio.ViewSettings(serve=False))
session.tree()                 # no petekTools import required
session.diagnostics
session.refresh()              # rebuild after rename/delete/add
session.serve()
session.save("visible.html")
session.save("complete.html", include="selected")
```

`include="visible"` embeds only initially visible resources and their active
lanes. `include="selected"` materializes every selected item and declared lane
for offline toggling, so it can be much larger.
