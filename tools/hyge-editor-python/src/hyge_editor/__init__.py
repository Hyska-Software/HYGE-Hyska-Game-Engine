"""PySide6 frontend support for the Hyge editor protocol."""

from .ipc import Envelope, EditorClient, PROTOCOL_VERSION

__all__ = ["Envelope", "EditorClient", "PROTOCOL_VERSION"]
