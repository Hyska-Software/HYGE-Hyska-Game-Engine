from PySide6.QtCore import QTimer

from hyge_editor.main import create_application

from fake_server import FakeEditorServer


def test_packaged_free_shell_connects_loads_qml_and_closes_cleanly(monkeypatch):
    monkeypatch.setenv("QT_QPA_PLATFORM", "offscreen")
    monkeypatch.setenv("QT_QUICK_BACKEND", "software")
    server = FakeEditorServer()
    server.start()
    app, engine, session, viewport = create_application([], None)
    # Replace the environment-selected endpoint before starting the session.
    session._address = server.address
    session.connect_async()
    QTimer.singleShot(750, app.quit)
    assert engine.rootObjects()
    assert app.exec() == 0
    session.close()
    viewport.close()
    server.close()
    assert "hello" in server.requests
    assert "open_project" in server.requests
    assert "request_asset_snapshot" in server.requests
    assert "request_console_snapshot" in server.requests
    assert "request_profiler_snapshot" in server.requests
    assert engine.rootContext().contextProperty("hierarchyModel").rowCount() == 1
    assert engine.rootContext().contextProperty("assetModel").rowCount() == 1
    assert engine.rootContext().contextProperty("consoleModel").rowCount() == 1
    assert engine.rootContext().contextProperty("profilerModel").rowCount() == 1
    assert not session._thread
