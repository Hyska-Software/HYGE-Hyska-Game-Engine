from PySide6.QtWidgets import QApplication

from hyge_editor.models import AssetModel, ConsoleModel, HierarchyModel, InspectorModel, ProfilerModel


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
