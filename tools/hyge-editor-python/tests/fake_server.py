"""Small length-prefixed fake editor service for frontend tests."""

from __future__ import annotations

import json
import socket
import threading
from typing import Any

from hyge_editor.ipc import Envelope


WORLD = {
    "revision": 2,
    "scene_revision": 1,
    "hierarchy": [{"entity": 1, "name": "Root", "parent": None, "children": [2], "scene_id": "scene-root"}, {"entity": 2, "name": "Camera", "parent": 1, "children": [], "scene_id": "scene-camera"}],
    "entities": [{"entity": 1, "components": [{"type_id": "name", "type_path": "hyge_scene::Name", "value": {"value": "Root"}, "error": None}]}, {"entity": 2, "components": []}],
    "component_catalog": [],
    "selection": [2],
    "selection_scene_ids": ["scene-camera"],
    "diagnostics": [],
}


class FakeEditorServer:
    """Threaded fake server that records requests and serves deterministic snapshots."""

    def __init__(self) -> None:
        self.requests: list[str] = []
        self._listener = socket.socket()
        self._listener.bind(("127.0.0.1", 0))
        self._listener.listen(1)
        self.address = f"127.0.0.1:{self._listener.getsockname()[1]}"
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, name="fake-hyge-editor", daemon=True)

    def start(self) -> None:
        self._thread.start()

    def close(self) -> None:
        self._stop.set()
        try:
            self._listener.close()
        finally:
            self._thread.join(timeout=2)

    def _run(self) -> None:
        try:
            connection, _ = self._listener.accept()
        except OSError:
            return
        with connection:
            connection.settimeout(0.2)
            while not self._stop.is_set():
                try:
                    body = self._read(connection)
                except (OSError, ConnectionError, TimeoutError):
                    continue
                if body is None:
                    return
                request = json.loads(body)
                message = Envelope.from_bytes(body)
                self.requests.append(message.message_type)
                self._respond(connection, message)

    @staticmethod
    def _read(connection: socket.socket) -> bytes | None:
        header = b""
        while len(header) < 4:
            chunk = connection.recv(4 - len(header))
            if not chunk:
                return None
            header += chunk
        length = int.from_bytes(header, "big")
        body = b""
        while len(body) < length:
            chunk = connection.recv(length - len(body))
            if not chunk:
                return None
            body += chunk
        return body

    def _send(self, connection: socket.socket, request: Envelope, message_type: str, payload: dict[str, Any]) -> None:
        response = Envelope(request.message_id, message_type, payload, correlation_id=request.message_id)
        connection.sendall(response.to_bytes())

    def _respond(self, connection: socket.socket, request: Envelope) -> None:
        if request.message_type == "hello":
            self._send(connection, request, "hello_ack", {"selected_protocol_version": 2, "session_id": "fake-session", "resumed": False, "server": "hyge-editor", "request_timeout_ms": 1000})
        elif request.message_type in {"open_project", "open_scene"}:
            self._send(connection, request, "lifecycle_status", {"session_id": "fake-session", "state": "loading", "details": {}})
            self._send(connection, request, "world_snapshot", WORLD)
            self._send(connection, request, "selection_changed", {"revision": 2, "scene_revision": 1, "entities": [2], "scene_ids": ["scene-camera"]})
            self._send(connection, request, "command_completed", {"command": request.message_type, "revision": 2})
        elif request.message_type == "request_asset_snapshot":
            self._send(connection, request, "asset_snapshot", {"snapshot_revision": 1, "nodes": [{"asset_id": "a" * 64, "path": "assets/test.hyge-mesh"}], "edges": [], "diagnostics": []})
        elif request.message_type == "request_console_snapshot":
            self._send(connection, request, "console_snapshot", {"lines": [{"sequence": 1, "timestamp_ms": 1, "level": "info", "target": "fake", "message": "ready", "file": None, "line": None}], "filter": {}})
        elif request.message_type == "request_profiler_snapshot":
            self._send(connection, request, "profiler_snapshot", {"samples": [{"frame_id": 1, "frame_time_ms": 16.0, "fps": 60.0, "gpu_time_ms": 4.0, "draw_calls": 2, "instance_count": 3, "process_resident_bytes": None, "asset_cache_bytes": 0, "passes": []}]})
        elif request.message_type == "open_viewport_transport":
            self._send(connection, request, "viewport_transport_ready", {"mapping_name": "fake-mapping", "generation": 1, "width": 2, "height": 2, "pixel_format": "rgba8_srgb", "ring_slots": 3})
        elif request.message_type == "close_viewport_transport":
            self._send(connection, request, "command_completed", {"command": "close_viewport_transport", "released": True})
        else:
            self._send(connection, request, "command_completed", {"command": request.message_type})
