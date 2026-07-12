"""Dependency-light client for the Hyge length-prefixed JSON protocol."""

from __future__ import annotations

import json
import socket
import uuid
from dataclasses import dataclass
from typing import Any, Callable

PROTOCOL_VERSION = 2
SUPPORTED_PROTOCOL_VERSIONS = (1, PROTOCOL_VERSION)
MAX_MESSAGE_BYTES = 16 * 1024 * 1024
MESSAGE_TYPES = {
    "hello", "hello_ack", "open_project", "open_scene", "activate_asset", "save_scene", "resolve_scene_reload", "request_world_snapshot",
    "select_entities", "edit_component", "edit_components", "add_component", "remove_component",
    "reparent_entity", "duplicate_entity", "destroy_entity", "instantiate_prefab",
    "undo", "redo", "set_editor_camera", "set_viewport_size", "open_viewport_transport", "close_viewport_transport", "viewport_input", "request_asset_preview",
    "request_asset_snapshot", "request_console_snapshot", "request_profiler_snapshot",
    "cancel_asset_preview", "world_snapshot", "selection_changed", "component_changed",
    "asset_changed", "asset_snapshot", "console_snapshot", "profiler_snapshot",
    "asset_preview_ready", "asset_preview_cancelled", "scene_reloaded", "scene_reload_conflict", "console_line", "profiler_sample", "viewport_frame_available", "viewport_transport_ready", "viewport_transport_reset",
    "command_completed", "engine_error", "server_shutdown", "session_status", "session_replaced", "backend_shutdown", "transport_closed", "reconnect_required", "lifecycle_status",
}


@dataclass
class Envelope:
    """A protocol envelope matching ``protocol/editor.schema.json``."""

    message_id: str
    message_type: str
    payload: dict[str, Any]
    protocol_version: int = PROTOCOL_VERSION
    error: dict[str, str] | None = None
    correlation_id: str | None = None

    def validate(self) -> None:
        """Validate fields shared by every version-one envelope."""
        if self.protocol_version not in SUPPORTED_PROTOCOL_VERSIONS:
            raise ValueError(f"unsupported protocol version: {self.protocol_version}")
        if not self.message_id:
            raise ValueError("editor protocol message_id must not be empty")
        if self.message_type not in MESSAGE_TYPES:
            raise ValueError(f"unknown editor protocol message type: {self.message_type}")
        if not isinstance(self.payload, dict):
            raise ValueError("editor protocol payload must be an object")
        if self.error is not None:
            if not {"code", "message"}.issubset(self.error):
                raise ValueError("editor protocol error must contain code and message")
            if not isinstance(self.error["code"], str) or not isinstance(self.error["message"], str):
                raise ValueError("editor protocol error code and message must be strings")
        if self.message_type == "engine_error" and self.error is None:
            raise ValueError("engine_error requires error")
        if self.message_type != "engine_error" and self.error is not None:
            raise ValueError("only engine_error may contain error")
        if self.correlation_id == "":
            raise ValueError("editor protocol correlation_id must not be empty")

    def to_bytes(self) -> bytes:
        self.validate()
        body = json.dumps({
            "protocol_version": self.protocol_version,
            "message_id": self.message_id,
            "message_type": self.message_type,
            "payload": self.payload,
            **({"error": self.error} if self.error is not None else {}),
            **({"correlation_id": self.correlation_id} if self.correlation_id else {}),
        }, separators=(",", ":")).encode("utf-8")
        if len(body) > MAX_MESSAGE_BYTES:
            raise ValueError("editor protocol message is too large")
        return len(body).to_bytes(4, "big") + body

    @classmethod
    def from_bytes(cls, body: bytes) -> "Envelope":
        if not body or len(body) > MAX_MESSAGE_BYTES:
            raise ValueError("editor protocol message has an invalid size")
        try:
            data = json.loads(body)
        except (TypeError, json.JSONDecodeError) as error:
            raise ValueError("editor protocol body is not valid JSON") from error
        if not isinstance(data, dict):
            raise ValueError("editor protocol envelope must be an object")
        required = {"protocol_version", "message_id", "message_type", "payload"}
        if not required.issubset(data):
            raise ValueError("editor protocol envelope is missing required fields")
        envelope = cls(
            message_id=data["message_id"],
            message_type=data["message_type"],
            payload=data.get("payload", {}),
            protocol_version=data["protocol_version"],
            error=data.get("error"),
            correlation_id=data.get("correlation_id"),
        )
        envelope.validate()
        return envelope


