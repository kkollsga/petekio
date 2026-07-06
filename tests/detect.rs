use petekio::{detect, FormatKind, GeoData, Unit};

mod common;
use common::tmpdir;

fn write(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
}

#[test]
fn detects_extensionless_known_formats() {
    let d = tmpdir("detect_extensionless");

    let irap = d.join("top");
    write(
        &irap,
        "-996 2 10 10\n0 10 0 10\n2 0 0 0\n0 0 0 0 0 0 0\n1 2 3 4\n",
    );
    assert_eq!(detect(&irap).unwrap(), FormatKind::IrapClassicGrid);

    let xyz = d.join("points");
    write(&xyz, "100.0 200.0 -10.0\n110.0 200.0 -11.0\n");
    assert_eq!(detect(&xyz).unwrap(), FormatKind::IrapClassicPoints);
}

#[test]
fn content_wins_over_misnamed_extension() {
    let d = tmpdir("detect_misnamed");

    let las = d.join("logs.csv");
    write(&las, "~Version\n VERS. 2.0 :\n~Well\n STRT.M 100 :\n");
    assert_eq!(detect(&las).unwrap(), FormatKind::Las);

    let cps3 = d.join("surface.xyz");
    write(
        &cps3,
        "FSASCI 0 1 0 5 1.0E+30\nFSLIMI 0 1 0 1\nFSNROW 2 2\n->\n1 2\n3 4\n",
    );
    assert_eq!(detect(&cps3).unwrap(), FormatKind::Cps3Grid);

    let tops = d.join("picks.dat");
    write(
        &tops,
        "# Petrel well tops\nVERSION 2\nBEGIN HEADER\nX\nY\nZ\nTWT\nTWT2\nage\nMD\nPVD\nType\nSurface\nWell\nEND HEADER\n\
         1 2 -1 -999 -999 -999 100.0 -1 Horizon \"Top\" \"W1\"\n",
    );
    assert_eq!(detect(&tops).unwrap(), FormatKind::PetrelTops);
}

#[test]
fn detects_tabular_and_structured_formats() {
    let d = tmpdir("detect_structured");

    let csv = d.join("points.noext");
    write(&csv, "x,y,z,poro\n1,2,-3,0.2\n");
    assert_eq!(detect(&csv).unwrap(), FormatKind::CsvPoints);

    let geojson = d.join("outline.txt");
    write(
        &geojson,
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","geometry":{"type":"Point","coordinates":[1,2,3]},"properties":{}}]}"#,
    );
    assert_eq!(detect(&geojson).unwrap(), FormatKind::GeoJson);

    let crs = d.join("crsmeta");
    write(
        &crs,
        r#"<?xml version="1.0"?><crsmeta><label>ED50</label></crsmeta>"#,
    );
    assert_eq!(detect(&crs).unwrap(), FormatKind::CrsMetaXml);
}

#[test]
fn manager_surface_dispatch_uses_content_before_extension() {
    let d = tmpdir("manager_surface_detect");

    let cps3 = d.join("trend.xyz");
    write(
        &cps3,
        "FSASCI 0 1 0 5 1.0E+30\n\
         FSLIMI 100 110 200 220 0 6\n\
         FSNROW 3 2\n\
         FSXINC 10 10\n\
         ->\n1 2\n3 4\n5 6\n",
    );
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_surface("trend", &cps3).unwrap();
    assert_eq!(geo.surface("trend").unwrap().values()[[1, 2]], 6.0);

    let irap = d.join("top_no_ext");
    write(
        &irap,
        "-996 2 10 10\n0 10 0 10\n2 0 0 0\n0 0 0 0 0 0 0\n1 2 3 4\n",
    );
    geo.load_surface("top", &irap).unwrap();
    assert_eq!(geo.surface("top").unwrap().values()[[1, 1]], 4.0);
}

#[test]
fn manager_point_dispatch_uses_content_before_extension() {
    let d = tmpdir("manager_point_detect");

    let csv = d.join("points_no_ext");
    write(&csv, "x,y,z,poro\n1,2,-3,0.2\n4,5,-6,0.3\n");
    let mut geo = GeoData::new(Unit::Metres);
    geo.load_points("csv", &csv).unwrap();
    assert_eq!(geo.points("csv").unwrap().len(), 2);

    let ev = d.join("ev.xyz");
    write(
        &ev,
        "# Type: scattered data\n# Grid_size: 2 x 1\n# Null_value: 1.0e30\n# End:\n\
         100.0 200.0 -10.0\n110.0 200.0 1.0e30\n",
    );
    geo.load_points("ev", &ev).unwrap();
    assert_eq!(geo.points("ev").unwrap().len(), 1);
}

#[test]
fn well_loader_uses_content_and_crsmeta_sidecar() {
    let d = tmpdir("well_detect_crsmeta");
    write(
        &d.join("misnamed_log.txt"),
        "~Version\n VERS. 2.0 :\n WRAP. NO :\n~Well\n \
         STRT.M 100.0 :\n STOP.M 110.0 :\n STEP.M 10.0 :\n NULL. -999.25 :\n\
         ~Curve\n DEPT.M :\n GR.GAPI :\n~ASCII\n100.0 45.0\n110.0 46.0\n",
    );
    write(
        &d.join("crsmeta.xml"),
        r#"<?xml version="1.0"?><crsmeta><label>ED50 / UTM zone 31N</label></crsmeta>"#,
    );

    let mut geo = GeoData::new(Unit::Metres);
    geo.load_well("W1", (1000.0, 2000.0), 25.0, &d).unwrap();
    let well = geo.well("W1").unwrap();
    assert_eq!(well.crs(), Some("ED50 / UTM zone 31N"));
    assert_eq!(well.mnemonics(), vec!["GR"]);
}
