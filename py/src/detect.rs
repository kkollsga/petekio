//! Python mirror for the public content detector.

use crate::to_pyerr;
use pyo3::prelude::*;

#[pyclass(name = "FormatKind", eq, eq_int, skip_from_py_object)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    Cps3Grid,
    Cps3Lines,
    IrapClassicGrid,
    IrapClassicPoints,
    EarthVisionGrid,
    Las,
    WellPath,
    PetrelTops,
    CrsMetaXml,
    GeoJson,
    CsvPoints,
    Unknown,
}

impl From<petekio::FormatKind> for FormatKind {
    fn from(value: petekio::FormatKind) -> Self {
        match value {
            petekio::FormatKind::Cps3Grid => FormatKind::Cps3Grid,
            petekio::FormatKind::Cps3Lines => FormatKind::Cps3Lines,
            petekio::FormatKind::IrapClassicGrid => FormatKind::IrapClassicGrid,
            petekio::FormatKind::IrapClassicPoints => FormatKind::IrapClassicPoints,
            petekio::FormatKind::EarthVisionGrid => FormatKind::EarthVisionGrid,
            petekio::FormatKind::Las => FormatKind::Las,
            petekio::FormatKind::WellPath => FormatKind::WellPath,
            petekio::FormatKind::PetrelTops => FormatKind::PetrelTops,
            petekio::FormatKind::CrsMetaXml => FormatKind::CrsMetaXml,
            petekio::FormatKind::GeoJson => FormatKind::GeoJson,
            petekio::FormatKind::CsvPoints => FormatKind::CsvPoints,
            petekio::FormatKind::Unknown => FormatKind::Unknown,
        }
    }
}

#[pymethods]
impl FormatKind {
    fn __repr__(&self) -> &'static str {
        match self {
            FormatKind::Cps3Grid => "FormatKind.Cps3Grid",
            FormatKind::Cps3Lines => "FormatKind.Cps3Lines",
            FormatKind::IrapClassicGrid => "FormatKind.IrapClassicGrid",
            FormatKind::IrapClassicPoints => "FormatKind.IrapClassicPoints",
            FormatKind::EarthVisionGrid => "FormatKind.EarthVisionGrid",
            FormatKind::Las => "FormatKind.Las",
            FormatKind::WellPath => "FormatKind.WellPath",
            FormatKind::PetrelTops => "FormatKind.PetrelTops",
            FormatKind::CrsMetaXml => "FormatKind.CrsMetaXml",
            FormatKind::GeoJson => "FormatKind.GeoJson",
            FormatKind::CsvPoints => "FormatKind.CsvPoints",
            FormatKind::Unknown => "FormatKind.Unknown",
        }
    }

    fn __str__(&self) -> &'static str {
        self.__repr__()
    }
}

#[pyfunction]
pub fn detect(path: &str) -> PyResult<FormatKind> {
    petekio::detect(path)
        .map(FormatKind::from)
        .map_err(to_pyerr)
}
