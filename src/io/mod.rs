//! `io` — format readers/writers (IRAP, ZMAP, CSV, LAS, Excel, vector). Wraps
//! external crates behind petekIO's own types. Imports only from `foundation`.

pub mod csv_points;
pub mod irap;
pub mod las;
pub mod tops;
pub mod vector;
pub mod wellpath;
pub mod xyz;
