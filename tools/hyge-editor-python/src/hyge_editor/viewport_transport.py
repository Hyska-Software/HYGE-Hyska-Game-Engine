"""Reader for the Hyge R-088 Windows named shared-memory viewport ring."""

from __future__ import annotations

import mmap
import os
import struct
import time
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
        self._mapping: mmap.mmap | None = mmap.mmap(-1, size, tagname=name, access=mmap.ACCESS_WRITE)
        self.dropped_frames = 0
        self._last_frame_id = 0

    def close(self) -> None:
        if self._mapping is not None:
            self._mapping.close()
            self._mapping = None

    def __enter__(self) -> "ViewportTransport": return self
    def __exit__(self, *_: object) -> None: self.close()

    def read_frame(self, slot: int, stride: int) -> ViewportFrame | None:
        """Copy a slot only when its commit sequence is stable."""
        if self._mapping is None:
            return None
        mapping = self._mapping
        self.touch_heartbeat()
        offset = 64 + slot * stride
        header = mapping[offset:offset + 64]
        frame_id, width, height, pixel_format, byte_len, _scene, _camera, sequence = struct.unpack_from("<QIIIIQQQ", header)
        committed = struct.unpack_from("<Q", header, 48)[0]
        if not frame_id or pixel_format != 1 or not sequence or sequence != committed:
            return None
        pixels = bytes(mapping[offset + 64:offset + 64 + byte_len])
        if struct.unpack_from("<Q", mapping, offset + 48)[0] != sequence:
            self.dropped_frames += 1
            return None
        if self._last_frame_id and frame_id > self._last_frame_id + 1:
            self.dropped_frames += frame_id - self._last_frame_id - 1
        self._last_frame_id = frame_id
        if len(pixels) != width * height * 4:
            return None
        return ViewportFrame(frame_id, width, height, pixels)

    def touch_heartbeat(self) -> None:
        """Publish the consumer process identity and heartbeat."""
        if self._mapping is None:
            return
        self._mapping[40:48] = struct.pack("<Q", os.getpid())
        self._mapping[48:56] = struct.pack("<Q", int(time.time() * 1000))

    def producer_is_stale(self, timeout_ms: int = 2000) -> bool:
        """Return whether the producer heartbeat has stopped advancing."""
        if self._mapping is None:
            return True
        heartbeat = struct.unpack_from("<Q", self._mapping, 32)[0]
        return heartbeat == 0 or int(time.time() * 1000) - heartbeat > timeout_ms
