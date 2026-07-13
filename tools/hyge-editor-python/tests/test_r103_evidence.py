"""R-103 evidence plumbing without replacing the production session path."""

import json

from hyge_editor.session import EditorSession
from hyge_editor.interaction import EditorInteractionController
from hyge_editor.ipc import Envelope


def test_protocol_trace_is_jsonl_and_does_not_store_session_token(tmp_path, monkeypatch):
    trace = tmp_path / "protocol.jsonl"
    monkeypatch.setenv("HYGE_EDITOR_PROTOCOL_TRACE", str(trace))
    session = EditorSession("127.0.0.1:1", "secret-token")
    session.enable_trace(str(trace))
    session._record_trace("out", Envelope("request-1", "open_scene", {"path": "main.hyge-world"}))
    payload = json.loads(trace.read_text(encoding="utf-8"))
    assert payload["direction"] == "out"
    assert payload["message_type"] == "open_scene"
    assert payload["message_id"] == "request-1"
    assert "secret-token" not in trace.read_text(encoding="utf-8")

    session._record_trace(
        "out",
        Envelope("hello-1", "hello", {"session_token": "secret-token", "nested": {"authorization": "bearer-secret"}}),
    )
    contents = trace.read_text(encoding="utf-8")
    assert "secret-token" not in contents
    assert "bearer-secret" not in contents
    assert "[REDACTED]" in contents


def test_selection_event_advances_authoritative_interaction_revision():
    session = EditorSession("127.0.0.1:1", "token")
    interaction = EditorInteractionController(session)
    session.selectionChanged.emit(
        Envelope(
            "selection",
            "selection_changed",
            {"revision": 7, "scene_revision": 1, "entities": [42], "scene_ids": ["cube"]},
        )
    )
    assert interaction.revision == 7