class EditorClient:
    """Synchronous protocol client used by the Qt adapter thread."""

    def __init__(
        self,
        address: str,
        token: str,
        timeout: float = 5.0,
        client_name: str = "hyge-editor-python",
        supported_protocol_versions: tuple[int, ...] = SUPPORTED_PROTOCOL_VERSIONS,
    ) -> None:
        host, port = address.rsplit(":", 1)
        self._address = (host, int(port))
        self._token = token
        self._timeout = timeout
        self._client_name = client_name
        self._supported_protocol_versions = supported_protocol_versions
        self._session_id: str | None = None
        self._socket: socket.socket | None = None
        self.lifecycle_statuses: list[Envelope] = []

    def connect(self) -> Envelope:
        """Connect and complete the authenticated handshake."""
        self._socket = socket.create_connection(self._address, self._timeout)
        response = self.request(
            "hello",
            {
                "client_name": self._client_name,
                "supported_protocol_versions": list(self._supported_protocol_versions),
                "session_id": self._session_id,
                "session_token": self._token,
            },
        )
        if response.error is not None:
            self.close()
            return response
        selected = response.payload.get("selected_protocol_version")
        session_id = response.payload.get("session_id")
        if selected not in self._supported_protocol_versions or not isinstance(session_id, str) or not session_id:
            self.close()
            raise ValueError("invalid editor handshake response")
        self._session_id = session_id
        self._protocol_version = selected
        return response

    def reconnect(self) -> Envelope:
        """Reconnect while asking the service to resume the current session."""
        if self._session_id is None:
            raise RuntimeError("cannot reconnect before a successful handshake")
        self.close()
        return self.connect()

    def resume_session(self, session_id: str | None) -> None:
        """Set the session identity used by the next handshake."""
        self._session_id = session_id

    @property
    def session_id(self) -> str | None:
        """The current server-issued session identity."""
        return self._session_id

    def request(self, message_type: str, payload: dict[str, Any] | None = None) -> Envelope:
        """Send a request and read through lifecycle events to its response."""
        return self.request_observing(message_type, payload)

    def request_observing(
        self,
        message_type: str,
        payload: dict[str, Any] | None = None,
        on_event: Callable[[Envelope], None] | None = None,
    ) -> Envelope:
        """Send a request while delivering intermediate envelopes to ``on_event``.

        The Rust service may answer one request with lifecycle, snapshot and
        selection events before the terminal response.  The original
        ``request`` API remains synchronous, while this hook lets the Qt
        session retain those envelopes without changing the wire contract.
        """
        if self._socket is None:
            raise RuntimeError("editor client is not connected")
        envelope = Envelope(str(uuid.uuid4()), message_type, payload or {})
        self._socket.sendall(envelope.to_bytes())
        try:
            while True:
                response = self._read_envelope()
                if response.message_type == "lifecycle_status":
                    self.lifecycle_statuses.append(response)
                if on_event is not None and response.message_type != self._terminal_type(message_type):
                    on_event(response)
                if response.message_type == "engine_error" or response.error is not None or response.message_type in {self._terminal_type(message_type), "hello_ack"}:
                    return response
        except socket.timeout as error:
            raise TimeoutError("editor protocol response timed out") from error

    @staticmethod
    def _terminal_type(request_type: str) -> str:
        """Return the terminal response type for a request."""
        return {
            "hello": "hello_ack",
            "request_asset_snapshot": "asset_snapshot",
            "request_console_snapshot": "console_snapshot",
            "request_profiler_snapshot": "profiler_snapshot",
            "request_asset_preview": "asset_preview_ready",
            "cancel_asset_preview": "asset_preview_cancelled",
            "request_world_snapshot": "selection_changed",
            "resolve_scene_reload": "command_completed",
            "open_viewport_transport": "viewport_transport_ready",
            "viewport_transport_reset": "viewport_transport_reset",
            "server_shutdown": "server_shutdown",
        }.get(request_type, "command_completed")

    def _read_envelope(self) -> Envelope:
        """Read one length-prefixed envelope from the connected socket."""
        header = self._read_exact(4)
        length = int.from_bytes(header, "big")
        if length == 0 or length > MAX_MESSAGE_BYTES:
            raise ValueError("editor protocol response is too large")
        return Envelope.from_bytes(self._read_exact(length))

    def close(self) -> None:
        """Close the client socket."""
        if self._socket is not None:
            self._socket.close()
            self._socket = None

    def _read_exact(self, length: int) -> bytes:
        assert self._socket is not None
        chunks: list[bytes] = []
        remaining = length
        while remaining:
            try:
                chunk = self._socket.recv(remaining)
            except socket.timeout as error:
                raise TimeoutError("editor protocol response timed out") from error
            if not chunk:
                raise ConnectionError("editor service closed the connection")
            chunks.append(chunk)
            remaining -= len(chunk)
        return b"".join(chunks)

    def _read_response(self) -> Envelope:
        """Read the next terminal response, retaining lifecycle events."""
        response = self._read_envelope()
        if response.message_type == "lifecycle_status":
            self.lifecycle_statuses.append(response)
            return self._read_response()
        return response

    def request_asset_snapshot(self) -> Envelope:
        """Request the bounded asset tree and dependency graph."""
        return self.request("request_asset_snapshot")

    def request_world_snapshot(self) -> Envelope:
        """Request a fresh immutable world and selection snapshot."""
        return self.request("request_world_snapshot")

    def select_entities(self, entities: list[int], shift: bool = False) -> Envelope:
        """Select entities through the engine-owned selection resource."""
        return self.request("select_entities", {"entities": entities, "shift": shift})

    def edit_component(
        self,
        expected_revision: int,
        entity: int,
        type_path: str,
        value: Any,
        field_path: str | None = None,
    ) -> Envelope:
        """Edit one reflected component field."""
        payload: dict[str, Any] = {
            "expected_revision": expected_revision,
            "entity": entity,
            "type_path": type_path,
            "value": value,
        }
        if field_path is not None:
            payload["field_path"] = field_path
        return self.request("edit_component", payload)

    def edit_components(
        self,
        expected_revision: int,
        entities: list[int],
        type_path: str,
        value: Any,
        field_path: str | None = None,
    ) -> Envelope:
        """Edit one reflected component field atomically across entities."""
        payload: dict[str, Any] = {
            "expected_revision": expected_revision,
            "entities": entities,
            "type_path": type_path,
            "value": value,
        }
        if field_path is not None:
            payload["field_path"] = field_path
        return self.request("edit_components", payload)

    def reparent_entity(self, expected_revision: int, entity: int, new_parent: int | None) -> Envelope:
        """Reparent one entity through the engine-owned hierarchy."""
        return self.request("reparent_entity", {"expected_revision": expected_revision, "entity": entity, "new_parent": new_parent})

    def request_console_snapshot(self, min_level: str | None = None, target_prefix: str | None = None) -> Envelope:
        """Request retained console lines with optional filters."""
        payload = {key: value for key, value in (("min_level", min_level), ("target_prefix", target_prefix)) if value is not None}
        return self.request("request_console_snapshot", payload)

    def request_profiler_snapshot(self) -> Envelope:
        """Request the bounded profiler history."""
        return self.request("request_profiler_snapshot")

    def set_editor_camera(
        self,
        position: tuple[float, float, float],
        rotation: tuple[float, float, float, float],
        fov_degrees: float = 60.0,
        near: float = 0.1,
        far: float = 1000.0,
    ) -> Envelope:
        """Update the session-owned editor camera."""
        return self.request("set_editor_camera", {
            "position": list(position),
            "rotation": list(rotation),
            "fov_degrees": fov_degrees,
            "near": near,
            "far": far,
        })

    def set_viewport_size(self, width: int, height: int) -> Envelope:
        """Resize the session-owned editor viewport target."""
        return self.request("set_viewport_size", {"width": width, "height": height})

    def open_viewport_transport(self) -> Envelope:
        """Open the negotiated shared-memory viewport transport."""
        return self.request("open_viewport_transport")

    def send_viewport_input(self, generation: int, expected_input_revision: int, events: list[dict[str, Any]]) -> Envelope:
        """Send one coalesced, revisioned viewport input batch."""
        return self.request("viewport_input", {"generation": generation, "expected_input_revision": expected_input_revision, "input_revision": expected_input_revision + 1, "events": events})

    def request_asset_preview(self, asset_id: str, job_id: str | None = None) -> Envelope:
        """Request a deterministic asset preview."""
        payload: dict[str, Any] = {"asset_id": asset_id}
        if job_id is not None:
            payload["job_id"] = job_id
        return self.request("request_asset_preview", payload)

    def cancel_asset_preview(self, job_id: str) -> Envelope:
        """Cancel an asset preview job."""
        return self.request("cancel_asset_preview", {"job_id": job_id})
