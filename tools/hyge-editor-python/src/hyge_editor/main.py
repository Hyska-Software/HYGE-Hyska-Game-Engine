"""PySide6/QML entry point for the Hyge editor frontend."""

from __future__ import annotations

import os
import sys
from pathlib import Path

from PySide6.QtCore import QObject, Property, QUrl, Signal, Slot
from PySide6.QtQml import QQmlApplicationEngine
from PySide6.QtWidgets import QApplication

from .ipc import EditorClient


class EditorBridge(QObject):
    """Small QML-facing adapter; engine state remains in Rust."""

    statusChanged = Signal()

    def __init__(self, client: EditorClient) -> None:
        super().__init__()
        self._client = client
        self._status = "Connecting..."

    @Property(str, notify=statusChanged)
    def status(self) -> str:
        return self._status

    @Slot()
    def open_project(self) -> None:
        project = os.environ.get("HYGE_PROJECT", ".")
        response = self._client.request("open_project", {"path": project})
        self._status = "Project connected" if response.error is None else response.error["message"]
        self.statusChanged.emit()


def main() -> int:
    """Start the Qt application and connect it to the Rust service."""
    address = os.environ.get("HYGE_EDITOR_ADDRESS", "127.0.0.1:3765")
    token = os.environ.get("HYGE_EDITOR_TOKEN", "hyge-local-dev")
    client = EditorClient(address, token)
    handshake = client.connect()
    if handshake.error is not None:
        raise RuntimeError(handshake.error["message"])

    app = QApplication(sys.argv)
    engine = QQmlApplicationEngine()
    bridge = EditorBridge(client)
    engine.rootContext().setContextProperty("editorBridge", bridge)
    engine.load(QUrl.fromLocalFile(str(Path(__file__).parents[2] / "qml" / "Main.qml")))
    if not engine.rootObjects():
        client.close()
        return 1
    result = app.exec()
    client.close()
    return result


if __name__ == "__main__":
    raise SystemExit(main())
