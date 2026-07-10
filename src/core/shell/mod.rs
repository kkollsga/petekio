//! `shell` — the three-level **geometry shell** system.
//!
//! A shell is a **flat empty shell**: purely topological/positional geometry,
//! never a function of z. Surfaces are *shell + properties* — the shell carries
//! where the nodes are and how they connect; every value (depth, thickness,
//! amplitude, …) is a property lane mapped onto it. Three levels of increasing
//! complexity:
//!
//! 1. **Rigid grid** — [`GridGeometry`](crate::foundation::GridGeometry):
//!    8 scalars, node XY computed. Lives in `foundation`; unchanged.
//! 2. **[`StructuredShell`]** — `(i, j)`-organized nodes with explicit
//!    per-node XY (fault-shifted / curvilinear meshes that keep a rectangular
//!    logical topology).
//! 3. **[`MeshShell`]** — integer node ids with explicit XY, triangle
//!    topology, a quad-dominant wireframe, a boundary edge, and per-node walk
//!    labels (the fully unstructured level; fault-cut surfaces).
//!
//! Conversions go **up for free** (lossless; node identity preserved) and
//! **down by inference/fit** (`infer_grid`, which errors when the shell is not
//! regular). Shells are immutable once built and shared via `Arc`, so N
//! property lanes / clones never repeat geometry in memory.

mod corner;
mod fit;
mod mesh;
mod structured;

pub use corner::CornerTable;
pub use mesh::{MeshShell, WalkLabel};
pub use structured::StructuredShell;

pub(crate) use fit::{fit_grid_from_coords, fit_grid_from_indexed};
