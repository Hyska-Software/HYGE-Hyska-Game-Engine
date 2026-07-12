"""Qt interaction controller for revisioned hierarchy and inspector commands."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from PySide6.QtCore import QObject, Property, Signal, Slot

from .ipc import Envelope
from .session import EditorSession


@dataclass
class _PendingMutation:
    """One mutation waiting for its authoritative command response."""

    kind: str
    key: str


class EditorInteractionController(QObject):
    """Serializes editor mutations and owns only transient UI conflict state."""

    conflictChanged = Signal()
    conflictFieldChanged = Signal(str)
    commandError = Signal(str)
    revisionChanged = Signal(int)

    def __init__(self, session: EditorSession, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._session = session
        self._revision = 0
        self._selection: list[int] = []
        self._pending: _PendingMutation | None = None
        self._queued: dict[str, tuple[str, dict[str, Any]]] = {}
        self._conflict: dict[str, Any] | None = None
        session.worldSnapshot.connect(self._on_snapshot)
        session.selectionChanged.connect(self._on_selection)
        session.commandCompleted.connect(self._on_command_completed)
        session.engineError.connect(self._on_error)

    @Property(int, notify=revisionChanged)
    def revision(self) -> int:
        """Return the latest engine-owned snapshot revision."""
        return self._revision

    @Property(bool, notify=conflictChanged)
    def hasConflict(self) -> bool:
        """Return whether a stale field edit is awaiting refresh."""
        return self._conflict is not None

    @Property(str, notify=conflictChanged)
    def conflictMessage(self) -> str:
        """Return the current conflict diagnostic."""
        return (self._conflict or {}).get("message", "")

    @Slot(int, bool)
    def select_entity(self, entity: int, shift: bool = False) -> None:
        """Request authoritative single or additive selection."""
        self._session.request("select_entities", {"entities": [entity], "shift": shift})

    @Slot(int, object)
    def reparent_entity(self, entity: int, new_parent: int | None) -> None:
        """Request a reparent operation against the latest revision."""
        self._send_or_report(
            "reparent_entity",
            f"reparent:{entity}",
            {"expected_revision": self._revision, "entity": entity, "new_parent": new_parent},
        )

    @Slot(str, str, str, object)
    def edit_field(self, type_path: str, field_path: str, field_id: str, value: Any) -> None:
        """Queue a live batch edit for the current engine-owned selection."""
        if self._conflict is not None:
            return
        entities = list(self._selection)
        if not entities:
            self.commandError.emit("cannot edit without a selected entity")
            return
        payload = {
            "expected_revision": self._revision,
            "entities": entities,
            "type_path": type_path,
            "field_path": field_path or None,
            "value": value,
        }
        self._send_or_queue("edit_components", field_id, payload)

    @Slot()
    def refresh_snapshot(self) -> None:
        """Request an authoritative snapshot after a conflict or manual refresh."""
        self._session.request("request_world_snapshot")

    def _send_or_report(self, kind: str, key: str, payload: dict[str, Any]) -> None:
        if self._pending is not None:
            self.commandError.emit("another editor command is still pending")
            return
        self._pending = _PendingMutation(kind, key)
        self._session.request(kind, payload)

    def _send_or_queue(self, kind: str, key: str, payload: dict[str, Any]) -> None:
        if self._pending is not None:
            self._queued[key] = (kind, payload)
            return
        self._pending = _PendingMutation(kind, key)
        self._session.request(kind, payload)

    def _on_snapshot(self, envelope: Envelope) -> None:
        payload = envelope.payload if hasattr(envelope, "payload") else envelope
        revision = payload.get("revision") if isinstance(payload, dict) else None
        if isinstance(revision, int):
            self._set_revision(revision)
        if self._conflict is not None:
            self._conflict = None
            self.conflictFieldChanged.emit("")
            self.conflictChanged.emit()
        self._drain_queue()

    def _on_selection(self, envelope: Envelope) -> None:
        payload = envelope.payload if hasattr(envelope, "payload") else envelope
        entities = payload.get("entities", []) if isinstance(payload, dict) else []
        if isinstance(entities, list):
            self._selection = [entity for entity in entities if isinstance(entity, int)]

    def _on_command_completed(self, envelope: Envelope) -> None:
        payload = envelope.payload if hasattr(envelope, "payload") else envelope
        revision = payload.get("revision") if isinstance(payload, dict) else None
        if isinstance(revision, int):
            self._set_revision(revision)
        if self._pending is not None:
            self._pending = None
        self._drain_queue()

    def _on_error(self, envelope: Envelope) -> None:
        error = envelope.error or {}
        if self._pending is not None and error.get("code") == "stale_revision":
            field_id = self._pending.key
            self._conflict = {"field_id": field_id, "message": error.get("message", "editor state is stale")}
            self._pending = None
            self._queued.clear()
            self.conflictFieldChanged.emit(field_id)
            self.conflictChanged.emit()
            self._session.request("request_world_snapshot")
            return
        self._pending = None
        self.commandError.emit(error.get("message", "editor command failed"))
        self._drain_queue()

    def _drain_queue(self) -> None:
        if self._pending is not None or self._conflict is not None or not self._queued:
            return
        key, (kind, payload) = next(iter(self._queued.items()))
        del self._queued[key]
        payload = {**payload, "expected_revision": self._revision}
        self._pending = _PendingMutation(kind, key)
        self._session.request(kind, payload)

    def _set_revision(self, revision: int) -> None:
        if revision != self._revision:
            self._revision = revision
            self.revisionChanged.emit(revision)
