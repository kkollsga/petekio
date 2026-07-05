"""pyo3 boundary hardening — GIL release, Surface view/copy-on-write sharing,
and the batched LogView accessors. Fixtures are synthetic / committed."""
import math
import threading
import time
from pathlib import Path

import petekio

FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures"
IRAP = str(FIXTURES / "simple.irap")


def _scatter(n):
    """A synthetic scattered PointSet: `n×n` jittered nodes over a 1 km square."""
    xs, ys, zs = [], [], []
    for i in range(n):
        for j in range(n):
            xs.append(i * 1000.0 / n + (j % 3))
            ys.append(j * 1000.0 / n + (i % 3))
            zs.append(-2000.0 - 40.0 * math.sin(i * 0.3) * math.cos(j * 0.2))
    return petekio.PointSet.from_xyz(xs, ys, zs)


def test_gil_released_during_gridding():
    """A background Python thread must make real progress while a compute-heavy
    min-curvature gridding call runs — proving the call releases the GIL. If the
    GIL were held for the whole call, the spinner would be starved (~0 ticks)."""
    ps = _scatter(24)  # 576 points
    geom = petekio.GridGeometry(0.0, 0.0, 6.0, 6.0, 160, 160)

    counter = [0]
    running = [True]

    def spin():
        while running[0]:
            counter[0] += 1

    t = threading.Thread(target=spin)
    t.start()
    try:
        time.sleep(0.02)  # let the spinner warm up
        before = counter[0]
        surf = ps.to_surface(geom, "min_curvature")  # heavy, GIL-released
        after = counter[0]
    finally:
        running[0] = False
        t.join()

    assert surf.ncol == 160 and surf.nrow == 160
    # With the GIL released the spinner advances thousands of times; held, ~0.
    assert after - before > 100, f"spinner starved ({after - before} ticks) — GIL not released"


def test_surface_view_sharing_and_cow():
    """A surface fetched from a project is a cheap view; mutating it via set_attr
    is copy-on-write and never writes back into the project (the same observable
    semantics as the former eager deep copy)."""
    geo = petekio.GeoData(unit="m")
    geo.load_surface("top", IRAP)

    s1 = geo.surface("top")
    s2 = geo.surface("top")
    # Two independent views agree on the underlying data.
    assert s1.stats().mean == s2.stats().mean
    assert s1.ncol == s2.ncol and s1.nrow == s2.nrow

    # Copy-on-write: adding an attribute to a fetched surface does NOT mutate the
    # project's stored surface.
    overlay = petekio.Surface.constant(s1.geometry, 7.0)
    s1.set_attr("overlay", overlay)
    assert "overlay" in s1.attr
    assert s1.attr["overlay"].stats().mean == 7.0
    # A freshly fetched view of the project shows no such attribute.
    assert "overlay" not in geo.surface("top").attr
    assert "overlay" not in geo.surfaces()[0].attr


def test_surface_math_on_project_view():
    """Element-wise + surface↔surface math works on project-backed views."""
    geo = petekio.GeoData(unit="m")
    geo.load_surface("top", IRAP)
    s = geo.surface("top")
    doubled = s * 2.0
    assert math.isclose(doubled.stats().mean, s.stats().mean * 2.0, rel_tol=1e-12)
    diff = petekio.Surface.thickness(s, s)  # base - top of identical = 0
    assert math.isclose(diff.stats().mean, 0.0, abs_tol=1e-9)


def _petro_well(tmp_path):
    las = (
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n"
        "~Well\n STRT.M 100.0 :\n STOP.M 130.0 :\n STEP.M 10.0 :\n NULL. -999.25 :\n"
        "~Curve\n DEPTH.M :\n PHIE.v/v :\n"
        "~ASCII\n"
        "100.0 0.20\n110.0 0.25\n120.0 0.30\n130.0 0.35\n"
    )
    (tmp_path / "w.las").write_text(las)
    geo = petekio.GeoData(unit="m")
    geo.load_well("W", head=(0.0, 0.0), kb=0.0, files=str(tmp_path))
    return geo.well("W").sidetrack("").log("PHIE")


def test_logview_batch_variants(tmp_path):
    """`at_md_many` matches per-call `at_md`; `values_md` matches `.values()`/`.md()`."""
    lv = _petro_well(tmp_path)
    depths = [95.0, 100.0, 105.0, 125.0, 130.0, 140.0]
    many = lv.at_md_many(depths)
    one = [lv.at_md(d) for d in depths]
    assert many == one
    vals, md = lv.values_md()
    assert vals == lv.values()
    assert md == lv.md()
