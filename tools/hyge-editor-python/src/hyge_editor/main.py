"""PySide6/QML entry point and composition root for the Hyge editor."""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import Any

from PySide6.QtCore import QObject, Property, QUrl, Signal, Slot
from PySide6.QtQml import QQmlApplicationEngine
from PySide6.QtWidgets import QApplication

from .models import AssetGraphModel, AssetModel, AssetPreviewModel, ConsoleModel, HierarchyModel, InspectorModel, ProfilerModel
from .interaction import EditorInteractionController
from .session import EditorSession
from .viewport_item import ViewportController
from .input_controller import ViewportInputController
from .layout_state import EditorPreferences
from .theme import EditorTheme


class EditorBridge(QObject):
    """Small QML-facing command and status facade."""

    statusChanged = Signal()
    droppedFramesChanged = Signal()
    retryChanged = Signal()

    def __init__(self, session: EditorSession, viewport: ViewportController, preferences: EditorPreferences, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._session = session
        self._viewport = viewport
        self._preferences = preferences
        self._status = "Disconnected"
        self._retry_attempt = 0
        self._retry_delay_ms = 0
        session.stateChanged.connect(self._set_status)
        session.protocolError.connect(self._set_error)
        session.reconnectScheduled.connect(self._set_retry)
        viewport.droppedFramesChanged.connect(self.droppedFramesChanged)

    @Property(str, notify=statusChanged)
    def status(self) -> str:
        """Return the current backend connection state."""
        return self._status

    @Property(int, notify=droppedFramesChanged)
    def droppedFrames(self) -> int:
        """Return the number of dropped viewport frames."""
        return self._viewport.dropped_frames

    @Property(int, notify=retryChanged)
    def retryAttempt(self) -> int:
        """Return the current bounded reconnect attempt."""
        return self._retry_attempt

    @Property(int, notify=retryChanged)
    def retryDelayMs(self) -> int:
        """Return the delay before the next reconnect attempt."""
        return self._retry_delay_ms

    @Slot()
    def connect_backend(self) -> None:
        """Connect the session without blocking the QML thread."""
        self._session.connect_async()

    @Slot()
    def open_project(self) -> None:
        """Open the project selected by ``HYGE_PROJECT``."""
        self._session.open_project(os.environ.get("HYGE_PROJECT", "."))

    @Slot()
    def close_backend(self) -> None:
        """Close the backend connection."""
        self._session.close()

    @Slot()
    def retry_backend(self) -> None:
        """Retry the backend connection immediately."""
        self._session.retry_now()

    @Property(str, notify=statusChanged)
    def mode(self) -> str:
        """Return the local editor mode label."""
        return self._preferences.mode

    def _set_status(self, state: str) -> None:
        self._status = state.capitalize()
        self.statusChanged.emit()

    def _set_retry(self, attempt: int, delay_ms: int) -> None:
        self._retry_attempt = attempt
        self._retry_delay_ms = delay_ms
        self.retryChanged.emit()

    def _set_error(self, message: str) -> None:
        self._status = f"Error: {message}"
        self.statusChanged.emit()


def create_application(
    argv: list[str] | None = None,
    session: EditorSession | None = None,
    user_data: Path | None = None,
) -> tuple[QApplication, QQmlApplicationEngine, EditorSession, ViewportController]:
    """Create a source-checkout shell suitable for production or tests."""
    app = QApplication.instance() or QApplication(argv if argv is not None else sys.argv)
    preferences = EditorPreferences(user_data or Path(os.environ.get("HYGE_USER_DATA", Path.cwd() / "user_data")))
    theme = EditorTheme(preferences, app)
    theme.apply()
    backend = session or EditorSession(
        os.environ.get("HYGE_EDITOR_ADDRESS", "127.0.0.1:3765"),
        os.environ.get("HYGE_EDITOR_TOKEN", "hyge-local-dev"),
    )
    viewport = ViewportController(backend)
    engine = QQmlApplicationEngine()
    engine.addImageProvider("hyge-viewport", viewport.provider)
    bridge = EditorBridge(backend, viewport, preferences, engine)
    interaction = EditorInteractionController(backend, engine)
    viewport_input = ViewportInputController(backend, viewport, engine)
    hierarchy = HierarchyModel(interaction, engine)
    inspector = InspectorModel(interaction, engine)
    assets = AssetModel(interaction, backend, engine)
    asset_graph = AssetGraphModel(engine)
    preview = AssetPreviewModel(backend, engine)
    console = ConsoleModel(backend, engine)
    profiler = ProfilerModel(engine)
    backend.worldSnapshot.connect(hierarchy.update_snapshot)
    backend.worldSnapshot.connect(inspector.update_snapshot)
    backend.selectionChanged.connect(hierarchy.update_selection)
    backend.selectionChanged.connect(inspector.update_selection)
    interaction.conflictFieldChanged.connect(inspector.set_conflict)
    backend.assetSnapshot.connect(assets.update_snapshot)
    backend.assetSnapshot.connect(asset_graph.update_snapshot)
    backend.consoleSnapshot.connect(console.update_snapshot)
    backend.profilerSnapshot.connect(profiler.update_snapshot)
    def prime_frontend(_handshake: Any) -> None:
        backend.open_project(os.environ.get("HYGE_PROJECT", "."))
        backend.request("request_asset_snapshot")
        backend.request("request_console_snapshot")
        backend.request("request_profiler_snapshot")
        backend.request("open_viewport_transport")

    backend.connected.connect(prime_frontend)
    root = engine.rootContext()
    root.setContextProperty("editorBridge", bridge)
    root.setContextProperty("editorPreferences", preferences)
    root.setContextProperty("editorTheme", theme)
    root.setContextProperty("editorInteraction", interaction)
    root.setContextProperty("hierarchyModel", hierarchy)
    root.setContextProperty("inspectorModel", inspector)
    root.setContextProperty("assetModel", assets)
    root.setContextProperty("assetGraphModel", asset_graph)
    root.setContextProperty("assetPreviewModel", preview)
    root.setContextProperty("consoleModel", console)
    root.setContextProperty("profilerModel", profiler)
    root.setContextProperty("viewportController", viewport)
    root.setContextProperty("viewportInput", viewport_input)
    qml_path = Path(__file__).parents[2] / "qml" / "Main.qml"
    engine.load(QUrl.fromLocalFile(str(qml_path)))
    app.aboutToQuit.connect(backend.close)
    app.aboutToQuit.connect(viewport.close)
    app.aboutToQuit.connect(viewport_input.clear_transient)
    app.aboutToQuit.connect(preferences.save)
    return app, engine, backend, viewport


def main() -> int:
    """Start the Qt application and connect it to the Rust service."""
    app, engine, session, _viewport = create_application()
    if not engine.rootObjects():
        session.close()
        return 1
    session.connect_async()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
