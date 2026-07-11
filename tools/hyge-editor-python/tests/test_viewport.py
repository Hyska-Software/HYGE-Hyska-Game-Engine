from PySide6.QtWidgets import QApplication

from hyge_editor.viewport_item import ViewportController
from hyge_editor.viewport_transport import ViewportFrame


class FakeReader:
    def __init__(self, _name, _size):
        self.dropped_frames = 0
        self.frames = {0: ViewportFrame(1, 2, 1, bytes([255, 0, 0, 255, 0, 255, 0, 255]))}
        self.closed = False

    def read_frame(self, slot, _stride):
        return self.frames.pop(slot, None)

    def close(self):
        self.closed = True


def test_viewport_consumes_fake_stable_frame_without_gpu():
    QApplication.instance() or QApplication([])
    controller = ViewportController(reader_factory=FakeReader)
    received = []
    controller.frameReceived.connect(received.append)
    controller.open_transport({"mapping_name": "fake", "generation": 1, "width": 2, "height": 1})
    controller.poll()
    assert received == [1]
    assert controller.state == "connected"
    controller._reader.dropped_frames = 2
    controller.poll()
    assert controller.dropped_frames == 2
    controller.reset_transport({"mapping_name": "fake-reconnect", "generation": 2, "width": 2, "height": 1})
    assert controller.state == "connected"
    assert controller._generation == 2
    controller.close()
    assert controller.state == "disconnected"
