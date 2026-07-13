"""R-103 target-perspective workflow driver using production Qt controllers."""

from __future__ import annotations

import json
import hashlib
import os
import shutil
from pathlib import Path
from typing import Any, Callable

from PySide6.QtCore import QObject, QTimer
from PySide6.QtWidgets import QApplication

from .interaction import EditorInteractionController
from .session import EditorSession
from .viewport_item import ViewportController


class EvidenceWorkflow(QObject):
    """Drive the real editor workflow and retain machine-readable assertions."""

    def __init__(
        self,
        app: QApplication,
        session: EditorSession,
        interaction: EditorInteractionController,
        viewport: ViewportController,
        scene: Path,
        external_scene: Path,
        evidence_dir: Path,
        capture: Callable[[], None],
        parent: QObject | None = None,
    ) -> None:
        super().__init__(parent)
        self._app = app
        self._session = session
        self._interaction = interaction
        self._viewport = viewport
        self._scene = scene
        self._external_scene = external_scene
        self._evidence_dir = evidence_dir
        self._capture = capture
        self._phase = "waiting_scene"
        self._entity = 0
        self._scene_id = ""
        self._transform_path = ""
        self._field_id = ""
        self._expected_frame_revision = 0
        self._restored_scene_ids: list[str] = []
        self._poll = QTimer(self)
        self._poll.setInterval(100)
        self._poll.timeout.connect(lambda: session.request("request_world_snapshot"))
        session.worldSnapshot.connect(self._on_snapshot)
        session.selectionChanged.connect(self._on_selection)
        session.commandCompleted.connect(self._on_command)
        session.sceneReloaded.connect(self._on_reloaded)
        session.engineError.connect(self._on_error)
        viewport.frameReceived.connect(self._on_frame)
        QTimer.singleShot(20_000, lambda: self._fail("workflow timed out"))

    def _on_snapshot(self, envelope: Any) -> None:
        if self._phase == "waiting_reload_snapshot":
            entity = next(
                (item for item in envelope.payload.get("entities", [])
                 if item.get("scene_id") == self._scene_id),
                None,
            )
            transform = next(
                (item for item in (entity or {}).get("components", [])
                 if item.get("type_path") == self._transform_path),
                None,
            )
            translation = (
                (transform or {}).get("value", {})
                .get("hyge_scene::components::Transform", {})
                .get("translation")
            )
            if translation != [3.0, 1.0, 0.0]:
                self._fail(f"PersistOnReload Transform mismatch: {translation}")
                return
            self._complete_reload(translation)
            return
        if self._phase != "waiting_scene":
            return
        payload = envelope.payload
        node = next(
            (item for item in payload.get("hierarchy", []) if item.get("name") == "Persistent Cube"),
            None,
        )
        descriptor = next(
            (item for item in payload.get("component_catalog", []) if item.get("short_name") == "Transform"),
            None,
        )
        if not node or not descriptor:
            return
        field = next(
            (item for item in descriptor.get("fields", []) if item.get("field_path") == "translation"),
            None,
        )
        if not field:
            self._fail("Transform.translation is not reflected")
            return
        self._entity = int(node["entity"])
        self._scene_id = str(node.get("scene_id") or "")
        self._transform_path = str(descriptor["type_path"])
        self._field_id = str(field["field_id"])
        self._phase = "selecting"
        self._interaction.select_entity(self._entity)

    def _on_selection(self, envelope: Any) -> None:
        if self._phase != "selecting" or self._entity not in envelope.payload.get("entities", []):
            return
        self._phase = "waiting_initial_frame"

    def _on_frame(self, _frame_id: int) -> None:
        if self._phase == "waiting_initial_frame":
            before = self._evidence_dir / "viewport-before.png"
            if not self._viewport.save_png(str(before)):
                self._fail("initial viewport frame could not be captured")
                return
            self._phase = "editing"
            self._interaction.edit_field(
                self._transform_path,
                "translation",
                self._field_id,
                [3.0, 1.0, 0.0],
            )
            return
        if self._phase != "waiting_edited_frame":
            return
        if self._viewport.last_scene_revision < self._expected_frame_revision:
            return
        after = self._evidence_dir / "viewport-after.png"
        if not self._viewport.save_png(str(after)):
            self._fail("edited viewport frame could not be captured")
            return
        before_hash = hashlib.sha256((self._evidence_dir / "viewport-before.png").read_bytes()).digest()
        after_hash = hashlib.sha256(after.read_bytes()).digest()
        if before_hash == after_hash:
            self._fail("Transform edit did not change the viewport frame")
            return
        self._phase = "undoing"
        QTimer.singleShot(0, self._interaction.undo)

    def _on_command(self, envelope: Any) -> None:
        command = envelope.payload.get("command")
        transitions = {
            ("undoing", "undo"): ("redoing", self._interaction.redo),
            ("redoing", "redo"): ("saving", self._interaction.save_scene),
        }
        transition = transitions.get((self._phase, command))
        if transition:
            self._phase, action = transition
            QTimer.singleShot(0, action)
            return
        if self._phase == "editing" and command == "edit_components":
            self._expected_frame_revision = int(envelope.payload.get("revision", 0))
            self._phase = "waiting_edited_frame"
            return
        if self._phase == "saving" and command == "save_scene":
            shutil.copy2(self._scene, self._evidence_dir / "saved.hyge-world")
            temporary = self._scene.with_suffix(self._scene.suffix + ".r103-new")
            with self._external_scene.open("rb") as source, temporary.open("wb") as target:
                shutil.copyfileobj(source, target)
                target.flush()
                os.fsync(target.fileno())
            os.replace(temporary, self._scene)
            self._phase = "reloading"
            self._poll.start()

    def _on_reloaded(self, envelope: Any) -> None:
        if self._phase != "reloading":
            return
        restored = envelope.payload.get("restored_scene_ids", [])
        if self._scene_id not in restored:
            self._fail("PersistOnReload identity was not restored")
            return
        self._restored_scene_ids = list(restored)
        self._phase = "waiting_reload_snapshot"
        self._session.request("request_world_snapshot")

    def _complete_reload(self, translation: list[float]) -> None:
        self._poll.stop()
        self._phase = "complete"
        (self._evidence_dir / "workflow.json").write_text(
            json.dumps(
                {
                    "success": True,
                    "entity": self._entity,
                    "scene_id": self._scene_id,
                    "restored_scene_ids": self._restored_scene_ids,
                    "reload_translation": translation,
                    "commands": ["edit_components", "undo", "redo", "save_scene"],
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        QTimer.singleShot(750, self._finish)

    def _finish(self) -> None:
        self._capture()
        self._session.request("server_shutdown")
        QTimer.singleShot(100, self._app.quit)

    def _on_error(self, envelope: Any) -> None:
        error = envelope.error or {}
        self._fail(error.get("message", "engine error"))

    def _fail(self, message: str) -> None:
        if self._phase in {"complete", "failed"}:
            return
        self._phase = "failed"
        self._poll.stop()
        (self._evidence_dir / "workflow.json").write_text(
            json.dumps({"success": False, "error": message}, indent=2),
            encoding="utf-8",
        )
        self._session.request("server_shutdown")
        self._app.exit(2)
