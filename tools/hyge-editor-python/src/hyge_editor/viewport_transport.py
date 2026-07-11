"""Reader for the Hyge R-088 Windows named shared-memory viewport ring."""

from __future__ import annotations

import mmap
import struct
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

    def read_frame(self, slot: int, stride: int) -> ViewportFrame | None:
        """Copy a slot only when its commit sequence is stable."""
        offset = 64 + slot * stride
        header = self._mapping[offset:offset + 64]
        frame_id, width, height, pixel_format, byte_len, _scene, _camera, sequence = struct.unpack_from("<QIIIIQQQ", header)
        committed = struct.unpack_from("<Q", header, 48)[0]
        if not frame_id or pixel_format != 1 or not sequence or sequence != committed:
            return None
        pixels = bytes(self._mapping[offset + 64:offset + 64 + byte_len])
        if struct.unpack_from("<Q", self._mapping, offset + 48)[0] != sequence:
            self.dropped_frames += 1
            return None
        if self._last_frame_id and frame_id > self._last_frame_id + 1:
            self.dropped_frames += frame_id - self._last_frame_id - 1
        self._last_frame_id = frame_id
        if len(pixels) != width * height * 4:
            return None
        return ViewportFrame(frame_id, width, height, pixels)
