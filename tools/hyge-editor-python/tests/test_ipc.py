import json
import socket
import threading
from pathlib import Path

import pytest

from hyge_editor.ipc import EditorClient, Envelope


def test_envelope_round_trip():
    envelope = Envelope("1", "hello", {"session_token": "test"})
    decoded = Envelope.from_bytes(envelope.to_bytes()[4:])
    assert decoded.message_id == "1"
    assert decoded.message_type == "hello"
    assert decoded.payload["session_token"] == "test"


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
    assert schema["properties"]["protocol_version"]["const"] == 1
    assert "hello" in schema["properties"]["message_type"]["enum"]
    assert "server_shutdown" in schema["properties"]["message_type"]["enum"]
    assert schema["properties"]["payload"]["type"] == "object"


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
                response = Envelope(
                    request["message_id"], "hello_ack", {"protocol_version": 1}
                ).to_bytes()
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
