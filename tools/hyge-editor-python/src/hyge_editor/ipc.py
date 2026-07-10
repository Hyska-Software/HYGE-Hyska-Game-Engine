"""Dependency-light client for the Hyge length-prefixed JSON protocol."""

from __future__ import annotations

import json
import socket
import uuid
from dataclasses import dataclass
from typing import Any

PROTOCOL_VERSION = 1
MAX_MESSAGE_BYTES = 16 * 1024 * 1024


@dataclass
class Envelope:
    """A protocol envelope matching ``protocol/editor.schema.json``."""

    message_id: str
    message_type: str
    payload: dict[str, Any]
    protocol_version: int = PROTOCOL_VERSION
    error: dict[str, str] | None = None

    def to_bytes(self) -> bytes:
        body = json.dumps({
            "protocol_version": self.protocol_version,
            "message_id": self.message_id,
            "message_type": self.message_type,
            "payload": self.payload,
            **({"error": self.error} if self.error is not None else {}),
        }, separators=(",", ":")).encode("utf-8")
        if len(body) > MAX_MESSAGE_BYTES:
            raise ValueError("editor protocol message is too large")
        return len(body).to_bytes(4, "big") + body

    @classmethod
    def from_bytes(cls, body: bytes) -> "Envelope":
        data = json.loads(body)
        if data["protocol_version"] != PROTOCOL_VERSION:
            raise ValueError(f"unsupported protocol version: {data['protocol_version']}")
        return cls(
            message_id=data["message_id"],
            message_type=data["message_type"],
            payload=data.get("payload", {}),
            protocol_version=data["protocol_version"],
            error=data.get("error"),
        )


class EditorClient:
    """Synchronous protocol client used by the Qt adapter thread."""

    def __init__(self, address: str, token: str, timeout: float = 5.0) -> None:
        host, port = address.rsplit(":", 1)
        self._address = (host, int(port))
        self._token = token
        self._timeout = timeout
        self._socket: socket.socket | None = None

    def connect(self) -> Envelope:
        """Connect and complete the authenticated handshake."""
        self._socket = socket.create_connection(self._address, self._timeout)
        return self.request("hello", {"session_token": self._token})

    def request(self, message_type: str, payload: dict[str, Any] | None = None) -> Envelope:
        """Send a request and read exactly one response."""
        if self._socket is None:
            raise RuntimeError("editor client is not connected")
        envelope = Envelope(str(uuid.uuid4()), message_type, payload or {})
        self._socket.sendall(envelope.to_bytes())
        header = self._read_exact(4)
        length = int.from_bytes(header, "big")
        if length > MAX_MESSAGE_BYTES:
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
            chunk = self._socket.recv(remaining)
            if not chunk:
                raise ConnectionError("editor service closed the connection")
            chunks.append(chunk)
            remaining -= len(chunk)
        return b"".join(chunks)
