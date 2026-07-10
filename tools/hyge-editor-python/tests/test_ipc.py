import json
import socket
import threading

from hyge_editor.ipc import EditorClient, Envelope


def test_envelope_round_trip():
    envelope = Envelope("1", "hello", {"session_token": "test"})
    decoded = Envelope.from_bytes(envelope.to_bytes()[4:])
    assert decoded.message_id == "1"
    assert decoded.message_type == "hello"
    assert decoded.payload["session_token"] == "test"


def test_client_uses_length_prefixed_json():
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    address = f"127.0.0.1:{listener.getsockname()[1]}"

    def server():
        connection, _ = listener.accept()
        with connection:
            header = connection.recv(4)
            body = connection.recv(int.from_bytes(header, "big"))
            request = json.loads(body)
            response = Envelope(request["message_id"], "hello_ack", {"protocol_version": 1}).to_bytes()
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
