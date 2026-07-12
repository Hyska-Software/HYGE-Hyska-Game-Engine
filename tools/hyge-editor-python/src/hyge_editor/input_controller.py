"""Coalesced, revisioned viewport input owned by the Qt frontend."""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QObject, Property, QTimer, Signal, Slot


class ViewportInputController(QObject):
    """Collects transient input and sends one ordered batch at a time."""

    stateChanged = Signal()
    inputError = Signal(str)

    def __init__(self, session: Any, viewport: Any, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._session = session
        self._viewport = viewport
        self._revision = 0
        self._events: list[dict[str, Any]] = []
        self._keys: set[str] = set()
        self._buttons: set[str] = set()
        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.setInterval(8)
        self._timer.timeout.connect(self.flush)
        session.viewportTransportReady.connect(self._reset_generation)
        session.viewportTransportReset.connect(self._reset_generation)
        session.disconnected.connect(self.clear_transient)
        session.engineError.connect(self._on_error)

    @Property(int, notify=stateChanged)
    def revision(self) -> int:
        """Return the next local input revision."""
        return self._revision

    @Property(int, notify=stateChanged)
    def generation(self) -> int:
        """Return the current viewport transport generation."""
        return self._viewport.generation

    @Slot(str, bool)
    def key(self, code: str, pressed: bool) -> None:
        """Queue a keyboard edge."""
        code = str(code)
        if pressed:
            self._keys.add(code)
        else:
            self._keys.discard(code)
        self._events.append({"kind": "key", "code": code, "pressed": bool(pressed)})
        self._schedule()

    @Slot(str, bool)
    def button(self, button: str, pressed: bool) -> None:
        """Queue a mouse button edge."""
        button = str(button)
        if pressed:
            self._buttons.add(button)
        else:
            self._buttons.discard(button)
        self._events.append({"kind": "key", "code": f"mouse_{button}", "pressed": bool(pressed)})
        self._schedule()

    @Slot(float, float)
    def mouse(self, dx: float, dy: float) -> None:
        """Queue a mouse delta."""
        self._events.append({"kind": "mouse", "dx": float(dx), "dy": float(dy)})
        self._schedule()

    @Slot(float, float)
    def wheel(self, dx: float, dy: float) -> None:
        """Queue a wheel delta."""
        self._events.append({"kind": "wheel", "dx": float(dx), "dy": float(dy)})
        self._schedule()

    @Slot(str, float, float)
    def camera(self, command: str, x: float, y: float) -> None:
        """Queue an explicit camera operation."""
        self._events.append({"kind": "camera", "command": command, "x": float(x), "y": float(y)})
        self._schedule()

    @Slot()
    def clear_transient(self) -> None:
        """Release all active inputs and discard pending motion."""
        self._timer.stop()
        releases = [{"kind": "key", "code": code, "pressed": False} for code in sorted(self._keys)]
        releases.extend({"kind": "key", "code": f"mouse_{button}", "pressed": False} for button in sorted(self._buttons))
        self._keys.clear()
        self._buttons.clear()
        self._events = releases
        if releases:
            self.flush()
        else:
            self._events.clear()
        self.stateChanged.emit()

    @Slot()
    def flush(self) -> None:
        """Send the pending batch using the current mapping generation."""
        if not self._events or self.generation <= 0:
            self._events.clear()
            return
        expected = self._revision
        self._revision += 1
        events = self._events
        self._events = []
        self._session.request("viewport_input", {
            "generation": self.generation,
            "expected_input_revision": expected,
            "input_revision": self._revision,
            "events": events,
        })
        self.stateChanged.emit()

    def _schedule(self) -> None:
        if not self._timer.isActive():
            self._timer.start()

    def _reset_generation(self, _envelope: Any) -> None:
        self._timer.stop()
        self._events.clear()
        self._revision = 0
        self._keys.clear()
        self._buttons.clear()
        self.stateChanged.emit()

    def _on_error(self, envelope: Any) -> None:
        error = getattr(envelope, "error", None) or {}
        if error.get("code") in {"invalid_viewport_input", "stale_input_revision", "input_rate_limited"}:
            self._revision = 0
            self._events.clear()
            self.inputError.emit(error.get("message", error.get("code", "input failed")))
            self.stateChanged.emit()
