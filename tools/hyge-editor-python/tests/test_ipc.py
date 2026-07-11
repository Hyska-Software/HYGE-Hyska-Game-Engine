import json
import socket
import threading
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

from hyge_editor.ipc import MESSAGE_TYPES, EditorClient, Envelope


def test_envelope_round_trip():
    envelope = Envelope("1", "hello", {"session_token": "test"})
    decoded = Envelope.from_bytes(envelope.to_bytes()[4:])
    assert decoded.message_id == "1"
    assert decoded.message_type == "hello"
    assert decoded.payload["session_token"] == "test"


def test_handshake_payload_round_trip_and_correlation_id():
    envelope = Envelope(
        "1",
        "hello",
        {
            "client_name": "test",
            "supported_protocol_versions": [1],
            "session_id": None,
            "session_token": "test",
        },
        correlation_id="parent",
    )
    decoded = Envelope.from_bytes(envelope.to_bytes()[4:])
    assert decoded.correlation_id == "parent"


def test_envelope_rejects_invalid_shared_contract_fields():
    with_invalid_id = Envelope("", "hello", {})
    try:
        with_invalid_id.to_bytes()
        assert False, "empty ids must fail"
    except ValueError:
        pass

    with pytest.raises(ValueError):
        Envelope("1", "not_a_message", {}).to_bytes()
    with pytest.raises(ValueError):
        Envelope("1", "hello", [],).to_bytes()


def test_schema_lists_the_version_and_wire_message_names():
    schema = json.loads(
        (Path(__file__).parents[3] / "protocol" / "editor.schema.json").read_text()
    )
    assert schema["properties"]["protocol_version"] == {"type": "integer", "minimum": 1}
    assert "hello" in schema["properties"]["message_type"]["enum"]
    assert "server_shutdown" in schema["properties"]["message_type"]["enum"]
    assert schema["properties"]["payload"]["type"] == "object"
    assert Draft202012Validator.check_schema(schema) is None


def test_schema_validates_handshake_ack_and_errors():
    schema = json.loads(
        (Path(__file__).parents[3] / "protocol" / "editor.schema.json").read_text()
    )
    validator = Draft202012Validator(schema)
    valid_ack = {
        "protocol_version": 1,
        "message_id": "ack",
        "message_type": "hello_ack",
        "payload": {
            "selected_protocol_version": 1,
            "session_id": "session",
            "resumed": False,
            "server": "hyge-editor",
            "request_timeout_ms": 5000,
        },
    }
    assert list(validator.iter_errors(valid_ack)) == []
    invalid_error = {
        "protocol_version": 1,
        "message_id": "error",
        "message_type": "engine_error",
        "payload": {},
    }
    assert list(validator.iter_errors(invalid_error))


def test_schema_validates_every_declared_message_type():
    schema = json.loads(
        (Path(__file__).parents[3] / "protocol" / "editor.schema.json").read_text()
    )
    validator = Draft202012Validator(schema)
    for message_type in MESSAGE_TYPES:
        payload = {}
        envelope = {
            "protocol_version": 1,
            "message_id": message_type,
            "message_type": message_type,
            "payload": payload,
        }
        if message_type == "hello":
            envelope["payload"] = {
                "client_name": "test",
                "supported_protocol_versions": [1],
                "session_id": None,
                "session_token": "test",
            }
        elif message_type == "hello_ack":
            envelope["payload"] = {
                "selected_protocol_version": 1,
                "session_id": "session",
                "resumed": False,
                "server": "hyge-editor",
                "request_timeout_ms": 5000,
            }
        elif message_type == "lifecycle_status":
            envelope["payload"] = {
                "session_id": "session",
                "state": "ready",
                "details": {},
            }
        elif message_type == "selection_changed":
            envelope["payload"] = {"revision": 1, "scene_revision": 0, "entities": []}
        elif message_type == "select_entities":
            envelope["payload"] = {"entities": []}
        elif message_type == "world_snapshot":
            envelope["payload"] = {
                "revision": 1,
                "scene_revision": 0,
                "hierarchy": [],
                "entities": [],
                "component_catalog": [],
                "selection": [],
                "diagnostics": [],
            }
        elif message_type == "engine_error":
            envelope["error"] = {"code": "test", "message": "test"}
        elif message_type in {"undo", "redo"}:
            envelope["payload"] = {"expected_revision": 1}
        elif message_type in {"duplicate_entity", "destroy_entity"}:
            envelope["payload"] = {"expected_revision": 1, "entity": 1}
        elif message_type == "reparent_entity":
            envelope["payload"] = {"expected_revision": 1, "entity": 1, "new_parent": None}
        elif message_type in {"add_component", "edit_component"}:
            envelope["payload"] = {
                "expected_revision": 1,
                "entity": 1,
                "type_path": "hyge_scene::components::Name",
                "value": {},
            }
        elif message_type == "remove_component":
            envelope["payload"] = {
                "expected_revision": 1,
                "entity": 1,
                "type_path": "hyge_scene::components::Name",
            }
        elif message_type == "instantiate_prefab":
            envelope["payload"] = {
                "expected_revision": 1,
                "prefab": {},
                "transform": {},
            }
        assert list(validator.iter_errors(envelope)) == [], message_type


def test_client_rejects_requests_before_connecting():
    client = EditorClient("127.0.0.1:1", "test")
    with pytest.raises(RuntimeError):
        client.request("open_project", {"path": "."})


def test_client_uses_length_prefixed_json():
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    address = f"127.0.0.1:{listener.getsockname()[1]}"

    def server():
        connection, _ = listener.accept()
        with connection:
            for _ in range(2):
                header = connection.recv(4)
                body = connection.recv(int.from_bytes(header, "big"))
                request = json.loads(body)
                if request["message_type"] == "hello":
                    payload = {
                        "selected_protocol_version": 1,
                        "session_id": "session",
                        "resumed": False,
                        "server": "hyge-editor",
                        "request_timeout_ms": 5000,
                    }
                else:
                    payload = {}
                response = Envelope(request["message_id"], "hello_ack", payload).to_bytes()
                connection.sendall(response)

    thread = threading.Thread(target=server)
    thread.start()
    client = EditorClient(address, "test")
    client.connect()
    response = client.request("open_project", {"path": "."})
    assert response.message_type == "hello_ack"
    client.close()
    thread.join(timeout=2)
    listener.close()


def test_client_rejects_oversized_and_truncated_responses():
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    address = f"127.0.0.1:{listener.getsockname()[1]}"

    def server():
        connection, _ = listener.accept()
        with connection:
            header = connection.recv(4)
            _ = connection.recv(int.from_bytes(header, "big"))
            connection.sendall((MAX_MESSAGE_BYTES + 1).to_bytes(4, "big"))

    from hyge_editor.ipc import MAX_MESSAGE_BYTES

    thread = threading.Thread(target=server)
    thread.start()
    client = EditorClient(address, "test")
    with pytest.raises(ValueError):
        client.connect()
    client.close()
    thread.join(timeout=2)
    listener.close()


def test_client_exposes_timeout_for_a_silent_server():
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    address = f"127.0.0.1:{listener.getsockname()[1]}"

    def server():
        connection, _ = listener.accept()
        with connection:
            connection.recv(4096)
            threading.Event().wait(0.2)

    thread = threading.Thread(target=server)
    thread.start()
    client = EditorClient(address, "test", timeout=0.05)
    with pytest.raises(TimeoutError):
        client.connect()
    client.close()
    thread.join(timeout=2)
    listener.close()
