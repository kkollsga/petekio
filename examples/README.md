# petekIO examples

Runnable Jupyter notebooks demonstrating the Python API on small bundled data.

| Notebook | Covers |
|----------|--------|
| [`01_surfaces.ipynb`](01_surfaces.ipynb) | Load an IRAP horizon; sample; resample onto another lattice; surface arithmetic + isochore thickness; stats / `area_above` / hypsometry; grid scattered points (`minimum_curvature`); optional `matplotlib` view. |
| [`02_wells.ipynb`](02_wells.ipynb) | Load a well (LAS + tops); position MDs via the trajectory (`xyz`/`tvd`); read logs (`LogView`); tops → intervals and the fluent `well.<top>.<log>` → `Stats`; broadcast across a project with `GeoData.wells`. |

## Setup

```bash
pip install petekio matplotlib        # matplotlib only for the optional plot
# or, from a checkout of this repo:
maturin develop --release
```

Run the notebooks **from this `examples/` folder** so the relative paths under
`data/` resolve.

## Data — external (not in the repo)

The sample data lives in the **external data folder**, not the repo. The
notebooks resolve it via the `PETEKIO_TEST_DATA` environment variable (default
`/Volumes/EksternalHome/Data/modellingProject/petekio-fixtures`), reading
`$PETEKIO_TEST_DATA/examples-data/`:

```bash
export PETEKIO_TEST_DATA=/path/to/your/data/folder   # contains examples-data/
```

`examples-data/` holds: `horizon_top.irap` (tiny IRAP surface), `scatter_points.csv`
(scattered `x,y,depth` + `poro`), and `wells/15_9-A1/` (a small well: `sample.las`
+ `tops.csv`). Point `DATA` at your own files to run on real data.
