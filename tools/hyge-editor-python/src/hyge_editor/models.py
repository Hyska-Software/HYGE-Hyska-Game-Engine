"""Qt models backed by immutable protocol snapshots."""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QAbstractListModel, QAbstractItemModel, QModelIndex, Qt, QObject, Signal


def _payload(value: Any) -> dict[str, Any]:
    """Extract an envelope payload while tolerating test dictionaries."""
    return value.payload if hasattr(value, "payload") else value


class SnapshotListModel(QAbstractListModel):
    """Base model that replaces a disposable list from one snapshot."""

    def __init__(self, roles: dict[str, int], parent: Any = None) -> None:
        super().__init__(parent)
        self._roles = roles
        self._rows: list[dict[str, Any]] = []

    def roleNames(self) -> dict[int, bytes]:
        return {value: key.encode() for key, value in self._roles.items()}

    def rowCount(self, parent: QModelIndex = QModelIndex()) -> int:
        return 0 if parent.isValid() else len(self._rows)

    def data(self, index: QModelIndex, role: int = Qt.ItemDataRole.DisplayRole) -> Any:
        if not index.isValid() or not 0 <= index.row() < len(self._rows):
            return None
        if role == Qt.ItemDataRole.DisplayRole:
            return self._rows[index.row()].get("display", "")
        key = next((key for key, value in self._roles.items() if value == role), None)
        return self._rows[index.row()].get(key) if key else None

    def _replace(self, rows: list[dict[str, Any]]) -> None:
        self.beginResetModel()
        self._rows = rows
        self.endResetModel()


class HierarchyNode:
    """Internal tree node for the hierarchy model."""

    def __init__(self, row: dict[str, Any], parent: "HierarchyNode | None") -> None:
        self.row = row
        self.parent = parent
        self.children: list[HierarchyNode] = []


class HierarchyModel(QAbstractItemModel):
    """Read-only hierarchy model derived from ``world_snapshot``."""

    EntityRole = Qt.ItemDataRole.UserRole + 1
    NameRole = Qt.ItemDataRole.UserRole + 2
    SceneIdRole = Qt.ItemDataRole.UserRole + 3
    SelectedRole = Qt.ItemDataRole.UserRole + 4
    ParentRole = Qt.ItemDataRole.UserRole + 5

    def __init__(self, parent: Any = None) -> None:
        super().__init__(parent)
        self._root = HierarchyNode({}, None)
        self._nodes: dict[int, HierarchyNode] = {}
        self._selection: set[int] = set()

    def roleNames(self) -> dict[int, bytes]:
        return {
            self.EntityRole: b"entity",
            self.NameRole: b"name",
            self.SceneIdRole: b"sceneId",
            self.SelectedRole: b"selected",
            self.ParentRole: b"parentEntity",
        }

    def rowCount(self, parent: QModelIndex = QModelIndex()) -> int:
        node = self._node(parent)
        return len(node.children)

    def columnCount(self, parent: QModelIndex = QModelIndex()) -> int:
        return 1

    def index(self, row: int, column: int, parent: QModelIndex = QModelIndex()) -> QModelIndex:
        if column != 0:
            return QModelIndex()
        node = self._node(parent)
        if not 0 <= row < len(node.children):
            return QModelIndex()
        return self.createIndex(row, column, node.children[row])

    def parent(self, index: QModelIndex) -> QModelIndex:
        if not index.isValid():
            return QModelIndex()
        node = index.internalPointer()
        parent = node.parent
        if parent is None or parent is self._root:
            return QModelIndex()
        return self.createIndex(parent.parent.children.index(parent), 0, parent)

    def data(self, index: QModelIndex, role: int = Qt.ItemDataRole.DisplayRole) -> Any:
        if not index.isValid():
            return None
        row = index.internalPointer().row
        if role in (Qt.ItemDataRole.DisplayRole, self.NameRole):
            return row.get("name", str(row.get("entity", "")))
        if role == self.EntityRole:
            return row.get("entity")
        if role == self.SceneIdRole:
            return row.get("scene_id", row.get("sceneId"))
        if role == self.ParentRole:
            return row.get("parent")
        if role == self.SelectedRole:
            return row.get("entity") in self._selection
        return None

    def update_snapshot(self, snapshot: Any) -> None:
        """Replace the tree from a world snapshot without retaining backend objects."""
        payload = _payload(snapshot)
        self.beginResetModel()
        self._root = HierarchyNode({}, None)
        self._nodes = {}
        rows = payload.get("hierarchy", []) if isinstance(payload, dict) else []
        for row in rows:
            if isinstance(row, dict) and isinstance(row.get("entity"), int):
                self._nodes[row["entity"]] = HierarchyNode(dict(row), None)
        for node in self._nodes.values():
            parent_id = node.row.get("parent")
            parent = self._nodes.get(parent_id)
            node.parent = parent or self._root
            node.parent.children.append(node)
        self._selection = set(payload.get("selection", [])) if isinstance(payload, dict) else set()
        self._root.children.sort(key=lambda node: node.row.get("entity", 0))
        self.endResetModel()

    def update_selection(self, selection: Any) -> None:
        """Update selection roles from a selection event."""
        payload = _payload(selection)
        self._selection = set(payload.get("entities", [])) if isinstance(payload, dict) else set()
        if self._nodes:
            top_left = self.index(0, 0)
            bottom_right = self.index(max(0, self.rowCount() - 1), 0)
            self.dataChanged.emit(top_left, bottom_right, [self.SelectedRole])

    def _node(self, index: QModelIndex) -> HierarchyNode:
        return index.internalPointer() if index.isValid() else self._root


