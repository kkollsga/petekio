//! Synthetic fixture builders for integration tests. Everything here is
//! hand-authored to format spec and written to a **fresh temp dir at runtime** —
//! the repo's tests never read real data (nothing under `/…/Data`), so they are
//! fully self-contained and CI-clean.
//!
//! `tests/common` is compiled into each integration-test binary separately; a
//! given binary uses only a subset of these builders, so allow dead code.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh, unique temp directory for one test's synthetic files.
pub fn tmpdir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("petekio_{tag}_{}_{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// Header shared by the synthetic `.wellpath` files (wellhead 1000/2000, KB 27.3).
fn wp(rows: &str) -> String {
    format!(
        "# WELL TRACE FROM PETREL\n\
         # WELL HEAD X-COORDINATE: 1000.0 (m)\n\
         # WELL HEAD Y-COORDINATE: 2000.0 (m)\n\
         # WELL DATUM (KB, Kelly bushing, from MSL): 27.3 (m)\n\
         # CRS: ED50 / UTM zone 31N\n\
         =====\n\
         MD X Y Z TVD DX DY AZIM_TN INCL DLS AZIM_GN\n{rows}"
    )
}

fn las(strt: f64, stop: f64, mnemonic: &str, rows: &str) -> String {
    format!(
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n~Well\n \
         STRT.M {strt} :\n STOP.M {stop} :\n STEP.M 10.0 :\n NULL. -999.25 :\n\
         ~Curve\n DEPT.M :\n {mnemonic} :\n~ASCII\n{rows}"
    )
}

/// A synthetic multi-bore well folder (flat): bores A (vertical) + ST2 (build),
/// per-bore comp-logs (`PHIE_2025` / `SW_2025`) and a core LAS (`CPOR`).
pub fn synth_well() -> PathBuf {
    let d = tmpdir("well");
    write(
        &d.join("99_9-X_A.wellpath"),
        &wp("0 1000 2000 0 0 0 0 145 0 0 145\n\
             1200 1000 2000 -1200 1200 0 0 145 0 0 145\n\
             2000 1000 2000 -2000 2000 0 0 145 0 0 145\n"),
    );
    write(
        &d.join("99_9-X_ST2.wellpath"),
        &wp("0 1000 2000 0 0 0 0 145 0 0 145\n\
             1500 1000 2000 -1500 1500 0 0 145 0 0 145\n\
             2000 1050 1970 -1990 1995 50 -30 145 10 1 145\n"),
    );
    write(
        &d.join("99_9-X_A_CompLogs.las"),
        &las(
            1200.0,
            1220.0,
            "PHIE_2025.m3/m3",
            "1200.0 0.20\n1210.0 0.22\n1220.0 0.18\n",
        ),
    );
    write(
        &d.join("99_9-X_ST2_CompLogs.las"),
        &las(
            1500.0,
            1520.0,
            "SW_2025.v/v",
            "1500.0 0.30\n1510.0 0.35\n1520.0 0.40\n",
        ),
    );
    write(
        &d.join("99_9-X A full_core.las"),
        &las(
            1205.0,
            1215.0,
            "CPOR.pu",
            "1205.0 19.5\n1210.0 21.0\n1215.0 18.0\n",
        ),
    );
    d
}

/// A synthetic Petrel well-tops file: `Top A` (Horizon) picked on bores A (MD
/// 1210) and ST2 (MD 1510); an `Other`-type `OWC` contact on A (must be excluded
/// — not stratigraphy); one `-999`-MD row and a foreign well (both skipped).
pub fn synth_tops() -> PathBuf {
    let d = tmpdir("tops");
    let p = d.join("wells_tops.tops");
    write(
        &p,
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n\
         1.0 2.0 -1180.0 -999 -999 -999 1210.0 -1180.0 Horizon \"Top A\" \"99/9-X A\"\n\
         1.0 2.0 -1300.0 -999 -999 -999 1300.0 -1300.0 Other \"OWC\" \"99/9-X A\"\n\
         1.0 2.0 -1480.0 -999 -999 -999 -999 -999 Horizon \"No Pick\" \"99/9-X ST2\"\n\
         1.0 2.0 -1490.0 -999 -999 -999 1510.0 -1490.0 Horizon \"Top A\" \"99/9-X ST2\"\n\
         1.0 2.0 -2000.0 -999 -999 -999 2020.0 -2000.0 Horizon \"Base\" \"99/9-Z\"\n",
    );
    p
}

/// A synthetic **single deviated** well (one sidetrack): one `.wellpath` that is
/// already 200 m east of the wellhead by MD 1000 and keeps building, one comp-log
/// `GR` sampled across the deviated section, and a `tops.csv`. Mirrors a real NCS
/// deviated sidetrack (e.g. `99/9-1 A`): one path, one log, tops by well name —
/// everything on the well's single (main) bore.
pub fn synth_deviated_well() -> PathBuf {
    let d = tmpdir("deviated");
    // Rows: MD X Y Z TVD DX DY AZIM_TN INCL DLS AZIM_GN. A constant-inclination
    // hold due east (INCL = asin(0.6) ≈ 36.8699°, AZIM 90°) → unit tangent
    // ≈ [north 0, east 0.6, down 0.8]. X/TVD are integrated from that tangent so
    // the stored positions and the survey tangents are mutually consistent (as in
    // a real Petrel export): X steps 1000→1600→1612 east, TVD 0→800→816. A real
    // eastward deviation, not a vertical drop.
    write(
        &d.join("99_9-1_A.wellpath"),
        &wp("0 1000 2000 0 0 0 0 90 36.8699 0 90\n\
             1000 1600 2000 -800 800 600 0 90 36.8699 0 90\n\
             1020 1612 2000 -816 816 612 0 90 36.8699 0 90\n"),
    );
    write(
        &d.join("99_9-1_A_CompLogs.las"),
        &las(
            1000.0,
            1020.0,
            "GR.GAPI",
            "1000.0 40.0\n1010.0 50.0\n1020.0 60.0\n",
        ),
    );
    // Id-named so the well-id file filter retains it alongside the wellpath/LAS.
    write(
        &d.join("99_9-1_A_tops.csv"),
        "name,md\nBrent,1000.0\nDunlin,1010.0\n",
    );
    d
}

/// A synthetic Petrel well-tops file whose `Well` field uses a **separator
/// variant** of the well id (`99_9-1_A` for the id `99/9-1 A`) — exercising the
/// variant-tolerant name matching. One Horizon pick `Base` at MD 1015.
pub fn synth_variant_tops() -> PathBuf {
    let d = tmpdir("vtops");
    let p = d.join("variant.tops");
    write(
        &p,
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n\
         1.0 2.0 -995.0 -999 -999 -999 1015.0 -995.0 Horizon \"Base\" \"99_9-1_A\"\n",
    );
    p
}

/// A synthetic multi-well field demonstrating the cross-well lithostratigraphic
/// merge. Three wells share `Top`/`Mid`; the order of the rest is *unresolvable
/// within a single well* but determined across the field:
/// - **FIELD-1** (loaded here): `Mid` and `Sand` are coincident (zero thickness),
///   and the `Sand` pick is listed **last** in the file (as Petrel appends sand
///   members) — so by MD/insertion alone its `zones()` would read `Top, Mid, Sand`.
/// - **FIELD-2**: develops `Lower` strictly below `Mid` (`Mid ≺ Lower`).
/// - **FIELD-3**: develops `Sand` strictly above `Mid` (`Sand ≺ Mid`).
///
/// The merged column must be `Top, Sand, Mid, Lower`, and FIELD-1's `zones()`
/// must follow it (`Top, Sand, Mid`) — the field resolves what the borehole can't.
/// Returns `(field-1 well folder, tops path)`.
pub fn synth_field() -> (PathBuf, PathBuf) {
    let d = tmpdir("field");
    // FIELD-1 as a two-bore well (A carries the picks, B is a second bore) so
    // the shared-prefix `FIELD-1_` strips to clean labels A/B — mirroring a real
    // Petrel tree, where tops route to a labelled bore that owns a trajectory.
    let well = d.join("FIELD-1");
    for bore in ["A", "B"] {
        write(
            &well.join(format!("FIELD-1_{bore}.wellpath")),
            &wp("0 1000 2000 0 0 0 0 145 0 0 145\n\
                 200 1000 2000 -200 200 0 0 145 0 0 145\n"),
        );
    }
    let tops = d.join("field.tops");
    write(
        &tops,
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n\
         1 2 -1 -999 -999 -999 100.0 -1 Horizon \"Top\" \"FIELD-1 A\"\n\
         1 2 -1 -999 -999 -999 100.0 -1 Horizon \"Top\" \"FIELD-2\"\n\
         1 2 -1 -999 -999 -999 100.0 -1 Horizon \"Top\" \"FIELD-3\"\n\
         1 2 -1 -999 -999 -999 120.0 -1 Horizon \"Mid\" \"FIELD-1 A\"\n\
         1 2 -1 -999 -999 -999 120.0 -1 Horizon \"Mid\" \"FIELD-2\"\n\
         1 2 -1 -999 -999 -999 120.0 -1 Horizon \"Mid\" \"FIELD-3\"\n\
         1 2 -1 -999 -999 -999 130.0 -1 Horizon \"Lower\" \"FIELD-2\"\n\
         1 2 -1 -999 -999 -999 120.0 -1 Horizon \"Sand\" \"FIELD-1 A\"\n\
         1 2 -1 -999 -999 -999 110.0 -1 Horizon \"Sand\" \"FIELD-3\"\n",
    );
    (well, tops)
}

/// A synthetic Petrel-style split tree: `Paths/` + `Logs/` in separate subdirs,
/// plus a foreign well's log (`99_9-OTHER`) that the id-filter must exclude.
pub fn synth_split() -> PathBuf {
    let d = tmpdir("split");
    for bore in ["A", "B"] {
        write(
            &d.join("Paths").join(format!("99_9-Y_{bore}.wellpath")),
            &wp("0 1000 2000 0 0 0 0 145 0 0 145\n1000 1000 2000 -1000 1000 0 0 145 0 0 145\n"),
        );
        write(
            &d.join("Logs").join(format!("99_9-Y_{bore}_CompLogs.las")),
            &las(100.0, 110.0, "GR.GAPI", "100.0 40.0\n110.0 60.0\n"),
        );
    }
    write(
        &d.join("Logs").join("99_9-OTHER_A_CompLogs.las"),
        &las(100.0, 110.0, "GR.GAPI", "100.0 40.0\n110.0 60.0\n"),
    );
    d
}
