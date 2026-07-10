//! `algorithms` — petekIO's internal numeric/geostatistical kernels, **grouped
//! by discipline** (`wells`, … ; more land as they arrive).
//!
//! The discipline (see `SPEC.md` constitution): high-value numeric routines live
//! here as **pure, type-light functions** — primitives + `foundation` types
//! (`Point3`, `GeoError`) in and out, no domain-object (`Surface`/`Well`/…) or IO
//! coupling. Domain types in `core`/`analysis` call into these kernels rather
//! than inlining a formula, and a kernel's math has exactly one home (no
//! duplicated formula across call sites).
//!
//! Two payoffs: each kernel is trivial to **QC in isolation** (analytic tests on
//! raw numbers), and a kernel that proves high-value is a cheap **lift-and-shift
//! into the external `petekTools` library** (this module mirrors its
//! type-light boundary). Imports only from `foundation`.

pub mod surfaces;
pub mod wells;
