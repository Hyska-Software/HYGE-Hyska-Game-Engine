"""Dependency-light client for the Hyge length-prefixed JSON protocol."""

from __future__ import annotations

import json
import socket
import uuid
from dataclasses import dataclass
from typing import Any

PROTOCOL_VERSION = 1
MAX_MESSAGE_BYTES = 16 * 1024 * 1024
MESSAGE_TYPES = {
    "hello", "hello_ack", "open_project", "open_scene", "save_scene",
    "select_entities", "edit_component", "add_component", "remove_component",
    "reparent_entity", "duplicate_entity", "destroy_entity", "instantiate_prefab",
    "undo", "redo", "set_editor_camera", "set_viewport_size", "request_asset_preview",
    "world_snapshot", "selection_changed", "component_changed", "asset_changed",
    "scene_reloaded", "console_line", "profiler_sample", "viewport_frame_available",
    "command_completed", "engine_error", "server_shutdown",
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
        if self.protocol_version != PROTOCOL_VERSION:
            raise ValueError(f"unsupported protocol version: {self.protocol_version}")
        if not self.message_id:
            raise ValueError("editor protocol message_id must not be empty")
        if self.message_type not in MESSAGE_TYPES:
            raise ValueError(f"unknown editor protocol message type: {self.message_type}")
        if not isinstance(self.payload, dict):
            raise ValueError("editor protocol payload must be an object")
        if self.error is not None:
            if set(self.error) != {"code", "message"}:
                raise ValueError("editor protocol error must contain code and message")
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
        supported_protocol_versions: tuple[int, ...] = (PROTOCOL_VERSION,),
    ) -> None:
        host, port = address.rsplit(":", 1)
        self._address = (host, int(port))
        self._token = token
        self._timeout = timeout
        self._client_name = client_name
        self._supported_protocol_versions = supported_protocol_versions
        self._session_id: str | None = None
        self._socket: socket.socket | None = None

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
        if selected != PROTOCOL_VERSION or not isinstance(session_id, str) or not session_id:
            self.close()
            raise ValueError("invalid editor handshake response")
        self._session_id = session_id
        return response

    def reconnect(self) -> Envelope:
        """Reconnect while asking the service to resume the current session."""
        if self._session_id is None:
            raise RuntimeError("cannot reconnect before a successful handshake")
        self.close()
        return self.connect()

    @property
    def session_id(self) -> str | None:
        """The current server-issued session identity."""
        return self._session_id

    def request(self, message_type: str, payload: dict[str, Any] | None = None) -> Envelope:
        """Send a request and read exactly one response."""
        if self._socket is None:
            raise RuntimeError("editor client is not connected")
        envelope = Envelope(str(uuid.uuid4()), message_type, payload or {})
        self._socket.sendall(envelope.to_bytes())
        header = self._read_exact(4)
        length = int.from_bytes(header, "big")
        if length == 0 or length > MAX_MESSAGE_BYTES:
            raise ValueError("editor protocol response is too large")
        try:
            return Envelope.from_bytes(self._read_exact(length))
        except socket.timeout as error:
            raise TimeoutError("editor protocol response timed out") from error

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
