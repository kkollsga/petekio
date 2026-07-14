//! Analytic accuracy and persistence tests for trajectory/surface crossings.

use ndarray::Array2;
use petekio::{
    GeoData, GridGeometry, MeshShell, Point3, PolygonSet, Station, StructuredMeshSurface, Surface,
    Trajectory, TrajectoryInput, TriSurface, Unit, Well,
};
use std::sync::Arc;

fn geom(rotation_deg: f64) -> GridGeometry {
    GridGeometry {
        xori: -10.0,
        yori: -10.0,
        xinc: 10.0,
        yinc: 10.0,
        ncol: 3,
        nrow: 3,
        rotation_deg,
        yflip: false,
    }
}

fn vertical(x: f64, y: f64, td: f64) -> Trajectory {
    Trajectory::from_input(
        TrajectoryInput::Stations(vec![
            Station::new(0.0, 0.0, 0.0),
            Station::new(td, 0.0, 0.0),
        ]),
        (x, y),
        0.0,
    )
    .unwrap()
}

fn constant_surface(z: f64, rotation: f64) -> Surface {
    Surface::constant(geom(rotation), z)
}

fn edge() -> PolygonSet {
    PolygonSet::from_rings(vec![vec![
        [-10.0, -10.0, 0.0],
        [10.0, -10.0, 0.0],
        [10.0, 10.0, 0.0],
        [-10.0, 10.0, 0.0],
        [-10.0, -10.0, 0.0],
    ]])
}

fn structured(z: f64) -> StructuredMeshSurface {
    let x = Array2::from_shape_fn((3, 3), |(i, j)| -10.0 + i as f64 * 10.0 + j as f64);
    let y = Array2::from_shape_fn((3, 3), |(i, j)| -10.0 + j as f64 * 10.0 + i as f64 * 0.5);
    StructuredMeshSurface::new(x, y, Array2::from_elem((3, 3), z), None, edge()).unwrap()
}

fn flat_tri(z: f64) -> TriSurface {
    let shell = MeshShell::new(
        vec![[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]],
        vec![[0, 1, 2], [0, 2, 3]],
        vec![],
        edge(),
        vec![None; 4],
    )
    .unwrap();
    TriSurface::from_shell(Arc::new(shell), vec![z; 4]).unwrap()
}

#[test]
fn regular_rotated_structured_and_tin_are_analytic() {
    let trajectory = vertical(0.0, 0.0, 20.0);
    for hit in [
        trajectory.intersection(&constant_surface(-5.0, 0.0), 1e-6),
        trajectory.intersection(&constant_surface(-5.0, 31.0), 1e-6),
        trajectory.intersection(&structured(-5.0), 1e-6),
        trajectory.intersection(&flat_tri(-5.0), 1e-6),
    ] {
        let hit = hit.unwrap().unwrap();
        assert!((hit.md - 5.0).abs() < 1e-5, "{hit:?}");
        assert!((hit.xyz.z + 5.0).abs() < 1e-5, "{hit:?}");
    }
}

#[test]
fn deviated_minimum_curvature_arc_is_refined_in_md() {
    let trajectory = Trajectory::from_input(
        TrajectoryInput::Stations(vec![
            Station::new(0.0, 0.0, 90.0),
            Station::new(20.0, 60.0, 90.0),
            Station::new(40.0, 60.0, 90.0),
        ]),
        (0.0, 0.0),
        0.0,
    )
    .unwrap();
    let broad = Surface::constant(
        GridGeometry {
            xori: -50.0,
            yori: -50.0,
            xinc: 50.0,
            yinc: 50.0,
            ncol: 5,
            nrow: 5,
            rotation_deg: 0.0,
            yflip: false,
        },
        -10.0,
    );
    let hit = trajectory.intersection(&broad, 1e-5).unwrap().unwrap();
    // Proves root refinement on the actual arc: recomputing the trajectory at
    // the returned MD lies on the analytic horizontal plane.
    assert!((hit.xyz.z + 10.0).abs() < 1e-5, "hit={hit:?}");
    assert_eq!(trajectory.xyz(hit.md).unwrap(), hit.xyz);
}

