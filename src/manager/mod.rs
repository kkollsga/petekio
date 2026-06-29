//! `manager` — the `GeoData` substrate + views: load once, named/collection
//! access, broadcastable filtered views. The top layer; imports from all below.
//!
//! [`GeoData`] is the load-once project (surfaces/wells/points/polygons keyed by
//! name); [`WellsView`] is the lightweight, filterable borrow over its wells
//! behind the broadcast ergonomic. Realizes `API.md` §"GeoData".

mod geodata; // GeoData — the load-once project + named/collection access
mod model_inputs; // GeoData::model_inputs — the model-ready inputs contract
mod wells_view; // WellsView — broadcastable, filterable borrow over wells

pub use geodata::GeoData;
pub use wells_view::WellsView;
