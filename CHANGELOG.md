# Changelog

All notable changes to petekIO are recorded here. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/); the project uses SemVer. The
`release` skill promotes `[Unreleased]` to a versioned block at release time.

## [Unreleased]

### Added
- Project scaffolding: single layered `petekio` crate
  (`foundation → io → core → analysis → manager → py`), MSRV 1.88, `py`/`f32`
  feature placeholders.
- `foundation` layer: `GeoError`/`Result`, `Unit` (+conversions), `Point3`,
  `BBox`, `GridGeometry` (rotation + `yflip`, with `node_xy`/`xy_to_ij`/`bbox`),
  and NaN-skipping `Stats` (`of`/`weighted`/`percentile`).
- `Surface` (`core`): construction (`new`/`constant`), attribute layers
  (`attr`/`set_attr`/`attr_names`/`as_attr_surface`), and **IRAP-classic** read/
  write (`load_irap_classic`/`save_irap_classic`).
- `Surface::sample` (strict NaN-aware bilinear) + `Surface::resample` onto a
  target geometry.
- `Surface` math (immutable, NaN-propagating): element-wise `ln`/`log10`/`exp`/
  `sqrt`/`abs`/`powf`/`clamp_min`/`clamp`; surface↔surface `plus`/`minus`/
  `times`/`divided_by` + `thickness`; operator overloads (scalar `+ - * /` →
  `Surface`, surface `+ - * /` → `Result<Surface>`).