class InspectorModel(SnapshotListModel):
    """Read-only reflected components for the current selection."""

    TypeIdRole = Qt.ItemDataRole.UserRole + 1
    TypePathRole = Qt.ItemDataRole.UserRole + 2
    ValueRole = Qt.ItemDataRole.UserRole + 3
    ErrorRole = Qt.ItemDataRole.UserRole + 4
    EntityRole = Qt.ItemDataRole.UserRole + 5

    def __init__(self, parent: Any = None) -> None:
        super().__init__({"typeId": self.TypeIdRole, "typePath": self.TypePathRole, "value": self.ValueRole, "error": self.ErrorRole, "entity": self.EntityRole}, parent)

    def update_snapshot(self, snapshot: Any) -> None:
        payload = _payload(snapshot)
        selected = set(payload.get("selection", [])) if isinstance(payload, dict) else set()
        entity = next((row for row in payload.get("entities", []) if row.get("entity") in selected), None) if isinstance(payload, dict) else None
        rows = []
        for component in (entity or {}).get("components", []):
            rows.append({"display": component.get("type_path", ""), "typeId": component.get("type_id"), "typePath": component.get("type_path"), "value": component.get("value"), "error": component.get("error"), "entity": entity.get("entity")})
        self._replace(rows)


class AssetModel(SnapshotListModel):
    """Asset tree rows from the authoritative AssetDb snapshot."""

    AssetIdRole = Qt.ItemDataRole.UserRole + 1
    PathRole = Qt.ItemDataRole.UserRole + 2

    def __init__(self, parent: Any = None) -> None:
        super().__init__({"assetId": self.AssetIdRole, "path": self.PathRole}, parent)

    def update_snapshot(self, snapshot: Any) -> None:
        payload = _payload(snapshot)
        self._replace([{"display": row.get("path", ""), "assetId": row.get("asset_id"), "path": row.get("path")} for row in payload.get("nodes", []) if isinstance(row, dict)] if isinstance(payload, dict) else [])


class ConsoleModel(SnapshotListModel):
    """Bounded tracing lines from the Rust console service."""

    SequenceRole = Qt.ItemDataRole.UserRole + 1
    LevelRole = Qt.ItemDataRole.UserRole + 2
    TargetRole = Qt.ItemDataRole.UserRole + 3
    MessageRole = Qt.ItemDataRole.UserRole + 4
    FileRole = Qt.ItemDataRole.UserRole + 5
    LineRole = Qt.ItemDataRole.UserRole + 6

    def __init__(self, parent: Any = None) -> None:
        super().__init__({"sequence": self.SequenceRole, "level": self.LevelRole, "target": self.TargetRole, "message": self.MessageRole, "file": self.FileRole, "line": self.LineRole}, parent)

    def update_snapshot(self, snapshot: Any) -> None:
        payload = _payload(snapshot)
        self._replace([{**row, "display": f"[{row.get('level', '')}] {row.get('message', '')}"} for row in payload.get("lines", []) if isinstance(row, dict)] if isinstance(payload, dict) else [])


class ProfilerModel(SnapshotListModel):
    """Bounded frame telemetry rows from the Rust profiler service."""

    FrameRole = Qt.ItemDataRole.UserRole + 1
    FpsRole = Qt.ItemDataRole.UserRole + 2
    FrameTimeRole = Qt.ItemDataRole.UserRole + 3
    GpuTimeRole = Qt.ItemDataRole.UserRole + 4
    DrawCallsRole = Qt.ItemDataRole.UserRole + 5
    InstanceCountRole = Qt.ItemDataRole.UserRole + 6
    MemoryRole = Qt.ItemDataRole.UserRole + 7
    PassesRole = Qt.ItemDataRole.UserRole + 8

    def __init__(self, parent: Any = None) -> None:
        super().__init__({"frameId": self.FrameRole, "fps": self.FpsRole, "frameTimeMs": self.FrameTimeRole, "gpuTimeMs": self.GpuTimeRole, "drawCalls": self.DrawCallsRole, "instanceCount": self.InstanceCountRole, "memory": self.MemoryRole, "passes": self.PassesRole}, parent)

    def update_snapshot(self, snapshot: Any) -> None:
        payload = _payload(snapshot)
        rows = []
        for row in payload.get("samples", []) if isinstance(payload, dict) else []:
            if isinstance(row, dict):
                rows.append({**row, "display": f"Frame {row.get('frame_id', '')}: {row.get('frame_time_ms', 0):.2f} ms"})
        self._replace(rows)


class ViewportStateModel(QObject):
    """Small QML-facing state model for the viewport transport."""

    changed = Signal()

    def __init__(self, parent: Any = None) -> None:
        super().__init__(parent)
        self.width = 640
        self.height = 360
        self.frame_id = 0
        self.generation = 0
        self.dropped_frames = 0
        self.state = "disconnected"
        self.last_error = ""

    def update_transport(self, payload: Any) -> None:
        data = _payload(payload)
        self.width = int(data.get("width", self.width))
        self.height = int(data.get("height", self.height))
        self.generation = int(data.get("generation", self.generation))
        self.state = "connected"
        self.changed.emit()

    def record_frame(self, frame_id: int, dropped: int = 0) -> None:
        self.frame_id = frame_id
        self.dropped_frames += dropped
        self.changed.emit()

    def fail(self, message: str) -> None:
        self.state = "degraded"
        self.last_error = message
        self.changed.emit()
