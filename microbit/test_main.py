"""Logic tests for main.py's serial buffer and chunk framing.

Run with: python3 microbit/test_main.py

The micro:bit `microbit` and `audio` modules are stubbed so the pure parsing
logic (buffer rebind, line splitting, `C`/`Z`/`S` framing) runs under CPython.
Hardware behaviour still needs on-device verification.
"""

import importlib
import os
import sys
import types


class FakeUart:
    def __init__(self, script=b""):
        self.inbuf = bytearray(script)
        self.out = []

    def init(self, *a, **k):
        pass

    def read(self):
        if not self.inbuf:
            return None
        data = bytes(self.inbuf)
        self.inbuf = bytearray()
        return data

    def write(self, s):
        self.out.append(s)


def _fake_microbit(uart):
    m = types.ModuleType("microbit")
    m.uart = uart

    class Display:
        def show(self, *a, **k):
            pass

    class Image:
        MUSIC_QUAVER = "quaver"
        YES = "yes"
        ASLEEP = "asleep"

    class Button:
        def was_pressed(self):
            return False

    class Pin:
        PULL_UP = "pullup"

        def read_digital(self):
            return 1

        def read_analog(self):
            return 0

        def is_touched(self):
            return False

        def set_pull(self, *a):
            pass

    m.display = Display()
    m.Image = Image
    m.button_a = Button()
    m.button_b = Button()
    m.pin0 = m.pin1 = m.pin2 = m.pin_logo = Pin()
    m.set_volume = lambda *a, **k: None
    m.running_time = lambda: 0
    return m


def _fake_audio(captured):
    a = types.ModuleType("audio")

    class AudioFrame(list):
        def __init__(self):
            super().__init__([0] * 32)

    def play(src, wait=True, pin=None):
        for frame in src:  # wait=True drains the generator to completion
            captured.append(list(frame))

    a.AudioFrame = AudioFrame
    a.play = play
    a.stop = lambda *a, **k: None
    return a


def load_main(script=b""):
    uart = FakeUart(script)
    captured = []
    sys.modules["microbit"] = _fake_microbit(uart)
    sys.modules["audio"] = _fake_audio(captured)
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    sys.modules.pop("main", None)
    main = importlib.import_module("main")
    return main, uart, captured


def test_take_line_rebinds_without_del():
    main, _, _ = load_main()
    main.rx = bytearray(b"C 512\nABC")
    assert main.take_line() == "C 512"
    assert bytes(main.rx) == b"ABC"  # prefix consumed via rebind, tail intact


def test_take_line_tolerates_undecodable_bytes():
    main, _, _ = load_main()
    main.rx = bytearray(b"\xff\xfe\n")
    assert main.take_line() == ""  # dropped, not a crash
    assert bytes(main.rx) == b""


def test_read_exact_rebinds_without_del():
    main, _, _ = load_main()
    main.rx = bytearray(b"ABCDEF")
    assert main.read_exact(3) == b"ABC"
    assert bytes(main.rx) == b"DEF"


def test_chunk_stream_plays_pcm_credits_and_done():
    script = b"C 4\n" + bytes([10, 20, 200, 255]) + b"Z\n"
    main, uart, captured = load_main(script)
    main.stream()
    out = "".join(uart.out)
    assert "K\n" in out, "no credit sent for the chunk"
    assert "D\n" in out, "no done sent after Z"
    assert captured, "no audio frames played"
    assert captured[0][:4] == [10, 20, 200, 255], "PCM bytes not played verbatim"
    assert captured[0][4] == main.SILENCE, "short frame not padded with silence"


def test_host_stop_sends_no_done():
    script = b"C 2\n" + bytes([1, 2]) + b"S\n"
    main, uart, captured = load_main(script)
    main.stream()
    out = "".join(uart.out)
    assert "K\n" in out
    assert "D\n" not in out, "S is a host-initiated stop; it must not echo D"


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
        print("ok   ", t.__name__)
    print("\n{} firmware logic tests passed".format(len(tests)))


if __name__ == "__main__":
    main()
