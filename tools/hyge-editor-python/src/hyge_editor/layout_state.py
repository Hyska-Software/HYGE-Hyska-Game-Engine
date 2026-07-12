"""Persistent, validated preferences for the Qt editor shell."""

from __future__ import annotations

import json
import os
import tempfile
from copy import deepcopy
from pathlib import Path
from typing import Any

from PySide6.QtCore import QObject, Property, Signal, Slot, QTimer


DEFAULT_STATE: dict[str, Any] = {
    "version": 1,
    "theme": "dark",
    "window": {"width": 1440, "height": 900},
    "panels": {"left_width": 330, "right_width": 390, "bottom_height": 300},
    "panel_visibility": {
        "hierarchy": True,
        "inspector": True,
        "assets": True,
        "console": True,
        "profiler": True,
        "asset_graph": True,
    },
    "mode": "editor",
    "shortcuts": {
        "save": "Ctrl+S",
        "undo": "Ctrl+Z",
        "redo": "Ctrl+Shift+Z",
        "focus_selection": "F",
        "frame_selection": "Shift+F",
        "toggle_play_editor": "Space",
    },
}


def _bounded_int(value: Any, fallback: int, minimum: int, maximum: int) -> int:
    if isinstance(value, bool):
        return fallback
    try:
        value = int(value)
    except (TypeError, ValueError):
        return fallback
    return max(minimum, min(maximum, value))


def normalize_state(raw: Any) -> dict[str, Any]:
    """Return a safe state merged with defaults, never raising for user data."""
    state = deepcopy(DEFAULT_STATE)
    if not isinstance(raw, dict) or raw.get("version") != DEFAULT_STATE["version"]:
        return state
    if raw.get("theme") in {"dark", "light"}:
        state["theme"] = raw["theme"]
    if raw.get("mode") in {"editor", "play"}:
        state["mode"] = raw["mode"]
    window = raw.get("window", {})
    panels = raw.get("panels", {})
    state["window"] = {
        "width": _bounded_int(window.get("width"), 1440, 800, 8192),
        "height": _bounded_int(window.get("height"), 900, 600, 8192),
    }
    state["panels"] = {
        "left_width": _bounded_int(panels.get("left_width"), 330, 220, 900),
        "right_width": _bounded_int(panels.get("right_width"), 390, 260, 1000),
        "bottom_height": _bounded_int(panels.get("bottom_height"), 300, 120, 900),
    }
    visibility = raw.get("panel_visibility", {})
    if isinstance(visibility, dict):
        for name in state["panel_visibility"]:
            if isinstance(visibility.get(name), bool):
                state["panel_visibility"][name] = visibility[name]
    shortcuts = raw.get("shortcuts", {})
    if isinstance(shortcuts, dict):
        for name in state["shortcuts"]:
            value = shortcuts.get(name)
            if isinstance(value, str) and value.strip():
                state["shortcuts"][name] = value.strip()
    return state


class EditorPreferences(QObject):
    """Owns presentation preferences and persists them atomically."""

    changed = Signal()
    themeChanged = Signal()
    modeChanged = Signal()
    layoutChanged = Signal()

    def __init__(self, user_data: Path | None = None, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._path = (user_data or Path.cwd() / "user_data") / "editor_layout.json"
        self._state = deepcopy(DEFAULT_STATE)
        self._save_timer = QTimer(self)
        self._save_timer.setSingleShot(True)
        self._save_timer.setInterval(250)
        self._save_timer.timeout.connect(self.save)
        self.load()

    @property
    def path(self) -> Path:
        """Return the configured preferences file path."""
        return self._path

    @Property(str, notify=themeChanged)
    def theme(self) -> str:
        """Return ``dark`` or ``light``."""
        return self._state["theme"]

    @Property(str, notify=modeChanged)
    def mode(self) -> str:
        """Return the local presentation mode."""
        return self._state["mode"]

    @Property(int, notify=layoutChanged)
    def windowWidth(self) -> int:
        """Return persisted window width."""
        return self._state["window"]["width"]

    @Property(int, notify=layoutChanged)
    def windowHeight(self) -> int:
        """Return persisted window height."""
        return self._state["window"]["height"]

    @Property(int, notify=layoutChanged)
    def leftWidth(self) -> int:
        """Return persisted left panel width."""
        return self._state["panels"]["left_width"]

    @Property(int, notify=layoutChanged)
    def rightWidth(self) -> int:
        """Return persisted right panel width."""
        return self._state["panels"]["right_width"]

    @Property(int, notify=layoutChanged)
    def bottomHeight(self) -> int:
        """Return persisted bottom panel height."""
        return self._state["panels"]["bottom_height"]

    @Slot()
    def toggle_theme(self) -> None:
        """Toggle and persist the active theme."""
        self._state["theme"] = "light" if self.theme == "dark" else "dark"
        self.themeChanged.emit()
        self._changed()

    @Slot()
    def toggle_mode(self) -> None:
        """Toggle the local Editor/Play presentation mode."""
        self._state["mode"] = "play" if self.mode == "editor" else "editor"
        self.modeChanged.emit()
        self._changed()

    @Slot(int, int)
    def set_window_size(self, width: int, height: int) -> None:
        """Store a bounded window size."""
        self._state["window"] = {
            "width": _bounded_int(width, self.windowWidth, 800, 8192),
            "height": _bounded_int(height, self.windowHeight, 600, 8192),
        }
        self._changed(layout=True)

    @Slot(int)
    def set_left_width(self, width: int) -> None:
        """Store a bounded left split width."""
        self._state["panels"]["left_width"] = _bounded_int(width, self.leftWidth, 220, 900)
        self._changed(layout=True)

    @Slot(int)
    def set_right_width(self, width: int) -> None:
        """Store a bounded right split width."""
        self._state["panels"]["right_width"] = _bounded_int(width, self.rightWidth, 260, 1000)
        self._changed(layout=True)

    @Slot(str, bool)
    def set_panel_visible(self, name: str, visible: bool) -> None:
        """Set visibility for a known panel."""
        if name in self._state["panel_visibility"]:
            self._state["panel_visibility"][name] = bool(visible)
            self._changed(layout=True)

    @Slot(str, result=bool)
    def panel_visible(self, name: str) -> bool:
        """Return visibility for a known panel."""
        return bool(self._state["panel_visibility"].get(name, True))

    @Slot(str, result=str)
    def shortcut(self, name: str) -> str:
        """Return a persisted shortcut binding."""
        return str(self._state["shortcuts"].get(name, ""))

    def load(self) -> None:
        """Load preferences, falling back safely when the file is invalid."""
        try:
            self._state = normalize_state(json.loads(self._path.read_text(encoding="utf-8")))
        except (OSError, ValueError, TypeError, json.JSONDecodeError):
            self._state = deepcopy(DEFAULT_STATE)

    @Slot()
    def save(self) -> None:
        """Persist the current state through a same-directory atomic replace."""
        self._save_timer.stop()
        self._path.parent.mkdir(parents=True, exist_ok=True)
        payload = json.dumps(self._state, indent=2, sort_keys=True) + "\n"
        descriptor, temporary = tempfile.mkstemp(prefix=".editor_layout.", suffix=".tmp", dir=self._path.parent)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                stream.write(payload)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, self._path)
        finally:
            try:
                os.unlink(temporary)
            except FileNotFoundError:
                pass

    def _changed(self, layout: bool = False) -> None:
        self.changed.emit()
        if layout:
            self.layoutChanged.emit()
        self._save_timer.start()
