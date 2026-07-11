"""Qt bridge for the R-088 shared-memory viewport ring."""

from __future__ import annotations

from typing import Any, Protocol

from PySide6.QtCore import QObject, Property, QTimer, Signal
from PySide6.QtGui import QImage
from PySide6.QtQuick import QQuickImageProvider

from .viewport_transport import ViewportFrame, ViewportTransport


class FrameReader(Protocol):
    """Minimal reader interface used by the real transport and tests."""

    dropped_frames: int

    def read_frame(self, slot: int, stride: int) -> ViewportFrame | None: ...

    def close(self) -> None: ...


class ViewportImageProvider(QQuickImageProvider):
    """Returns the most recently copied RGBA frame to QML."""

    def __init__(self) -> None:
        super().__init__(QQuickImageProvider.ImageType.Image)
        self._image = QImage()

    def set_image(self, image: QImage) -> None:
        self._image = image

    def requestImage(self, _identifier: str, size: Any, requested_size: Any) -> QImage:
        if size is not None:
            size.width = self._image.width()
            size.height = self._image.height()
        if requested_size is not None and requested_size.width() > 0 and requested_size.height() > 0:
            return self._image.scaled(requested_size)
        return self._image


class ViewportController(QObject):
    """Polls stable shared-memory frames without blocking the Qt thread."""

    imageChanged = Signal()
    frameReceived = Signal(int)
    droppedFramesChanged = Signal(int)
    transportChanged = Signal(str)
    error = Signal(str)

    def __init__(self, session: Any = None, reader_factory: Any = ViewportTransport, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._session = session
        self._reader_factory = reader_factory
        self._reader: FrameReader | None = None
        self._mapping_name = ""
        self._generation = 0
        self._width = 640
        self._height = 360
        self._dropped_frames = 0
        self._last_transport_drops = 0
        self._provider = ViewportImageProvider()
        self._timer = QTimer(self)
        self._timer.setInterval(16)
        self._timer.timeout.connect(self.poll)
        if session is not None:
            session.viewportTransportReady.connect(self.open_transport)
            session.viewportTransportReset.connect(self.reset_transport)
            session.disconnected.connect(self.close)

    @Property(int, notify=droppedFramesChanged)
    def droppedFrames(self) -> int:
        """Return the cumulative number of dropped frames."""
        return self._dropped_frames

    @Property(str, notify=transportChanged)
    def state(self) -> str:
        """Return the current transport state."""
        return "connected" if self._reader is not None else "disconnected"

    @property
    def provider(self) -> ViewportImageProvider:
        """Return the image provider registered with the QML engine."""
        return self._provider

    @property
    def dropped_frames(self) -> int:
        """Return the cumulative count of dropped or unstable frames."""
        return self._dropped_frames

    def open_transport(self, envelope: Any) -> None:
        """Open the mapping described by ``viewport_transport_ready``."""
        self._open(envelope)

    def reset_transport(self, envelope: Any) -> None:
        """Replace the mapping after a resize or reconnect generation."""
        self.close()
        self._open(envelope)

    def _open(self, envelope: Any) -> None:
        payload = envelope.payload if hasattr(envelope, "payload") else envelope
        try:
            self._mapping_name = str(payload["mapping_name"])
            self._generation = int(payload["generation"])
            self._width = int(payload["width"])
            self._height = int(payload["height"])
            size = 64 + 3 * (64 + self._width * self._height * 4)
            self._reader = self._reader_factory(self._mapping_name, size)
            self._last_transport_drops = 0
            self._timer.start()
            self.transportChanged.emit("connected")
        except (KeyError, OSError, ValueError) as exc:
            self.error.emit(str(exc))
            self.transportChanged.emit("degraded")

    def poll(self) -> None:
        """Copy the newest stable slot and notify QML when an image changes."""
        if self._reader is None:
            return
        stride = 64 + self._width * self._height * 4
        newest: ViewportFrame | None = None
        for slot in range(3):
            frame = self._reader.read_frame(slot, stride)
            if frame is not None and (newest is None or frame.frame_id > newest.frame_id):
                newest = frame
        current_drops = getattr(self._reader, "dropped_frames", 0)
        delta = max(0, current_drops - self._last_transport_drops)
        self._last_transport_drops = current_drops
        if delta:
            self._dropped_frames += delta
            self.droppedFramesChanged.emit(self._dropped_frames)
        if newest is None:
            return
        image = QImage(newest.pixels, newest.width, newest.height, newest.width * 4, QImage.Format.Format_RGBA8888).copy()
        self._provider.set_image(image)
        self.imageChanged.emit()
        self.frameReceived.emit(newest.frame_id)

    def close(self) -> None:
        """Stop polling and release the current mapping."""
        self._timer.stop()
        if self._reader is not None:
            self._reader.close()
            self._reader = None
        self.transportChanged.emit("disconnected")
