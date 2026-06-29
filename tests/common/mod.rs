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

/// A synthetic Petrel well-tops file: `Top A` picked on bores A (MD 1210) and ST2
/// (MD 1510), one `-999`-MD row (skipped), and a foreign well (skipped).
pub fn synth_tops() -> PathBuf {
    let d = tmpdir("tops");
    let p = d.join("wells_tops.tops");
    write(
        &p,
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n\
         1.0 2.0 -1180.0 -999 -999 -999 1210.0 -1180.0 Horizon \"Top A\" \"99/9-X A\"\n\
         1.0 2.0 -1480.0 -999 -999 -999 -999 -999 Horizon \"No Pick\" \"99/9-X ST2\"\n\
         1.0 2.0 -1490.0 -999 -999 -999 1510.0 -1490.0 Horizon \"Top A\" \"99/9-X ST2\"\n\
         1.0 2.0 -2000.0 -999 -999 -999 2020.0 -2000.0 Horizon \"Base\" \"99/9-Z\"\n",
    );
    p
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
