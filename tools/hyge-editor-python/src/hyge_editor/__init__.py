"""PySide6 frontend support for the Hyge editor protocol."""

from .ipc import Envelope, EditorClient, PROTOCOL_VERSION
from .viewport_transport import ViewportTransport

__all__ = ["Envelope", "EditorClient", "PROTOCOL_VERSION", "ViewportTransport"]


def __getattr__(name: str):
    """Load Qt-dependent exports only when PySide6 is available."""
    if name in {"EditorSession", "EditorInteractionController", "HierarchyModel", "InspectorModel", "AssetModel", "ConsoleModel", "ProfilerModel", "ViewportController"}:
        from .models import AssetModel, ConsoleModel, HierarchyModel, InspectorModel, ProfilerModel
        from .session import EditorSession
        from .interaction import EditorInteractionController
        from .viewport_item import ViewportController
        return {
            "EditorSession": EditorSession,
            "EditorInteractionController": EditorInteractionController,
            "HierarchyModel": HierarchyModel,
            "InspectorModel": InspectorModel,
            "AssetModel": AssetModel,
            "ConsoleModel": ConsoleModel,
            "ProfilerModel": ProfilerModel,
            "ViewportController": ViewportController,
        }[name]
    raise AttributeError(name)
