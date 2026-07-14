"""Synthetic subsurface data generator for petekIO examples and demos.

Writes a tiny, fully **synthetic** field to Petrel / LAS format spec — no real
data. `make_field(out_dir)` returns the paths a notebook (or a test) then loads
through petekIO. The field is deliberately shaped to exercise the interesting
features:

- a **multi-bore** well (`25/1-1` bores A/B) plus single-bore wells,
- a lithostratigraphic column with a **pinch-out** (`Mid Sand` is developed in
  some wells, zero-thickness in another) so the cross-well order merge has work
  to do — and two stacked lobes (`Lower Sand A`/`B`) coincident *everywhere*, so
  a manual `strat_hint` is needed to order them,
- `PHIE` logs with believable zone trends, with two bores sampled at **different
  rates** so thickness-weighting visibly matters,
- a viewer-design variant with a **rotated** regular surface, two continuous
  attribute lanes, and a categorical facies lane with an explicit code table.

Designed to expand: drop new `write_*` helpers + `make_*` builders here as
surfaces / points / polygons land (see the TODO at the bottom).
"""

from __future__ import annotations

import math
import random
from pathlib import Path

_WellPathStation = tuple[float, float, float, float, float, float]

# --- format writers ---------------------------------------------------------


def _write(path: Path, body: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    return path


def write_wellpath(path: Path, head: tuple[float, float], kb: float, crs: str,
                   stations: list[_WellPathStation]) -> Path:
    """Write positioned stations as ``(MD, X, Y, TVD, inclination, azimuth)``."""
    rows = "".join(
        f"{md} {x} {y} {kb - tvd} {tvd} 0 0 {azi} {inc} 0 {azi}\n"
        for (md, x, y, tvd, inc, azi) in stations
    )
    return _write(path, (
        "# WELL TRACE FROM PETREL\n"
        f"# WELL HEAD X-COORDINATE: {head[0]} (m)\n"
        f"# WELL HEAD Y-COORDINATE: {head[1]} (m)\n"
        f"# WELL DATUM (KB, Kelly bushing, from MSL): {kb} (m)\n"
        f"# CRS: {crs}\n=====\n"
        "MD X Y Z TVD DX DY AZIM_TN INCL DLS AZIM_GN\n" + rows
    ))


def write_las(path: Path, mnemonic: str, unit: str,
              samples: list[tuple[float, float]]) -> Path:
    """A LAS 2.0 curve: depth + one mnemonic."""
    strt, stop = samples[0][0], samples[-1][0]
    rows = "".join(f"{md:.4f} {v:.4f}\n" for md, v in samples)
    return _write(path, (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        f"~Well\n STRT.M {strt} :\n STOP.M {stop} :\n STEP.M 0 :\n NULL. -999.25 :\n"
        f"~Curve\n DEPT.M :\n {mnemonic}.{unit} :\n~ASCII\n" + rows
    ))


def write_tops(path: Path, rows: list[tuple[float, str, str]],
               kind: str = "Horizon") -> Path:
    """A Petrel multi-well tops file. `rows` = (md, surface, well)."""
    header = ("# Petrel well tops\nVERSION 2\nBEGIN HEADER\n"
              "X\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n")
    body = "".join(
        f'1 2 -1 -999 -999 -999 {md} -1 {kind} "{surface}" "{well}"\n'
        for md, surface, well in rows
    )
    return _write(path, header + body)


# --- field model ------------------------------------------------------------

# The intended column, shallow → deep. `phi` is the zone's mean porosity.
# `Lower Sand A`/`Lower Sand B` are stacked lobes, coincident in every well, so
# the data can't order them — a `strat_hint` does. `Lower Sand A` is listed last
# so it governs the shared interval's modelled porosity.
_COLUMN = [
    ("Top Shale", 0.06),
    ("Top Reservoir", 0.10),
    ("Upper Sand", 0.24),
    ("Mid Sand", 0.26),
    ("Lower Sand B", 0.20),
    ("Lower Sand A", 0.23),
    ("Base Reservoir", 0.09),
]


def _phi_for_md(md, tops, rng, bias=0.0) -> float:
    """Porosity at `md`: the enclosing zone's mean + a per-bore bias + noise."""
    phi = 0.05
    for (name, base_phi), (_, top_md) in zip(_COLUMN, _zip_tops(tops)):
        if md >= top_md:
            phi = base_phi
    return max(0.0, phi + bias + rng.uniform(-0.012, 0.012))


def _zip_tops(tops):
    # tops as (name, md), aligned to _COLUMN order; missing names skipped.
    by_name = {n: md for n, md in tops}
    return [(n, by_name.get(n, float("inf"))) for n, _ in _COLUMN]


def _log(path: Path, top_md: float, td: float, step: float,
         tops: list[tuple[str, float]], seed: int, bias: float = 0.0) -> Path:
    rng = random.Random(seed)
    n = int((td - top_md) / step) + 1
    samples = [(top_md + i * step, _phi_for_md(top_md + i * step, tops, rng, bias))
               for i in range(n)]
    return write_las(path, "PHIE", "m3/m3", samples)


def make_field(out_dir: str | Path) -> dict[str, Path]:
    """Generate the synthetic field under `out_dir`. Returns key paths.

    Wells: ``25/1-1`` (bores A, B — A sampled fine, B coarse), ``25/1-2``,
    ``25/1-3`` (where ``Mid Sand`` pinches out). ``Lower Sand A`` and
    ``Lower Sand B`` are coincident everywhere (needs a ``strat_hint``).
    """
    out = Path(out_dir)
    crs = "ED50 / UTM zone 31N"

    # Per-well/bore tops (name, md). Each shifted in depth; Mid Sand pinches out
    # in 25/1-3 (== Lower Sand md); the two Lower Sand lanes are coincident.
    def column(base: float, pinch_mid: bool) -> list[tuple[str, float]]:
        ls = base + 120
        return [
            ("Top Shale", base + 0),
            ("Top Reservoir", base + 50),
            ("Upper Sand", base + 70),
            ("Mid Sand", ls if pinch_mid else base + 100),
            ("Lower Sand B", ls),  # stacked with Lower Sand A → coincident, needs a hint
            ("Lower Sand A", ls),
            ("Base Reservoir", base + 160),
        ]

    # Ids carry no bore letter (A/B), so the LAS router never confuses the well
    # prefix with a bore label. Bore B is biased lower AND sampled coarser, so the
    # pooled average differs between thickness-weighted and plain sample mean.
    wells = [
        # (well id, bore, wellhead, base, sample step, pinch Mid, phi bias)
        ("25/1-1", "A", (1000.0, 5000.0), 2000.0, 0.15, False, 0.0),
        ("25/1-1", "B", (1000.0, 5000.0), 2010.0, 0.50, False, -0.06),
        ("25/1-2", "", (3000.0, 5200.0), 2120.0, 0.15, False, 0.0),
        ("25/1-3", "", (5000.0, 5400.0), 2240.0, 0.15, True, 0.0),  # Mid Sand pinches out
    ]

    paths: dict[str, Path] = {}
    top_rows: list[tuple[float, str, str]] = []
    main_block = ["Top Shale", "Top Reservoir", "Upper Sand", "Base Reservoir"]
    # `Lower Sand B` listed before `Lower Sand A` → file order alone puts B above
    # A. The two are coincident in every well, so only a strat_hint can reorder
    # them — exactly the demo we want.
    member_block = ["Mid Sand", "Lower Sand B", "Lower Sand A"]

    for seed, (wid, bore, head, base, step, pinch, bias) in enumerate(wells):
        tops = column(base, pinch)
        td = base + 200
        wdir = out / "wells" / wid.replace("/", "_")
        suffix = f"_{bore}" if bore else ""
        well_col = f"{wid} {bore}" if bore else wid
        # Every synthetic well carries a positioned path, so downstream views
        # exercise the declared wellhead instead of a (0, 0) loader fallback.
        wp = wdir / f"{wid.replace('/', '_')}{suffix}.wellpath"
        if bore == "A":
            # A genuinely deviated positioned bore: the endpoint has a
            # falsifiable horizontal departure from the header wellhead.
            stations = [
                (base - 50, head[0], head[1], base - 50, 0.0, 40.0),
                (base + 60, head[0] + 25, head[1] + 20, base + 55, 22.0, 40.0),
                (td, head[0] + 140, head[1] + 110, td - 35, 38.0, 40.0),
            ]
        else:
            stations = [
                (base - 50, head[0], head[1], base - 50, 0.0, 0.0),
                (td, head[0], head[1], td, 0.0, 0.0),
            ]
        write_wellpath(wp, head, 25.0, crs, stations)
        _log(wdir / f"{wid.replace('/', '_')}{suffix}_PHIE.las",
             base - 40, td, step, tops, seed=seed, bias=bias)
        for name, md in tops:
            top_rows.append((md, name, well_col))
        paths.setdefault(wid, wdir)

    # Order rows: main block first (by column), then member block — mimics Petrel
    # appending sand/coal members, which misorders Coal until the merge + hint.
    def block_key(row):
        _, name, _ = row
        b = 0 if name in main_block else 1
        order = (main_block if b == 0 else member_block).index(name)
        return (b, order)

    top_rows.sort(key=block_key)
    paths["tops"] = write_tops(out / "tops" / "field.tops", top_rows)
    paths["wells_dir"] = out / "wells"
    return paths


def write_irap_surface(path: Path, xori: float, yori: float, xinc: float,
                       yinc: float, ncol: int, nrow: int, fn, *,
                       rotation_deg: float = 0.0, yflip: bool = False) -> Path:
    """An IRAP-classic ASCII surface. `fn(i, j)` -> value (return NaN for undef).

    Matches `io::irap::save_irap_classic`: header lines then values column-major,
    x-fastest, 6 per line; undefined cells use the 9999900 sentinel.
    """
    undef = 9999900.0
    xmax, ymax = xori + (ncol - 1) * xinc, yori + (nrow - 1) * yinc
    signed_yinc = -yinc if yflip else yinc
    lines = [
        f"-996 {nrow} {xinc} {signed_yinc}",
        f"{xori} {xmax} {yori} {ymax}",
        f"{ncol} {rotation_deg} {xori} {yori}",
        "0  0  0  0  0  0  0",
    ]
    toks = []
    for j in range(nrow):
        for i in range(ncol):
            v = fn(i, j)
            toks.append(f"{undef:.6f}" if v != v else f"{v:.6f}")  # v!=v -> NaN
    for k in range(0, len(toks), 6):
        lines.append(" ".join(toks[k:k + 6]))
    return _write(path, "\n".join(lines) + "\n")


def make_viewer_design_field(out_dir: str | Path) -> dict:
    """Build a small, falsifiable fixture for the project-view redesign.

    The attribute lanes are separate IRAP files with identical geometry. This
    keeps the fixture reusable before lane metadata is persisted by petekIO;
    callers can attach them with the existing ``Surface.set_attr`` operation.
    ``attributes`` supplies the intended durable unit/kind/code-table metadata
    without changing the current viewer transport schema.
    """
    paths = make_field(out_dir)
    out = Path(out_dir)
    surface_dir = out / "surfaces" / "viewer_design"
    geometry = {
        "xori": 850.0,
        "yori": 4850.0,
        "xinc": 75.0,
        "yinc": 60.0,
        "ncol": 7,
        "nrow": 6,
        "rotation_deg": 27.0,
        "yflip": False,
    }

    def lane(name: str, fn) -> Path:
        return write_irap_surface(
            surface_dir / f"{name}.irap",
            geometry["xori"],
            geometry["yori"],
            geometry["xinc"],
            geometry["yinc"],
            geometry["ncol"],
            geometry["nrow"],
            fn,
            rotation_deg=geometry["rotation_deg"],
            yflip=geometry["yflip"],
        )

    primary = lane("reservoir_top", lambda i, j: 1980.0 + 8.0 * i + 5.0 * j)
    attributes = {
        "gross_thickness": {
            "path": lane("gross_thickness", lambda i, j: 18.0 + 1.5 * i + 0.5 * j),
            "kind": "continuous",
            "unit": "m",
        },
        "porosity": {
            "path": lane("porosity", lambda i, j: 0.16 + 0.01 * ((i + j) % 6)),
            "kind": "continuous",
            "unit": "fraction",
        },
        "facies": {
            "path": lane("facies", lambda i, j: float(1 + ((i // 2 + j) % 3))),
            "kind": "categorical",
            "unit": None,
            "codes": {1: "Shale", 2: "Sand", 3: "Silt"},
        },
    }
    return {
        **paths,
        "surface": primary,
        "surface_geometry": geometry,
        "attributes": attributes,
    }


def write_points_csv(path: Path, coords: list[tuple[float, float, float, float]]) -> Path:
    """A headered scattered-point CSV (`x,y,z,poro`) — `poro` rides as an attr."""
    body = "x,y,z,poro\n" + "".join(
        f"{x:.2f},{y:.2f},{z:.2f},{p:.4f}\n" for x, y, z, p in coords
    )
    return _write(path, body)


# --- scalable benchmark field ----------------------------------------------

# A plain 8-marker column for the bench wells (order only; the stress is volume).
_BENCH_COLUMN = [
    "Sea Bed", "Top Shale", "Top Reservoir", "Upper Sand",
    "Mid Sand", "Lower Sand", "Base Reservoir", "Basement",
]


def make_bench_field(out_dir: str | Path, *, n_wells: int = 30, top: float = 100.0,
                     td: float = 4000.0, step: float = 0.0508, n_points: int = 100_000,
                     n_surfaces: int = 3, surface_n: int = 250,
                     seed: int = 0) -> dict:
    """A **scalable** synthetic field for perf benchmarking — deliberately heavy:
    **long, high-density** PHIE logs (small `step` → many samples/log), many
    wells, a big scattered point cloud, and IRAP surfaces.

    Each well is a single LAS in its **own subdir** (so a per-well load walks one
    directory, not the whole tree). Returns paths + well ids + `samples_per_log`.

    Defaults: 30 wells x ~76.8k samples (2-inch step over ~3.9 km) ≈ 2.3M log
    samples. Tune `step` down (e.g. 0.0254 = 1 inch) for a denser stress run.
    """
    out = Path(out_dir)
    rng = random.Random(seed)
    span = td - top
    samples_per_log = int(span / step) + 1
    well_ids: list[str] = []
    top_rows: list[tuple[float, str, str]] = []

    for w in range(n_wells):
        wid = f"B/1-{w + 1}"
        well_ids.append(wid)
        wtop = top + rng.uniform(0.0, 60.0)          # per-well depth shift
        n = int((td - wtop) / step) + 1
        # Long, dense PHIE: smooth regional trend + fine high-frequency texture.
        samples = []
        for i in range(n):
            md = wtop + i * step
            phi = 0.14 + 0.08 * math.sin(md / 220.0) + 0.03 * math.sin(md / 13.0)
            samples.append((md, max(0.0, phi + rng.uniform(-0.015, 0.015))))
        stem = wid.replace("/", "_")
        write_las(out / "wells" / stem / f"{stem}_PHIE.las", "PHIE", "m3/m3", samples)
        for k, name in enumerate(_BENCH_COLUMN):          # 8 markers down the hole
            top_rows.append((wtop + span * (k + 0.5) / len(_BENCH_COLUMN), name, wid))

    tops = write_tops(out / "tops" / "bench.tops", top_rows)
    pts = write_points_csv(out / "points" / "scatter.csv", [
        (rng.uniform(0, 5000), rng.uniform(0, 5000), rng.uniform(1000, 3000),
         rng.uniform(0.05, 0.30)) for _ in range(n_points)
    ])
    surfaces = [
        write_irap_surface(
            out / "surfaces" / f"h{s}.irap", 0.0, 0.0, 25.0, 25.0, surface_n, surface_n,
            lambda i, j, s=s: 1000.0 + 50.0 * s + 0.01 * i + 0.02 * j)
        for s in range(n_surfaces)
    ]
    return {
        "wells_dir": out / "wells",
        "well_ids": well_ids,
        "tops": tops,
        "points": pts,
        "surfaces": surfaces,
        "samples_per_log": samples_per_log,
        "n_wells": n_wells,
    }


if __name__ == "__main__":
    import sys
    dest = sys.argv[1] if len(sys.argv) > 1 else "data/synthetic"
    p = make_field(dest)
    print("wrote synthetic field to", Path(dest).resolve())
    for k, v in p.items():
        print(f"  {k}: {v}")