#[test]
fn multiple_tangent_coplanar_null_outside_and_shared_edge_contracts() {
    let trajectory = vertical(0.0, 0.0, 20.0);
    let shell = MeshShell::new(
        vec![
            [-10.0, -10.0],
            [10.0, -10.0],
            [10.0, 10.0],
            [-10.0, 10.0],
            [-10.0, -10.0],
            [10.0, -10.0],
            [10.0, 10.0],
            [-10.0, 10.0],
        ],
        vec![[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        vec![],
        edge(),
        vec![None; 8],
    )
    .unwrap();
    let folded = TriSurface::from_shell(
        Arc::new(shell),
        vec![-5.0, -5.0, -5.0, -5.0, -12.0, -12.0, -12.0, -12.0],
    )
    .unwrap();
    let hits = trajectory.intersections(&folded, 1e-6).unwrap();
    assert_eq!(
        hits.iter().map(|h| h.md.round() as i32).collect::<Vec<_>>(),
        [5, 12]
    );
    assert!(trajectory
        .intersection(&folded, 1e-6)
        .unwrap_err()
        .to_string()
        .contains("intersections"));

    // A V-shaped explicit path touches the plane at its middle node and returns
    // one de-duplicated tangent/shared-edge pick.
    let tangent = Trajectory::from_input(
        TrajectoryInput::Xyz(vec![
            Point3::new(0.0, 0.0, -1.0),
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(0.0, 0.0, -1.0),
        ]),
        (0.0, 0.0),
        0.0,
    )
    .unwrap();
    assert_eq!(
        tangent.intersections(&flat_tri(0.0), 1e-6).unwrap().len(),
        1
    );

    let coplanar = Trajectory::from_input(
        TrajectoryInput::Xyz(vec![
            Point3::new(-5.0, 0.0, 0.0),
            Point3::new(5.0, 0.0, 0.0),
        ]),
        (0.0, 0.0),
        0.0,
    )
    .unwrap();
    assert!(coplanar
        .intersections(&flat_tri(0.0), 1e-6)
        .unwrap_err()
        .to_string()
        .contains("coplanar"));

    let mut values = Array2::from_elem((3, 3), -5.0);
    values[[1, 1]] = f64::NAN;
    let holed = Surface::new(geom(0.0), values).unwrap();
    assert!(trajectory.intersections(&holed, 1e-6).unwrap().is_empty());
    assert!(vertical(100.0, 100.0, 20.0)
        .intersections(&constant_surface(-5.0, 0.0), 1e-6)
        .unwrap()
        .is_empty());
    assert_eq!(
        trajectory
            .intersections(&flat_tri(-5.0), 1e-6)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn multibore_top_mutation_and_pproj_roundtrip_keep_exact_top_shape() {
    let mut well = Well::new("W1", (0.0, 0.0), 0.0);
    well.sidetrack_mut("A")
        .add_trajectory(TrajectoryInput::Stations(vec![
            Station::new(0.0, 0.0, 0.0),
            Station::new(20.0, 0.0, 0.0),
        ]))
        .unwrap();
    well.sidetrack_mut("B")
        .add_trajectory(TrajectoryInput::Stations(vec![
            Station::new(0.0, 0.0, 0.0),
            Station::new(20.0, 0.0, 0.0),
        ]))
        .unwrap();
    let surface = constant_surface(-5.0, 0.0);
    assert!(well
        .intersection(&surface, 1e-6)
        .unwrap_err()
        .to_string()
        .contains("multiple bores"));
    let hit = well
        .sidetrack("A")
        .unwrap()
        .intersection(&surface, 1e-6)
        .unwrap()
        .unwrap();
    well.sidetrack_mut("A")
        .add_top_from_intersection("Top A", &hit)
        .unwrap();
    assert!(well.sidetrack_mut("A").add_top("top a", 6.0).is_err());
    well.sidetrack_mut("A").replace_top("Top A", 6.0).unwrap();
    assert_eq!(well.sidetrack("A").unwrap().tops().next().unwrap().md, 6.0);
    well.sidetrack_mut("A").remove_top("top a").unwrap();
    assert_eq!(well.sidetrack("A").unwrap().tops().count(), 0);

    // Existing loader-shaped Top{name,md} remains the only serialized pick.
    well.sidetrack_mut("A").add_top("Top A", 5.0).unwrap();
    let geo = GeoData::new(Unit::Metres);
    // Public load is file-based, so persist the standalone well to prove its
    // backward-compatible DTO without introducing a manager insertion API.
    let path =
        std::env::temp_dir().join(format!("petekio-intersection-{}.pproj", std::process::id()));
    well.save(&path).unwrap();
    let restored = Well::load(&path).unwrap();
    std::fs::remove_file(path).ok();
    let top = restored.sidetrack("A").unwrap().tops().next().unwrap();
    assert_eq!((&top.name, top.md), (&"Top A".to_string(), 5.0));
    // Keep GeoData referenced so this test also compiles the manager contract.
    assert!(geo.wells().is_empty());
}
