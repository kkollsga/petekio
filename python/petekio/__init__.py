"""petekIO — the subsurface data ingestion + structure layer.

A thin Python surface over the Rust `petekio` library: surfaces (and, as the
Rust phases land, wells/logs/points/polygons) with loading, interpolation, and
statistics. See https://github.com/kkollsga/petekio.
"""

from ._petekio import Stats, Surface, __version__

__all__ = ["Surface", "Stats", "__version__"]
