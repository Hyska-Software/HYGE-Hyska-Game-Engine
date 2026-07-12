"""R-100 persistence, restart and transient input evidence."""

from PySide6.QtCore import QObject, Signal
from PySide6.QtWidgets import QApplication

from hyge_editor.input_controller import ViewportInputController
from hyge_editor.interaction import EditorInteractionController
from hyge_editor.layout_state import DEFAULT_STATE, EditorPreferences, normalize_state
from hyge_editor.main import create_application

from fake_server import FakeEditorServer


def app():
    return QApplication.instance() or QApplication([])


def test_preferences_defaults_validate_corruption_and_round_trip(tmp_path):
    app()
    preferences = EditorPreferences(tmp_path)
    assert preferences.theme == "dark"
    assert preferences.mode == "editor"
    assert preferences.shortcut("save") == "Ctrl+S"

    preferences.toggle_theme()
    preferences.toggle_mode()
    preferences.set_window_size(300, 20)
    preferences.set_left_width(99999)
    preferences.save()

    restored = EditorPreferences(tmp_path)
    assert restored.theme == "light"
    assert restored.mode == "play"
    assert restored.windowWidth == 800
    assert restored.windowHeight == 600
    assert restored.leftWidth == 900

    preferences.path.write_text("{broken", encoding="utf-8")
    fallback = EditorPreferences(tmp_path)
    assert fallback.theme == DEFAULT_STATE["theme"]
    assert normalize_state({"version": 999}) == DEFAULT_STATE


def test_viewport_input_coalesces_and_releases_on_focus_loss():
    app()

    class Session(QObject):
        viewportTransportReady = Signal(object)
        viewportTransportReset = Signal(object)
        disconnected = Signal()
        engineError = Signal(object)

        def __init__(self):
            super().__init__()
            self.requests = []

        def request(self, kind, payload=None):
            self.requests.append((kind, payload or {}))

    class Viewport:
        generation = 7

    session = Session()
    controller = ViewportInputController(session, Viewport())
    controller.key("Key_W", True)
    controller.mouse(2, -1)
    controller.flush()
    assert session.requests == [("viewport_input", {
        "generation": 7,
        "expected_input_revision": 0,
        "input_revision": 1,
        "events": [
            {"kind": "key", "code": "Key_W", "pressed": True},
            {"kind": "mouse", "dx": 2.0, "dy": -1.0},
        ],
    })]
    controller.clear_transient()
    assert session.requests[-1][1]["expected_input_revision"] == 1
    assert session.requests[-1][1]["events"] == [{"kind": "key", "code": "Key_W", "pressed": False}]


def test_global_commands_use_authoritative_revision_and_serialize():
    app()

    class Session(QObject):
        worldSnapshot = Signal(object)
        selectionChanged = Signal(object)
        commandCompleted = Signal(object)
        engineError = Signal(object)

        def __init__(self):
            super().__init__()
            self.requests = []

        def request(self, kind, payload=None):
            self.requests.append((kind, payload or {}))

    session = Session()
    controller = EditorInteractionController(session)
    session.worldSnapshot.emit({"revision": 12, "selection": []})
    controller.save_scene()
    controller.undo()
    assert session.requests == [("save_scene", {})]
    session.commandCompleted.emit({"revision": 13})
    assert session.requests[-1] == ("undo", {"expected_revision": 13})


def test_qml_shell_restores_preferences_after_restart(tmp_path):
    app()
    server = FakeEditorServer()
    server.start()
    first = create_application([], None, tmp_path)
    first[2]._address = server.address
    preferences = first[1].rootContext().contextProperty("editorPreferences")
    preferences.toggle_theme()
    preferences.toggle_mode()
    preferences.set_left_width(420)
    preferences.save()
    first[2].close()
    first[3].close()

    second = create_application([], None, tmp_path)
    restored = second[1].rootContext().contextProperty("editorPreferences")
    assert restored.theme == "light"
    assert restored.mode == "play"
    assert restored.leftWidth == 420
    assert second[1].rootObjects()
    second[2].close()
    second[3].close()
    server.close()
