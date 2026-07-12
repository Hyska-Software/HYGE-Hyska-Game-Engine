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
    reconnectScheduled = Signal(int, int)
    backendShutdown = Signal(object)
    transportClosed = Signal(object)
    reconnectRequired = Signal(object)

    _RETRY_DELAYS = (0.1, 0.25, 0.5, 1.0, 2.0, 5.0)
    _MUTATING_REQUESTS = {
        "open_project", "open_scene", "save_scene", "resolve_scene_reload",
        "select_entities", "edit_component", "edit_components", "add_component",
        "remove_component", "reparent_entity", "duplicate_entity", "destroy_entity",
        "instantiate_prefab", "undo", "redo", "set_editor_camera", "set_viewport_size",
        "viewport_input", "open_viewport_transport", "close_viewport_transport",
        "viewport_transport_reset", "activate_asset", "cancel_asset_preview",
    }

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
        self._closing = False
        self._session_id: str | None = None
        self._last_project: str | None = None
        self._last_scene: str | None = None
        self._had_connection = False
        self._needs_reopen = False

    @property
    def state(self) -> str:
        """Return the current connection state."""
        return self._state

    def connect_async(self) -> None:
        """Start the connection worker without blocking the Qt event loop."""
        if self._thread is not None and self._thread.is_alive():
            return
        self._closing = False
        self._stop.clear()
        self._set_state("connecting")
        self._thread = threading.Thread(target=self._run, name="hyge-editor-ipc", daemon=True)
        self._thread.start()

    def request(self, message_type: str, payload: dict[str, Any] | None = None) -> None:
        """Queue a protocol request for the connection worker."""
        if message_type == "open_project" and payload:
            self._last_project = payload.get("path")
        if message_type == "open_scene" and payload:
            self._last_scene = payload.get("path")
        if message_type in self._MUTATING_REQUESTS and self._state not in {"ready", "degraded"}:
            self.protocolError.emit("mutation blocked until the backend is ready")
            self._set_state("degraded")
            return
        with self._request_condition:
            self._requests.append(_Request(message_type, payload or {}))
            self._request_condition.notify()

    def close(self) -> None:
        """Stop the worker and close the socket exactly once."""
        self._closing = True
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

    def retry_now(self) -> None:
        """Cancel the current backoff and immediately start a new connection."""
        if self._state in {"ready", "connecting"}:
            return
        self.close()
        self.connect_async()

    def open_project(self, path: str) -> None:
        """Queue an open-project request."""
        self.request("open_project", {"path": path})

    def open_scene(self, path: str) -> None:
        """Queue an open-scene request."""
        self.request("open_scene", {"path": path})

    def _run(self) -> None:
        retry_index = 0
        while not self._stop.is_set():
            self._set_state("connecting" if not self._had_connection else "reconnecting")
            try:
                self._client = self._client_factory(self._address, self._token)
                resume = getattr(self._client, "resume_session", None)
                if resume is not None:
                    resume(self._session_id)
                handshake = self._client.connect()
                if handshake.error is not None:
                    self._emit_envelope(handshake)
                    code = handshake.error.get("code", "")
                    if code == "session_not_found":
                        self._session_id = None
                        self._needs_reopen = True
                    elif code in {"unauthorized", "incompatible_version"}:
                        self._set_state("failed")
                        return
                    raise ConnectionError(f"handshake failed: {code}")
                self._session_id = self._client.session_id
                resumed = bool(handshake.payload.get("resumed"))
                self._had_connection = True
                self.connected.emit(handshake)
                self._set_state("ready")
                retry_index = 0
                if self._needs_reopen and self._last_project:
                    self._enqueue_recovery("open_project", {"path": self._last_project})
                    if self._last_scene:
                        self._enqueue_recovery("open_scene", {"path": self._last_scene})
                    self._needs_reopen = False
                elif resumed:
                    for message_type in ("request_world_snapshot", "open_viewport_transport"):
                        self._enqueue_recovery(message_type, {})
                self._serve_requests()
                if self._stop.is_set():
                    break
                raise ConnectionError("editor service closed the connection")
            except (ConnectionError, OSError, TimeoutError, ValueError) as error:
                if self._stop.is_set() or self._closing:
                    break
                self._retain_safe_requests()
                self.protocolError.emit(str(error))
                self.disconnected.emit()
                self._set_state("reconnecting")
                delay = self._RETRY_DELAYS[min(retry_index, len(self._RETRY_DELAYS) - 1)]
                retry_index += 1
                self.reconnectScheduled.emit(retry_index, int(delay * 1000))
                self._stop.wait(delay)
            finally:
                if self._client is not None:
                    self._client.close()
                    self._client = None
        if self._state != "disconnected":
            self._set_state("disconnected")

    def _serve_requests(self) -> None:
        while not self._stop.is_set():
            with self._request_condition:
                if not self._requests:
                    self._request_condition.wait(timeout=0.1)
                if not self._requests:
                    continue
                request = self._requests.pop(0)
            if self._client is None:
                return
            response = self._client.request_observing(
                request.message_type,
                request.payload,
                on_event=self._emit_envelope,
            )
            self._emit_envelope(response)
            if response.error is not None:
                self._set_state("degraded")

    def _enqueue_recovery(self, message_type: str, payload: dict[str, Any]) -> None:
        with self._request_condition:
            if not any(item.message_type == message_type for item in self._requests):
                self._requests.append(_Request(message_type, payload))
            self._request_condition.notify()

    def _retain_safe_requests(self) -> None:
        with self._request_condition:
            self._requests = [
                request for request in self._requests
                if request.message_type not in self._MUTATING_REQUESTS
            ]

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
            "session_status": self.lifecycleStatus,
            "backend_shutdown": self.backendShutdown,
            "transport_closed": self.transportClosed,
            "reconnect_required": self.reconnectRequired,
        }.get(envelope.message_type)
        if signal is not None:
            signal.emit(envelope)

    def _set_state(self, state: str) -> None:
        self._state = state
        self.stateChanged.emit(state)
