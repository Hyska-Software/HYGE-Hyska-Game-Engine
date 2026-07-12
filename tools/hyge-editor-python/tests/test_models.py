from PySide6.QtCore import QObject, Signal
from PySide6.QtWidgets import QApplication

from hyge_editor.interaction import EditorInteractionController
from hyge_editor.models import AssetGraphModel, AssetModel, ConsoleModel, HierarchyModel, InspectorModel, ProfilerModel


def app():
    return QApplication.instance() or QApplication([])


def test_models_replace_snapshot_data_without_backend_ownership():
    app()
    snapshot = {
        "revision": 1,
        "selection": [2],
        "hierarchy": [{"entity": 1, "name": "Root", "parent": None, "children": [2]}, {"entity": 2, "name": "Child", "parent": 1, "children": []}],
        "entities": [{"entity": 2, "components": [{"type_id": "t", "type_path": "Example", "value": {"x": 1}, "error": None}]}],
    }
    hierarchy = HierarchyModel()
    inspector = InspectorModel()
    hierarchy.update_snapshot(snapshot)
    inspector.update_snapshot(snapshot)
    snapshot["hierarchy"][0]["name"] = "Mutated backend object"
    assert hierarchy.rowCount() == 1
    assert hierarchy.data(hierarchy.index(0, 0), hierarchy.NameRole) == "Root"
    assert inspector.rowCount() == 1


def test_asset_console_profiler_models_handle_empty_and_diagnostics():
    app()
    assets = AssetModel()
    console = ConsoleModel()
    profiler = ProfilerModel()
    assets.update_snapshot({"nodes": [], "edges": [], "diagnostics": ["missing"]})
    console.update_snapshot({"lines": [], "filter": {}})
    profiler.update_snapshot({"samples": []})
    assert assets.rowCount() == console.rowCount() == profiler.rowCount() == 0


def test_operational_panel_models_project_fixture_snapshots_and_actions():
    app()

    class Session:
        def __init__(self): self.requests = []
        def request(self, kind, payload=None): self.requests.append((kind, payload or {}))

    class Interaction:
        revision = 9

    session = Session()
    assets = AssetModel(Interaction(), session)
    graph = AssetGraphModel()
    console = ConsoleModel(session)
    profiler = ProfilerModel()
    first, second = "a" * 64, "b" * 64
    snapshot = {"nodes": [{"asset_id": first, "path": "assets/demo.hyge-world"}, {"asset_id": second, "path": "assets/mesh.hyge-mesh"}], "edges": [{"parent": first, "child": second}], "diagnostics": []}
    assets.update_snapshot(snapshot)
    graph.update_snapshot(snapshot)
    assets.select_asset(second)
    assets.activate_asset(second)
    graph.select_asset(first)
    console.set_filter("warn", "hyge")
    profiler.update_snapshot({"samples": [{"frame_id": 1, "frame_time_ms": 12.5, "fps": 80.0, "gpu_time_ms": 3.0, "draw_calls": 4, "instance_count": 8, "process_resident_bytes": None, "asset_cache_bytes": 5, "passes": []}]})
    assert assets.rowCount() == graph.rowCount() == 2
    assert graph.data(graph.index(1, 0), graph.HighlightedRole) is True
    assert session.requests == [("activate_asset", {"asset_id": second, "expected_revision": 9}), ("request_console_snapshot", {"min_level": "warn", "target_prefix": "hyge"})]
    assert profiler.frameTimes == [12.5]


def test_hierarchy_preserves_expansion_and_delegates_authoritative_selection_and_drop():
    app()
    calls = []

    class Interaction:
        def select_entity(self, entity, shift): calls.append(("select", entity, shift))
        def reparent_entity(self, entity, parent): calls.append(("reparent", entity, parent))

    model = HierarchyModel(Interaction())
    snapshot = {"selection": [], "hierarchy": [{"entity": 1, "scene_id": "root", "name": "Root", "parent": None, "children": [2]}, {"entity": 2, "scene_id": "child", "name": "Child", "parent": 1, "children": []}]}
    model.update_snapshot(snapshot)
    model.set_expanded(1, True)
    model.select_entity(2, True)
    model.reparent_entity(2, 1)
    model.update_snapshot(snapshot)
    assert model.data(model.index(0, 0), model.ExpandedRole) is True
    assert calls == [("select", 2, True), ("reparent", 2, 1)]


def test_inspector_intersects_components_and_marks_mixed_values():
    app()
    snapshot = {
        "selection": [1, 2],
        "entities": [
            {"entity": 1, "components": [{"type_path": "Example", "type_id": "example", "value": {"value": 1}, "error": None}]},
            {"entity": 2, "components": [{"type_path": "Example", "type_id": "example", "value": {"value": 2}, "error": None}]},
        ],
        "component_catalog": [{"type_path": "Example", "short_name": "Example", "editable": True, "fields": [{"field_path": "value", "field_id": "example-value", "name": "Value", "type_path": "i32", "fields": []}]}],
    }
    model = InspectorModel()
    model.update_snapshot(snapshot)
    component = model.index(0, 0)
    field = model.index(0, 0, component)
    assert model.rowCount() == 1
    assert model.data(field, model.FieldIdRole) == "example-value"
    assert model.data(field, model.MixedRole) is True
    assert model.data(field, model.DisplayValueRole) == "Multiple Values"


def test_interaction_conflict_refreshes_and_serializes_live_batch_edits():
    app()

    class FakeSession(QObject):
        worldSnapshot = Signal(object)
        selectionChanged = Signal(object)
        commandCompleted = Signal(object)
        engineError = Signal(object)

        def __init__(self):
            super().__init__()
            self.requests = []

        def request(self, kind, payload=None):
            self.requests.append((kind, payload or {}))

    session = FakeSession()
    controller = EditorInteractionController(session)
    session.worldSnapshot.emit({"revision": 7, "selection": [1, 2]})
    session.selectionChanged.emit({"entities": [1, 2]})
    controller.edit_field("Example", "value", "example-value", 9)
    assert session.requests[-1] == ("edit_components", {"expected_revision": 7, "entities": [1, 2], "type_path": "Example", "field_path": "value", "value": 9})
    session.engineError.emit(type("Envelope", (), {"error": {"code": "stale_revision", "message": "stale"}})())
    assert controller.hasConflict is True
    assert session.requests[-1][0] == "request_world_snapshot"
    session.worldSnapshot.emit({"revision": 8, "selection": [1, 2]})
    assert controller.hasConflict is False
