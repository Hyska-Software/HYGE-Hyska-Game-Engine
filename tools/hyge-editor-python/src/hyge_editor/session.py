"""Asynchronous Qt session for the Hyge editor control protocol."""

from __future__ import annotations

import threading
from dataclasses import dataclass
from typing import Any, Callable

from PySide6.QtCore import QObject, Signal

from .ipc import EditorClient, Envelope


@dataclass
class _Request:
    message_type: str
    payload: dict[str, Any]


class EditorSession(QObject):
    """Owns only the frontend connection and disposable snapshot caches."""

    stateChanged = Signal(str)
    connected = Signal(object)
    disconnected = Signal()
    protocolError = Signal(str)
    envelopeReceived = Signal(object)
    lifecycleStatus = Signal(object)
    worldSnapshot = Signal(object)
    selectionChanged = Signal(object)
    assetSnapshot = Signal(object)
    consoleSnapshot = Signal(object)
    profilerSnapshot = Signal(object)
    assetPreviewReady = Signal(object)
    assetPreviewCancelled = Signal(object)
    sceneReloaded = Signal(object)
    sceneReloadConflict = Signal(object)
    viewportTransportReady = Signal(object)
    viewportTransportReset = Signal(object)
    engineError = Signal(object)
    commandCompleted = Signal(object)

    def __init__(
        self,
        address: str,
        token: str,
        client_factory: Callable[[str, str], EditorClient] = EditorClient,
        parent: QObject | None = None,
    ) -> None:
        super().__init__(parent)
        self._address = address
        self._token = token
        self._client_factory = client_factory
        self._client: EditorClient | None = None
        self._thread: threading.Thread | None = None
        self._stop = threading.Event()
        self._requests: list[_Request] = []
        self._request_condition = threading.Condition()
        self._state = "disconnected"

    @property
    def state(self) -> str:
        """Return the current connection state."""
        return self._state

    def connect_async(self) -> None:
        """Start the connection worker without blocking the Qt event loop."""
        if self._thread is not None and self._thread.is_alive():
            return
        self._stop.clear()
        self._set_state("connecting")
        self._thread = threading.Thread(target=self._run, name="hyge-editor-ipc", daemon=True)
        self._thread.start()

    def request(self, message_type: str, payload: dict[str, Any] | None = None) -> None:
        """Queue a protocol request for the connection worker."""
        with self._request_condition:
            self._requests.append(_Request(message_type, payload or {}))
            self._request_condition.notify()

    def close(self) -> None:
        """Stop the worker and close the socket exactly once."""
        self._stop.set()
        with self._request_condition:
            self._request_condition.notify_all()
        client = self._client
        if client is not None:
            client.close()
        thread = self._thread
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout=2.0)
        self._thread = None
        self._client = None
        if self._state != "disconnected":
            self._set_state("disconnected")

    def open_project(self, path: str) -> None:
        """Queue an open-project request."""
        self.request("open_project", {"path": path})

    def open_scene(self, path: str) -> None:
        """Queue an open-scene request."""
        self.request("open_scene", {"path": path})

    def _run(self) -> None:
        try:
            self._client = self._client_factory(self._address, self._token)
            handshake = self._client.connect()
            if handshake.error is not None:
                self._emit_envelope(handshake)
                self._set_state("failed")
                return
            self.connected.emit(handshake)
            self._set_state("ready")
            while not self._stop.is_set():
                with self._request_condition:
                    if not self._requests:
                        self._request_condition.wait(timeout=0.1)
                    if not self._requests:
                        continue
                    request = self._requests.pop(0)
                response = self._client.request_observing(
                    request.message_type,
                    request.payload,
                    on_event=self._emit_envelope,
                )
                self._emit_envelope(response)
                if response.error is not None:
                    self._set_state("degraded")
        except (ConnectionError, OSError, TimeoutError, ValueError) as error:
            if not self._stop.is_set():
                self.protocolError.emit(str(error))
                self._set_state("failed")
        finally:
            if self._client is not None:
                self._client.close()
            self.disconnected.emit()

    def _emit_envelope(self, envelope: Envelope) -> None:
        self.envelopeReceived.emit(envelope)
        signal = {
            "lifecycle_status": self.lifecycleStatus,
            "world_snapshot": self.worldSnapshot,
            "selection_changed": self.selectionChanged,
            "asset_snapshot": self.assetSnapshot,
            "console_snapshot": self.consoleSnapshot,
            "profiler_snapshot": self.profilerSnapshot,
            "asset_preview_ready": self.assetPreviewReady,
            "asset_preview_cancelled": self.assetPreviewCancelled,
            "scene_reloaded": self.sceneReloaded,
            "scene_reload_conflict": self.sceneReloadConflict,
            "viewport_transport_ready": self.viewportTransportReady,
            "viewport_transport_reset": self.viewportTransportReset,
            "engine_error": self.engineError,
            "command_completed": self.commandCompleted,
        }.get(envelope.message_type)
        if signal is not None:
            signal.emit(envelope)

    def _set_state(self, state: str) -> None:
        self._state = state
        self.stateChanged.emit(state)
