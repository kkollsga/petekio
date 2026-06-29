//! `GeoData::model_inputs` — assemble the model-ready inputs contract from a
//! loaded project. The single entry point consumers call; everything upstream
//! (ingest → normalize → validate → interpret → characterise) is internal.

use super::GeoData;
use crate::analysis::ModelInputs;
use crate::foundation::Result;

impl GeoData {
    /// Assemble the [`ModelInputs`] this project can offer: normalized,
    /// validated, interpreted, unit-canonical, uncertainty-carrying, and
    /// provenance-flagged. The consumer derives nothing from raw data.
    ///
    /// GATE-0 contract: the signature is locked; the assembly pipeline is
    /// implemented per `dev-docs` (normalize → validate → interpret →
    /// characterise).
    pub fn model_inputs(&self) -> Result<ModelInputs> {
        todo!("GATE-0: assemble ModelInputs (normalize -> validate -> interpret -> characterise)")
    }
}
