# Install

## Rust

```toml
[dependencies]
petekio = "0.2"
```

petekIO requires Rust 1.88+. The crate is pure Rust core; the Python bindings
are an opt-in feature and are **not** built for a plain Rust dependency.

## Python

```bash
pip install petekio
```

The wheel is built with [maturin](https://www.maturin.rs/) (abi3), so a single
wheel works across Python versions.

### Building the Python bindings from source

```bash
maturin develop          # debug build into the active venv
maturin develop --release  # for any performance measurement
```

After `maturin develop`, confirm it printed `Installed petekio-…`; an upstream
build error otherwise leaves the previous extension in place.

## Length unit

A project is created under one declared length unit, shared by every surface,
well, point, and polygon in it:

```python
import petekio
geo = petekio.GeoData(unit="m")   # "m"/"metres" or "ft"/"feet"
```

petekIO never reprojects coordinates and never silently mixes units — the
declared unit is the project's contract.
