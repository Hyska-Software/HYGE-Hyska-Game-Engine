"""Qt models backed by immutable Rust editor snapshots."""

from __future__ import annotations

import copy
import json
import os
from dataclasses import dataclass, field
from typing import Any

from pathlib import PurePosixPath

from PySide6.QtCore import QAbstractItemModel, QAbstractListModel, QModelIndex, QObject, Property, Qt, Signal, Slot


def _payload(value: Any) -> dict[str, Any]:
    """Extract an envelope payload while tolerating plain test dictionaries."""
    return value.payload if hasattr(value, "payload") else value


class SnapshotListModel(QAbstractListModel):
    """Base model that replaces a disposable list from one snapshot."""

    def __init__(self, roles: dict[str, int], parent: QObject | None = None) -> None:
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


@dataclass
class HierarchyNode:
    """Internal tree node for the hierarchy model."""

    row: dict[str, Any]
    parent: "HierarchyNode | None"
    children: list["HierarchyNode"] = field(default_factory=list)

    @property
    def key(self) -> str:
        return str(self.row.get("scene_id") or self.row.get("sceneId") or self.row.get("entity"))


class HierarchyModel(QAbstractItemModel):
    """Interactive tree backed by engine-owned hierarchy and selection events."""

    EntityRole = Qt.ItemDataRole.UserRole + 1
    NameRole = Qt.ItemDataRole.UserRole + 2
    SceneIdRole = Qt.ItemDataRole.UserRole + 3
    SelectedRole = Qt.ItemDataRole.UserRole + 4
    ParentRole = Qt.ItemDataRole.UserRole + 5
    DepthRole = Qt.ItemDataRole.UserRole + 6
    HasChildrenRole = Qt.ItemDataRole.UserRole + 7
    ExpandedRole = Qt.ItemDataRole.UserRole + 8

    def __init__(self, interaction: Any = None, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._interaction = interaction
        self._root = HierarchyNode({}, None)
        self._nodes: dict[int, HierarchyNode] = {}
        self._selection: set[int] = set()
        self._expanded_keys: set[str] = set()

    def roleNames(self) -> dict[int, bytes]:
        return {
            self.EntityRole: b"entity",
            self.NameRole: b"name",
            self.SceneIdRole: b"sceneId",
            self.SelectedRole: b"selected",
            self.ParentRole: b"parentEntity",
            self.DepthRole: b"depth",
            self.HasChildrenRole: b"hasChildren",
            self.ExpandedRole: b"expanded",
        }

    def rowCount(self, parent: QModelIndex = QModelIndex()) -> int:
        return len(self._node(parent).children)

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
        node: HierarchyNode = index.internalPointer()
        parent = node.parent
        if parent is None or parent is self._root:
            return QModelIndex()
        return self.createIndex(parent.parent.children.index(parent), 0, parent)

    def data(self, index: QModelIndex, role: int = Qt.ItemDataRole.DisplayRole) -> Any:
        if not index.isValid():
            return None
        node: HierarchyNode = index.internalPointer()
        row = node.row
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
        if role == self.DepthRole:
            return self._depth(node)
        if role == self.HasChildrenRole:
            return bool(node.children)
        if role == self.ExpandedRole:
            return node.key in self._expanded_keys
        return None

    @Slot(int, bool)
    def select_entity(self, entity: int, shift: bool = False) -> None:
        """Delegate selection to the engine-owned selection resource."""
        if self._interaction is not None:
            self._interaction.select_entity(entity, shift)

    @Slot(int, bool)
    def set_expanded(self, entity: int, expanded: bool) -> None:
        """Persist expansion state locally by stable scene/entity key."""
        node = self._nodes.get(entity)
        if node is None:
            return
        if expanded:
            self._expanded_keys.add(node.key)
        else:
            self._expanded_keys.discard(node.key)
        self.dataChanged.emit(self._index_for(node), self._index_for(node), [self.ExpandedRole])

    @Slot(int, object)
    def reparent_entity(self, entity: int, new_parent: int | None) -> None:
        """Send a non-optimistic reparent request to Rust."""
        if entity == new_parent:
            return
        if self._interaction is not None:
            self._interaction.reparent_entity(entity, new_parent)

    def update_snapshot(self, snapshot: Any) -> None:
        """Replace hierarchy structure while retaining expansion keys."""
        payload = _payload(snapshot)
        rows = payload.get("hierarchy", []) if isinstance(payload, dict) else []
        self.beginResetModel()
        self._root = HierarchyNode({}, None)
        self._nodes = {}
        for row in rows:
            if isinstance(row, dict) and isinstance(row.get("entity"), int):
                self._nodes[row["entity"]] = HierarchyNode(dict(row), None)
        for node in self._nodes.values():
            parent = self._nodes.get(node.row.get("parent"))
            node.parent = parent or self._root
            node.parent.children.append(node)
        self._sort_children(self._root)
        self._selection = set(payload.get("selection", [])) if isinstance(payload, dict) else set()
        self.endResetModel()

    def update_selection(self, selection: Any) -> None:
        """Apply authoritative selection roles without changing hierarchy."""
        payload = _payload(selection)
        self._selection = set(payload.get("entities", [])) if isinstance(payload, dict) else set()
        for node in self._nodes.values():
            index = self._index_for(node)
            self.dataChanged.emit(index, index, [self.SelectedRole])

    def _node(self, index: QModelIndex) -> HierarchyNode:
        return index.internalPointer() if index.isValid() else self._root

    def _depth(self, node: HierarchyNode) -> int:
        depth = 0
        while node.parent is not None and node.parent is not self._root:
            depth += 1
            node = node.parent
        return depth

    def _index_for(self, node: HierarchyNode) -> QModelIndex:
        if node.parent is None or node.parent is self._root:
            return self.createIndex(self._root.children.index(node), 0, node)
        return self.createIndex(node.parent.children.index(node), 0, node)

    @staticmethod
    def _sort_children(node: HierarchyNode) -> None:
        node.children.sort(key=lambda child: child.row.get("entity", 0))
        for child in node.children:
            HierarchyModel._sort_children(child)


@dataclass
class InspectorNode:
    """Component or reflected field exposed by the inspector tree."""

    row: dict[str, Any]
    parent: "InspectorNode | None"
    children: list["InspectorNode"] = field(default_factory=list)


class InspectorModel(QAbstractItemModel):
    """Recursive reflection tree with common-value and mixed-value roles."""

    NodeKindRole = Qt.ItemDataRole.UserRole + 1
    LabelRole = Qt.ItemDataRole.UserRole + 2
    TypeIdRole = Qt.ItemDataRole.UserRole + 3
    TypePathRole = Qt.ItemDataRole.UserRole + 4
    FieldPathRole = Qt.ItemDataRole.UserRole + 5
    FieldIdRole = Qt.ItemDataRole.UserRole + 6
    ValueRole = Qt.ItemDataRole.UserRole + 7
    DisplayValueRole = Qt.ItemDataRole.UserRole + 8
    EditorKindRole = Qt.ItemDataRole.UserRole + 9
    EditableRole = Qt.ItemDataRole.UserRole + 10
    MixedRole = Qt.ItemDataRole.UserRole + 11
    ErrorRole = Qt.ItemDataRole.UserRole + 12
    EntityIdsRole = Qt.ItemDataRole.UserRole + 13
    ExpandedRole = Qt.ItemDataRole.UserRole + 14

    def __init__(self, interaction: Any = None, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._interaction = interaction
        self._root = InspectorNode({}, None)
        self._last_snapshot: dict[str, Any] = {}
        self._expanded: set[str] = set()
        self._conflict_field = ""

    def roleNames(self) -> dict[int, bytes]:
        return {
            self.NodeKindRole: b"nodeKind", self.LabelRole: b"label", self.TypeIdRole: b"typeId",
            self.TypePathRole: b"typePath", self.FieldPathRole: b"fieldPath", self.FieldIdRole: b"fieldId",
            self.ValueRole: b"value", self.DisplayValueRole: b"displayValue", self.EditorKindRole: b"editorKind",
            self.EditableRole: b"editable", self.MixedRole: b"mixed", self.ErrorRole: b"error",
            self.EntityIdsRole: b"entityIds", self.ExpandedRole: b"expanded",
        }

    def rowCount(self, parent: QModelIndex = QModelIndex()) -> int:
        return len(self._node(parent).children)

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
        node: InspectorNode = index.internalPointer()
        parent = node.parent
        if parent is None or parent is self._root:
            return QModelIndex()
        return self.createIndex(parent.parent.children.index(parent), 0, parent)

    def data(self, index: QModelIndex, role: int = Qt.ItemDataRole.DisplayRole) -> Any:
        if not index.isValid():
            return None
        row = index.internalPointer().row
        if role == Qt.ItemDataRole.DisplayRole:
            return row.get("displayValue", row.get("label", ""))
        role_keys = {
            self.NodeKindRole: "nodeKind", self.LabelRole: "label", self.TypeIdRole: "typeId",
            self.TypePathRole: "typePath", self.FieldPathRole: "fieldPath", self.FieldIdRole: "fieldId",
            self.ValueRole: "value", self.DisplayValueRole: "displayValue", self.EditorKindRole: "editorKind",
            self.EditableRole: "editable", self.MixedRole: "mixed", self.ErrorRole: "error",
            self.EntityIdsRole: "entityIds", self.ExpandedRole: "expanded",
        }
        return row.get(role_keys.get(role))

    @Slot(str, object)
    def edit_field(self, field_id: str, value: Any) -> None:
        """Forward a live field value to the revisioned interaction controller."""
        node = self._find_field(field_id, self._root)
        if node is None or not node.row.get("editable") or self._interaction is None:
            return
        self._interaction.edit_field(node.row.get("typePath", ""), node.row.get("fieldPath", ""), field_id, value)

    @Slot(str, bool)
    def set_expanded(self, field_id: str, expanded: bool) -> None:
        """Persist local expansion state for reflected groups."""
        node = self._find_field(field_id, self._root)
        if node is None:
            return
        if expanded:
            self._expanded.add(field_id)
        else:
            self._expanded.discard(field_id)
        index = self._index_for(node)
        self.dataChanged.emit(index, index, [self.ExpandedRole])

    def update_snapshot(self, snapshot: Any) -> None:
        """Build a recursive inspector from a copied immutable snapshot."""
        payload = copy.deepcopy(_payload(snapshot))
        self._last_snapshot = payload if isinstance(payload, dict) else {}
        selected = [entity for entity in self._last_snapshot.get("selection", []) if isinstance(entity, int)]
        entities = {row.get("entity"): row for row in self._last_snapshot.get("entities", []) if isinstance(row, dict)}
        catalog = {row.get("type_path"): row for row in self._last_snapshot.get("component_catalog", []) if isinstance(row, dict)}
        common = self._common_components([entities.get(entity, {}) for entity in selected])
        self.beginResetModel()
        self._root = InspectorNode({}, None)
        for type_path, components in common.items():
            descriptor = catalog.get(type_path, {})
            values = [component.get("value") for component in components]
            component_node = InspectorNode(self._node_row("component", descriptor.get("short_name", type_path), type_path, type_path, values, selected, descriptor.get("editable", False), type_id=descriptor.get("type_id")), self._root)
            self._root.children.append(component_node)
            self._append_fields(component_node, descriptor.get("fields", []), values, type_path, "", selected, bool(descriptor.get("editable", False)), descriptor.get("type_id"))
        self.endResetModel()

    def update_selection(self, selection: Any) -> None:
        """Rebuild against the last snapshot with the authoritative selection."""
        payload = _payload(selection)
        if not isinstance(payload, dict):
            return
        snapshot = copy.deepcopy(self._last_snapshot)
        snapshot["selection"] = payload.get("entities", [])
        self.update_snapshot(snapshot)

    def set_conflict(self, field_id: str) -> None:
        """Mark one field as conflicted until a fresh snapshot arrives."""
        self._conflict_field = field_id
        if not field_id:
            for node in self._walk(self._root):
                node.row["error"] = ""
        node = self._find_field(field_id, self._root) if field_id else None
        if node is not None:
            node.row["error"] = "State changed in engine; refresh required"
            node.row["editable"] = False
        index = self._index_for(node) if node is not None else QModelIndex()
        if index.isValid():
            self.dataChanged.emit(index, index, [self.ErrorRole, self.EditableRole])

    def _walk(self, node: InspectorNode) -> list[InspectorNode]:
        result: list[InspectorNode] = []
        for child in node.children:
            result.append(child)
            result.extend(self._walk(child))
        return result

    def _append_fields(self, parent: InspectorNode, fields: list[dict[str, Any]], values: list[Any], type_path: str, prefix: str, entities: list[int], editable: bool, type_id: str | None) -> None:
        for field in fields:
            path = field.get("field_path", "")
            relative = path[len(prefix) + 1:] if prefix and path.startswith(prefix + ".") else path
            if "." in relative:
                continue
            field_values = [_lookup(value, path) for value in values]
            field_id = field.get("field_id", f"{type_path}::{path}")
            child = InspectorNode(self._node_row("field", field.get("name", relative), type_path, path, field_values, entities, editable, type_id=type_id, value_type_path=field.get("type_path", type_path)), parent)
            child.row["fieldId"] = field_id
            parent.children.append(child)
            self._append_fields(child, field.get("fields", []), values, type_path, path, entities, editable, type_id)

    @staticmethod
    def _common_components(entity_rows: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
        if not entity_rows:
            return {}
        by_entity = [{component.get("type_path"): component for component in row.get("components", []) if isinstance(component, dict)} for row in entity_rows]
        common_paths = set(by_entity[0])
        for components in by_entity[1:]:
            common_paths.intersection_update(components)
        return {path: [components[path] for components in by_entity] for path in sorted(common_paths)}

    @staticmethod
    def _node_row(kind: str, label: str, type_path: str, field_path: str, values: list[Any], entities: list[int], editable: bool, type_id: str | None = None, value_type_path: str | None = None) -> dict[str, Any]:
        mixed = any(value != values[0] for value in values[1:]) if values else False
        value = None if mixed or not values else values[0]
        editor_kind = _editor_kind(value_type_path or type_path, field_path, value, kind)
        return {
            "nodeKind": kind, "label": label, "typeId": type_id or type_path, "typePath": type_path,
            "fieldPath": field_path, "fieldId": f"{type_path}::{field_path}" if field_path else type_path,
            "value": value, "displayValue": "Multiple Values" if mixed else _display(value),
            "editorKind": editor_kind, "editable": editable and editor_kind not in {"group", "handle", "json"},
            "mixed": mixed, "error": "", "entityIds": entities,
        }

    def _node(self, index: QModelIndex) -> InspectorNode:
        return index.internalPointer() if index.isValid() else self._root

    def _index_for(self, node: InspectorNode) -> QModelIndex:
        if node.parent is None or node.parent is self._root:
            return self.createIndex(self._root.children.index(node), 0, node)
        return self.createIndex(node.parent.children.index(node), 0, node)

    def _find_field(self, field_id: str, node: InspectorNode) -> InspectorNode | None:
        for child in node.children:
            if child.row.get("fieldId") == field_id:
                return child
            found = self._find_field(field_id, child)
            if found is not None:
                return found
        return None


def _lookup(value: Any, path: str) -> Any:
    """Read a dotted reflected field path from JSON-compatible data."""
    current = value
    if isinstance(current, dict) and len(current) == 1 and path.split(".")[0] not in current:
        wrapped = next(iter(current.values()))
        if isinstance(wrapped, dict):
            current = wrapped
    for part in path.split(".") if path else []:
        if not isinstance(current, dict) or part not in current:
            return None
        current = current[part]
    return current


def _editor_kind(type_path: str, field_path: str, value: Any, node_kind: str) -> str:
    if node_kind == "component":
        return "group" if isinstance(value, dict) else _editor_kind(type_path, field_path, value, "field")
    if "Handle" in type_path:
        return "handle"
    if "Quat" in type_path or field_path.lower().endswith("rotation"):
        return "quaternion"
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list) and len(value) in {2, 3, 4} and all(isinstance(item, (int, float)) for item in value):
        return "vector"
    if isinstance(value, dict):
        return "group"
    return "json"


def _display(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, (dict, list)):
        return json.dumps(value, separators=(",", ":"))
    return str(value)


class AssetModel(SnapshotListModel):
    """Asset tree rows from the authoritative AssetDb snapshot."""

    AssetIdRole = Qt.ItemDataRole.UserRole + 1
    PathRole = Qt.ItemDataRole.UserRole + 2
    NameRole = Qt.ItemDataRole.UserRole + 3
    KindRole = Qt.ItemDataRole.UserRole + 4
    SelectedRole = Qt.ItemDataRole.UserRole + 5
    selectionChanged = Signal()

    def __init__(self, interaction: Any = None, session: Any = None, parent: QObject | None = None) -> None:
        super().__init__({"assetId": self.AssetIdRole, "path": self.PathRole, "name": self.NameRole, "kind": self.KindRole, "selected": self.SelectedRole}, parent)
        self._interaction = interaction
        self._session = session
        self._selected_asset_id = ""

    @Property(str, notify=selectionChanged)
    def selectedAssetId(self) -> str:
        return self._selected_asset_id

    def update_snapshot(self, snapshot: Any) -> None:
        payload = _payload(snapshot)
        rows = []
        for row in payload.get("nodes", []) if isinstance(payload, dict) else []:
            if not isinstance(row, dict):
                continue
            path = str(row.get("path", ""))
            name = PurePosixPath(path).name
            rows.append({"display": path, "assetId": row.get("asset_id", ""), "path": path, "name": name, "kind": PurePosixPath(path).suffix.lstrip("."), "selected": row.get("asset_id") == self._selected_asset_id})
        self._replace(rows)

    @Slot(str)
    def select_asset(self, asset_id: str) -> None:
        if asset_id == self._selected_asset_id:
            return
        self._selected_asset_id = asset_id
        for row in self._rows:
            row["selected"] = row.get("assetId") == asset_id
        if self._rows:
            self.dataChanged.emit(self.index(0, 0), self.index(len(self._rows) - 1, 0), [self.SelectedRole])
        self.selectionChanged.emit()

    @Slot(str)
    def activate_asset(self, asset_id: str) -> None:
        if self._session is None:
            return
        payload: dict[str, Any] = {"asset_id": asset_id}
        if self._interaction is not None and getattr(self._interaction, "revision", 0):
            payload["expected_revision"] = self._interaction.revision
        self._session.request("activate_asset", payload)


class AssetGraphModel(SnapshotListModel):
    """Disposable graph nodes with deterministic presentation coordinates."""

    AssetIdRole = Qt.ItemDataRole.UserRole + 1
    LabelRole = Qt.ItemDataRole.UserRole + 2
    XRole = Qt.ItemDataRole.UserRole + 3
    YRole = Qt.ItemDataRole.UserRole + 4
    HighlightedRole = Qt.ItemDataRole.UserRole + 5
    selectedChanged = Signal()

    def __init__(self, parent: QObject | None = None) -> None:
        super().__init__({"assetId": self.AssetIdRole, "label": self.LabelRole, "x": self.XRole, "y": self.YRole, "highlighted": self.HighlightedRole}, parent)
        self._edges: list[tuple[str, str]] = []
        self._selected = ""

    @Property(str, notify=selectedChanged)
    def selectedAssetId(self) -> str:
        return self._selected

    @Property("QVariantList", notify=selectedChanged)
    def edges(self) -> list[dict[str, str]]:
        return [{"parent": parent, "child": child} for parent, child in self._edges]

    @Property("QVariantList", notify=selectedChanged)
    def edgeLines(self) -> list[dict[str, Any]]:
        positions = {str(row.get("assetId")): (int(row.get("x", 0)) + 72, int(row.get("y", 0)) + 21) for row in self._rows}
        return [{"x1": positions[parent][0], "y1": positions[parent][1], "x2": positions[child][0], "y2": positions[child][1], "highlighted": bool(self._selected) and (parent == self._selected or child == self._selected)} for parent, child in self._edges if parent in positions and child in positions]

    def update_snapshot(self, snapshot: Any) -> None:
        payload = _payload(snapshot)
        nodes = sorted((row for row in payload.get("nodes", []) if isinstance(row, dict)), key=lambda row: str(row.get("asset_id", ""))) if isinstance(payload, dict) else []
        self._edges = [(str(edge.get("parent")), str(edge.get("child"))) for edge in payload.get("edges", []) if isinstance(edge, dict)] if isinstance(payload, dict) else []
        self._replace([{"display": row.get("path", ""), "assetId": row.get("asset_id", ""), "label": PurePosixPath(str(row.get("path", ""))).name, "x": (index % 5) * 160, "y": (index // 5) * 96, "highlighted": self._highlighted(str(row.get("asset_id", "")))} for index, row in enumerate(nodes)])

    @Slot(str)
    def select_asset(self, asset_id: str) -> None:
        self._selected = asset_id
        for row in self._rows:
            row["highlighted"] = self._highlighted(str(row.get("assetId", "")))
        if self._rows:
            self.dataChanged.emit(self.index(0, 0), self.index(len(self._rows) - 1, 0), [self.HighlightedRole])
        self.selectedChanged.emit()

    def _highlighted(self, asset_id: str) -> bool:
        return bool(self._selected) and (asset_id == self._selected or any(parent == self._selected and child == asset_id or child == self._selected and parent == asset_id for parent, child in self._edges))


class ConsoleModel(SnapshotListModel):
    """Bounded tracing lines from the Rust console service."""

    SequenceRole = Qt.ItemDataRole.UserRole + 1
    LevelRole = Qt.ItemDataRole.UserRole + 2
    TargetRole = Qt.ItemDataRole.UserRole + 3
    MessageRole = Qt.ItemDataRole.UserRole + 4
    FileRole = Qt.ItemDataRole.UserRole + 5
    LineRole = Qt.ItemDataRole.UserRole + 6
    filterChanged = Signal()

    def __init__(self, session: Any = None, parent: QObject | None = None) -> None:
        super().__init__({"sequence": self.SequenceRole, "level": self.LevelRole, "target": self.TargetRole, "message": self.MessageRole, "file": self.FileRole, "line": self.LineRole}, parent)
        self._session = session
        self._min_level = ""
        self._target_prefix = ""

    @Property(str, notify=filterChanged)
    def minLevel(self) -> str:
        return self._min_level

    @Property(str, notify=filterChanged)
    def targetPrefix(self) -> str:
        return self._target_prefix

    @Slot(str, str)
    def set_filter(self, min_level: str, target_prefix: str) -> None:
        self._min_level = min_level
        self._target_prefix = target_prefix
        self.filterChanged.emit()
        self.refresh()

    @Slot()
    def refresh(self) -> None:
        if self._session is not None:
            payload = {}
            if self._min_level:
                payload["min_level"] = self._min_level
            if self._target_prefix:
                payload["target_prefix"] = self._target_prefix
            self._session.request("request_console_snapshot", payload)

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
    seriesChanged = Signal()

    def __init__(self, parent: QObject | None = None) -> None:
        super().__init__({"frameId": self.FrameRole, "fps": self.FpsRole, "frameTimeMs": self.FrameTimeRole, "gpuTimeMs": self.GpuTimeRole, "drawCalls": self.DrawCallsRole, "instanceCount": self.InstanceCountRole, "memory": self.MemoryRole, "passes": self.PassesRole}, parent)

    @Property("QVariantList", notify=seriesChanged)
    def frameTimes(self) -> list[float]:
        return [float(row.get("frame_time_ms", 0.0)) for row in self._rows]

    @Property("QVariantMap", notify=seriesChanged)
    def series(self) -> dict[str, list[float]]:
        return {
            "frame": [float(row.get("frame_time_ms", 0.0)) for row in self._rows],
            "gpu": [float(row.get("gpu_time_ms", 0.0)) for row in self._rows],
            "draw": [float(row.get("draw_calls", 0.0)) for row in self._rows],
            "instances": [float(row.get("instance_count", 0.0)) for row in self._rows],
            "memory": [float(row.get("process_resident_bytes") or row.get("asset_cache_bytes", 0)) / 1048576.0 for row in self._rows],
        }

    def update_snapshot(self, snapshot: Any) -> None:
        payload = _payload(snapshot)
        rows = []
        for row in payload.get("samples", []) if isinstance(payload, dict) else []:
            if isinstance(row, dict):
                rows.append({**row, "display": f"Frame {row.get('frame_id', '')}: {row.get('frame_time_ms', 0):.2f} ms"})
        self._replace(rows)
        self.seriesChanged.emit()


class AssetPreviewModel(QObject):
    """Presentation-only state for the latest session-owned mesh preview."""

    changed = Signal()

    def __init__(self, session: Any, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._path = ""
        self._state = "idle"
        self._error = ""
        session.assetPreviewReady.connect(self._ready)
        session.assetPreviewCancelled.connect(self._cancelled)
        session.engineError.connect(self._error_received)

    @Property(str, notify=changed)
    def source(self) -> str:
        return self._path

    @Property(str, notify=changed)
    def state(self) -> str:
        return self._state

    @Property(str, notify=changed)
    def error(self) -> str:
        return self._error

    def _ready(self, envelope: Any) -> None:
        payload = _payload(envelope)
        relative = str(payload.get("path") or "") if isinstance(payload, dict) else ""
        root = os.environ.get("HYGE_PROJECT", ".")
        candidate = os.path.realpath(os.path.join(root, relative))
        preview_root = os.path.realpath(os.path.join(root, ".hyge", "previews"))
        if relative and candidate.startswith(preview_root + os.sep) and os.path.isfile(candidate):
            self._path = "file:///" + candidate.replace("\\", "/")
            self._state, self._error = "ready", ""
        else:
            self._state, self._error = "failed", "preview output was unavailable"
        self.changed.emit()

    def _cancelled(self, _envelope: Any) -> None:
        self._state = "cancelled"
        self.changed.emit()

    def _error_received(self, envelope: Any) -> None:
        error = getattr(envelope, "error", None) or {}
        if error.get("code") in {"preview_failed", "asset_activation_failed"}:
            self._state, self._error = "failed", error.get("message", "preview failed")
            self.changed.emit()
