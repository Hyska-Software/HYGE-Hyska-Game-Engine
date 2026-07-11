"""Reader for the Hyge R-088 Windows named shared-memory viewport ring."""

from __future__ import annotations

import mmap
from dataclasses import dataclass

RING_MAGIC = b"HYGEVPR1"

@dataclass(frozen=True)
class ViewportFrame:
    """A copied stable frame returned by the transport reader."""
    frame_id: int
    width: int
    height: int
    pixels: bytes

class ViewportTransport:
    """Owns a named Windows mapping and records detected dropped frames."""
    def __init__(self, name: str, size: int) -> None:
        self._mapping = mmap.mmap(-1, size, tagname=name, access=mmap.ACCESS_READ)
        self.dropped_frames = 0
        self._last_frame_id = 0

    def close(self) -> None:
        self._mapping.close()

    def __enter__(self) -> "ViewportTransport": return self
    def __exit__(self, *_: object) -> None: self.close()
