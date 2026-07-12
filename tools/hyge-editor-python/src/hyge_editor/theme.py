"""Qt palette and QML color bridge for the editor themes."""

from __future__ import annotations

from PySide6.QtCore import QObject, Property, Signal, Slot
from PySide6.QtGui import QColor, QPalette


PALETTES = {
    "dark": {
        "window": "#20242b", "surface": "#252b34", "surfaceAlt": "#15191f",
        "text": "#e6e9ef", "muted": "#aab2c0", "accent": "#79c0ff",
        "selected": "#315a78", "error": "#ed8b8b", "ok": "#9be7a5",
    },
    "light": {
        "window": "#eef1f5", "surface": "#ffffff", "surfaceAlt": "#dfe5ec",
        "text": "#20242b", "muted": "#5d6878", "accent": "#1769aa",
        "selected": "#b9d9f2", "error": "#b42318", "ok": "#18794e",
    },
}


class EditorTheme(QObject):
    """Exposes the active theme colors and applies a QApplication palette."""

    changed = Signal()

    def __init__(self, preferences: QObject, application: QObject, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._preferences = preferences
        self._application = application
        preferences.themeChanged.connect(self.apply)

    @Property(str, notify=changed)
    def name(self) -> str:
        """Return the active theme name."""
        return self._preferences.theme

    @Property(str, notify=changed)
    def window(self) -> str:
        """Return the window background color."""
        return self.color("window")

    @Property(str, notify=changed)
    def surface(self) -> str:
        """Return the primary surface color."""
        return self.color("surface")

    @Property(str, notify=changed)
    def surfaceAlt(self) -> str:
        """Return the alternate surface color."""
        return self.color("surfaceAlt")

    @Property(str, notify=changed)
    def text(self) -> str:
        """Return the primary text color."""
        return self.color("text")

    @Property(str, notify=changed)
    def muted(self) -> str:
        """Return the muted text color."""
        return self.color("muted")

    @Property(str, notify=changed)
    def accent(self) -> str:
        """Return the accent color."""
        return self.color("accent")

    @Property(str, notify=changed)
    def selected(self) -> str:
        """Return the selected-row color."""
        return self.color("selected")

    @Property(str, notify=changed)
    def error(self) -> str:
        """Return the error color."""
        return self.color("error")

    @Property(str, notify=changed)
    def ok(self) -> str:
        """Return the success color."""
        return self.color("ok")

    def color(self, key: str) -> str:
        """Return one color from the active palette."""
        return PALETTES.get(self.name, PALETTES["dark"]).get(key, "#000000")

    @Slot()
    def apply(self) -> None:
        """Apply the palette to Qt widgets and notify QML."""
        palette = QPalette()
        colors = PALETTES.get(self.name, PALETTES["dark"])
        palette.setColor(QPalette.ColorRole.Window, QColor(colors["window"]))
        palette.setColor(QPalette.ColorRole.Base, QColor(colors["surfaceAlt"]))
        palette.setColor(QPalette.ColorRole.Button, QColor(colors["surface"]))
        palette.setColor(QPalette.ColorRole.Text, QColor(colors["text"]))
        palette.setColor(QPalette.ColorRole.ButtonText, QColor(colors["text"]))
        palette.setColor(QPalette.ColorRole.Highlight, QColor(colors["selected"]))
        palette.setColor(QPalette.ColorRole.HighlightedText, QColor(colors["text"]))
        self._application.setPalette(palette)
        self.changed.emit()
