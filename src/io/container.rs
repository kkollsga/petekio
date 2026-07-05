//! `io::container` — the `.pproj` section container.
//!
//! The generic framing (file magic + JSON header + per-section `zstd(payload)`
//! blobs + byte-lossless `filter_to`/`merge_to`) was **lifted into petekTools**
//! (`petektools::container`) as a domain-agnostic toolkit — the on-disk format
//! is unchanged. petekIO re-exports it here so its GeoData element DTOs
//! (`serial` + `core::persist` + `manager::project`) keep layering on `container::`
//! exactly as before. `petektools::AlgoError` composes into [`GeoError`] via the
//! `#[from]` seam, so the `?` operator keeps working across the boundary.
//!
//! [`GeoError`]: crate::foundation::GeoError

pub use petektools::container::{filter_to, merge_to, open, write, Section};
