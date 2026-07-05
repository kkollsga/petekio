//! End-to-end: a LAS 3.0 (comma-delimited) core file loaded through
//! `GeoData::load_well` now yields curves instead of a silent zero-curve well
//! (petekIO weakness W1). Fixture is hand-authored to LAS 3.0 spec.

use petekio::{GeoData, Unit};

mod common;
use common::tmpdir;

fn write(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
}

#[test]
fn load_well_reads_las3_core_curves() {
    let d = tmpdir("las3");
    // Single-bore well: no .wellpath → main bore with a vertical trajectory
    // synthesized over the log MD span; the LAS 3.0 core curves attach to it.
    write(
        &d.join("99_9-1 full_core.las"),
        "~Version\r\n\
         VERS. 3.0 : CWLS LOG ASCII STANDARD - VERSION 3.0\r\n\
         WRAP. NO :\r\n\
         DLM.  COMMA :\r\n\
         ~Well\r\n\
         STRT.M 1205.0 :\r\n\
         STOP.M 1215.0 :\r\n\
         NULL.  -999.25 :\r\n\
         ~Log_Definition\r\n\
         DEPTH.M : Depth\r\n\
         CPOR.pu : core porosity\r\n\
         CKH.mD  : core permeability\r\n\
         ~Log_Data\r\n\
         1205.0, 19.5, 120.0\r\n\
         1210.0, 21.0, -999.25\r\n\
         1215.0, 18.0, 95.5\r\n",
    );

    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("99/9-1", (0.0, 0.0), 0.0, &d).unwrap();
    let well = geo.well("99/9-1").unwrap();

    let mnem: Vec<&str> = well.mnemonics();
    assert!(mnem.contains(&"CPOR"), "expected CPOR, got {mnem:?}");
    assert!(mnem.contains(&"CKH"), "expected CKH, got {mnem:?}");

    let cpor = well.log("CPOR").expect("CPOR log");
    assert_eq!(cpor.values(), &[19.5, 21.0, 18.0]);
    let ckh = well.log("CKH").expect("CKH log");
    assert!(ckh.values()[1].is_nan()); // -999.25 → NaN
}
