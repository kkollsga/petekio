# petekIO examples

Runnable Jupyter notebooks demonstrating the Python API on small bundled data.

| Notebook | Covers |
|----------|--------|
| [`01_surfaces.ipynb`](01_surfaces.ipynb) | Load an IRAP horizon; sample; resample onto another lattice; surface arithmetic + isochore thickness; stats / `area_above` / hypsometry; grid scattered points (`minimum_curvature`); optional `matplotlib` view. |
| [`02_wells.ipynb`](02_wells.ipynb) | Load a well (LAS + tops); position MDs via the trajectory (`xyz`/`tvd`); read logs (`LogView`); tops → intervals and the fluent `well.<top>.<log>` → `Stats`; broadcast across a project with `GeoData.wells`. |
| [`well_example.ipynb`](well_example.ipynb) | The reservoir-quality showcase: a multi-bore field; the field-wide lithostratigraphic column (`strat_order`); per-zone `zone_table` (tidy / `pivot` / `aggregate`); **thickness-weighted** averages; the `zones=` filter; and a manual `strat_hint`. Needs `pandas` (`pip install petekio[pandas]`). |

## Synthetic data generator — `synthgen.py`

[`synthgen.py`](synthgen.py) writes a small **synthetic** field (multi-bore well;
a lithostratigraphic column with a pinch-out + coincident stacked lobes; `PHIE`
logs with a mixed sampling rate) to Petrel/LAS spec — `make_field(out_dir)`
returns the paths. `well_example.ipynb` uses it; it will grow to cover surfaces /
points / polygons. Dump a field to disk with `python synthgen.py <dir>`.

## Setup

```bash
pip install petekio matplotlib        # matplotlib only for the optional plot
# or, from a checkout of this repo:
maturin develop --release
```

Run the notebooks **from this `examples/` folder** so the relative paths under
`data/` resolve.

## Data — `data/` (synthetic, in repo)

The notebooks read tiny **synthetic** sample files under `examples/data/` (the
`DATA` variable, default `"data"`; override with `PETEKIO_TEST_DATA` to point at
your own files). These are hand-authored fixtures, **not** real data:

- `horizon_top.irap` — a tiny IRAP-classic depth surface.
- `scatter_points.csv` — scattered `x,y,depth` (+ a `poro` column) for gridding.
- `wells/15_9-A1/` — a small well: `sample.las` + `tops.csv`.
