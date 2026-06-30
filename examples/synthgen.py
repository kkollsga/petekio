"""Synthetic subsurface data generator for petekIO examples and demos.

Writes a tiny, fully **synthetic** field to Petrel / LAS format spec — no real
data. `make_field(out_dir)` returns the paths a notebook (or a test) then loads
through petekIO. The field is deliberately shaped to exercise the interesting
features:

- a **multi-bore** well (`25/1-A` bores A/B) plus single-bore wells,
- a lithostratigraphic column with a **pinch-out** (`Mid Sand` is developed in
  some wells, zero-thickness in another) so the cross-well order merge has work
  to do — and two stacked lobes (`Lower Sand A`/`B`) coincident *everywhere*, so
  a manual `strat_hint` is needed to order them,
- `PHIE` logs with believable zone trends, with two bores sampled at **different
  rates** so thickness-weighting visibly matters.

Designed to expand: drop new `write_*` helpers + `make_*` builders here as
surfaces / points / polygons land (see the TODO at the bottom).
"""

from __future__ import annotations

import random
from pathlib import Path

# --- format writers ---------------------------------------------------------


def _write(path: Path, body: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    return path


def write_wellpath(path: Path, head: tuple[float, float], kb: float, crs: str,
                   stations: list[tuple[float, float, float, float]]) -> Path:
    """A positioned `.wellpath` (vertical here): rows are MD X Y Z TVD ..."""
    rows = "".join(
        f"{md} {x} {y} {z} {md} 0 0 0 0 0 0\n" for (md, x, y, z) in stations
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

    Wells: ``25/1-A`` (bores A, B — A sampled fine, B coarse), ``25/1-B``,
    ``25/1-C`` (where ``Mid Sand`` pinches out). Tops include ``Coal``,
    coincident with ``Lower Sand`` everywhere (needs a ``strat_hint``).
    """
    out = Path(out_dir)
    crs = "ED50 / UTM zone 31N"

    # Per-well/bore tops (name, md). Each shifted in depth; Mid Sand pinches out
    # in 25/1-C (== Lower Sand md); Coal is coincident with Lower Sand in all.
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
        if bore:  # multi-bore needs a .wellpath per bore (shared-prefix labels)
            wp = wdir / f"{wid.replace('/', '_')}{suffix}.wellpath"
            stations = [(base - 50, head[0], head[1], -(base - 50)),
                        (td, head[0], head[1], -td)]
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


# TODO(expand): write_irap_surface / write_points_csv / write_cps3_polygon +
# make_surfaces / make_points / make_polygons, returned from make_field(), as
# those datatypes come online — keeping one generator for the whole demo set.


if __name__ == "__main__":
    import sys
    dest = sys.argv[1] if len(sys.argv) > 1 else "data/synthetic"
    p = make_field(dest)
    print("wrote synthetic field to", Path(dest).resolve())
    for k, v in p.items():
        print(f"  {k}: {v}")
