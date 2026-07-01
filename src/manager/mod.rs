//! `manager` — the `GeoData` substrate + views: load once, named/collection
//! access, broadcastable filtered views. The top layer; imports from all below.
//!
//! [`GeoData`] is the load-once project (surfaces/wells/points/polygons keyed by
//! name); [`WellsView`] is the lightweight, filterable borrow over its wells
//! behind the broadcast ergonomic. Realizes `API.md` §"GeoData".

mod geodata; // GeoData — the load-once project + named/collection access
mod loaders; // GeoData::load_* — extension-dispatched ingest + well routing
mod model_inputs; // GeoData::model_inputs — the model-ready inputs contract
mod project; // GeoData::save/open/inspect — whole-project .pproj persistence
mod wells_view; // WellsView — broadcastable, filterable borrow over wells

pub use geodata::GeoData;
pub(crate) use project::ModelSection;
pub use project::ProjectInfo;
pub use wells_view::WellsView;
