//! `normalize` — canonicalize heterogeneous inputs into model-ready form:
//! LAS mnemonic aliasing (`PHIE`/`PHI`/`NPHI` → canonical), formation & well
//! name-maps, and unit harmonisation. The first half of the path from loaded
//! data to [`ModelInputs`](super::model_inputs::ModelInputs).
//!
//! GATE-0: the layer is declared here; the canonicalisation tables and passes
//! land per `dev-docs`. (Tracked from the consumer side as the petekio
//! "normalize" gap.)
