//! Well-trajectory intersections with all three surface geometry levels.

use crate::algorithms::intersections::intersect_curve_surface;
use crate::core::{StructuredMeshSurface, Surface, Trajectory, TriSurface};
use crate::foundation::{GeoError, Point3, Result};

/// One immutable trajectory/surface crossing. Identity is attached by the
/// highest domain level that knows it: standalone trajectories leave well/bore
/// empty; sidetracks add bore; wells/project views add well and surface names.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceIntersection {
    pub md: f64,
    pub xyz: Point3,
    pub well: Option<String>,
    pub bore: Option<String>,
    pub surface: Option<String>,
}

impl SurfaceIntersection {
    pub(crate) fn anonymous(md: f64, xyz: Point3) -> Self {
        Self {
            md,
            xyz,
            well: None,
            bore: None,
            surface: None,
        }
    }

    pub(crate) fn identify(
        mut self,
        well: Option<&str>,
        bore: Option<&str>,
        surface: Option<&str>,
    ) -> Self {
        self.well = well.map(str::to_string);
        self.bore = bore.map(str::to_string);
        self.surface = surface.map(str::to_string);
        self
    }
}

/// Canonical triangle data consumed by the trajectory kernel.
#[doc(hidden)]
pub struct IntersectionMesh {
    pub vertices: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
}

/// A surface geometry that can be intersected by a well trajectory.
pub trait IntersectableSurface {
    #[doc(hidden)]
    fn intersection_mesh(&self) -> Result<IntersectionMesh>;
}

impl IntersectableSurface for Surface {
    fn intersection_mesh(&self) -> Result<IntersectionMesh> {
        self.to_tri_surface()?.intersection_mesh()
    }
}

impl IntersectableSurface for StructuredMeshSurface {
    fn intersection_mesh(&self) -> Result<IntersectionMesh> {
        self.to_tri_surface()?.intersection_mesh()
    }
}

impl IntersectableSurface for TriSurface {
    fn intersection_mesh(&self) -> Result<IntersectionMesh> {
        Ok(IntersectionMesh {
            vertices: self
                .points()
                .into_iter()
                .map(|p| Point3::new(p[0], p[1], p[2]))
                .collect(),
            triangles: self.triangles().to_vec(),
        })
    }
}

impl Trajectory {
    /// Every crossing with `surface`, ordered by measured depth.
    pub fn intersections<S: IntersectableSurface + ?Sized>(
        &self,
        surface: &S,
        tolerance: f64,
    ) -> Result<Vec<SurfaceIntersection>> {
        let mesh = surface.intersection_mesh()?;
        let breaks = self.md_breaks();
        intersect_curve_surface(
            &breaks,
            |md| self.xyz(md),
            &mesh.vertices,
            &mesh.triangles,
            tolerance,
        )
        .map(|hits| {
            hits.into_iter()
                .map(|h| SurfaceIntersection::anonymous(h.md, h.xyz))
                .collect()
        })
    }

    /// The sole crossing, `None` when there is no hit. Multiple crossings are
    /// ambiguous and fail with guidance to call [`intersections`](Self::intersections).
    pub fn intersection<S: IntersectableSurface + ?Sized>(
        &self,
        surface: &S,
        tolerance: f64,
    ) -> Result<Option<SurfaceIntersection>> {
        single(self.intersections(surface, tolerance)?)
    }
}

pub(crate) fn single(mut hits: Vec<SurfaceIntersection>) -> Result<Option<SurfaceIntersection>> {
    match hits.len() {
        0 => Ok(None),
        1 => Ok(hits.pop()),
        n => Err(GeoError::Unsupported(format!(
            "trajectory crosses the surface {n} times; call intersections(...) and select a crossing explicitly"
        ))),
    }
}
