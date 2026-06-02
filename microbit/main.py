from microbit import (
    uart,
    display,
    Image,
    button_a,
    button_b,
    pin0,
    pin1,
    pin2,
    pin_logo,
    set_volume,
    running_time,
)
import audio

BAUD = 115200
FRAME_SIZE = 32
SILENCE = 128
VOLUME_STEP = 4
ANNOUNCE_MS = 1000

rx = bytearray()
pin0_was = 1
logo_was = False
volume_last = -1
connected = False
last_announce = 0


def send(text):
    uart.write(text + "\n")


def drain_uart():
    data = uart.read()
    if data:
        rx.extend(data)


def find_newline(buf):
    for i in range(len(buf)):
        if buf[i] == 10:
            return i
    return -1


def take_line():
    # Rebind rather than `del rx[:n]`: in-place slice deletion isn't supported on
    # the micro:bit's bytearray, but slicing to a fresh one is.
    global rx
    nl = find_newline(rx)
    if nl < 0:
        return None
    line = bytes(rx[:nl])
    rx = rx[nl + 1 :]
    # A command line is always ASCII; if a stray byte ever slips in, drop the
    # line rather than letting decode() raise and kill the program.
    try:
        return line.decode().strip()
    except Exception:
        return ""


def read_exact(count):
    global rx
    while len(rx) < count:
        drain_uart()
    out = bytes(rx[:count])
    rx = rx[count:]
    return out


def poll_controls():
    global pin0_was, logo_was

    if button_a.was_pressed():
        return "B"
    if button_b.was_pressed():
        return "F"

    pin0_now = pin0.read_digital()
    pin0_pressed = pin0_was == 1 and pin0_now == 0
    pin0_was = pin0_now
    if pin0_pressed:
        return "P"

    touched = pin_logo.is_touched()
    logo_pressed = touched and not logo_was
    logo_was = touched
    if logo_pressed:
        return "N"

    return None


def update_volume():
    global volume_last
    level = pin2.read_analog() * 255 // 1023
    if abs(level - volume_last) > VOLUME_STEP:
        volume_last = level
        set_volume(level)


def stream_frames(report):
    # The stream is a series of `C <len>` chunks ending in `Z`. Between chunks
    # we are back in line mode, so a button press just reports an event (the host
    # then decides whether to stop/seek) and `S`/`Z` are always recognised — PCM
    # bytes can never be mistaken for a command.
    frame = audio.AudioFrame()
    while True:
        drain_uart()
        update_volume()
        event = poll_controls()
        if event:
            send(event)

        line = take_line()
        if line is None:
            for i in range(FRAME_SIZE):
                frame[i] = SILENCE
            yield frame
            continue

        parts = line.split()
        op = parts[0] if parts else ""
        if op == "C":
            try:
                length = int(parts[1])
            except (IndexError, ValueError):
                send("E bad C")
                continue
            # Read the whole chunk once (rx is rebound a single time), then frame
            # it from memory. Flow control keeps chunks buffered ahead, so this
            # rarely blocks and audio stays fed.
            data = read_exact(length)
            consumed = 0
            while consumed < length:
                size = FRAME_SIZE if length - consumed >= FRAME_SIZE else length - consumed
                for i in range(size):
                    frame[i] = data[consumed + i]
                for i in range(size, FRAME_SIZE):
                    frame[i] = SILENCE
                consumed += size
                yield frame
            send("K")
        elif op == "Z":
            report["end"] = "Z"
            return
        elif op == "S":
            report["end"] = "S"
            return
        elif op == "H":
            send("R")


def stream():
    display.show(Image.MUSIC_QUAVER)
    report = {"end": None}
    audio.play(stream_frames(report), wait=True, pin=pin1)
    display.show(Image.YES)
    # `Z` means the song played out; `S` is a host-initiated stop it already knows.
    if report["end"] == "Z":
        send("D")


def mark_connected():
    global connected
    if not connected:
        connected = True
        display.show(Image.YES)


def dispatch(line):
    if not line:
        return
    op = line.split()[0]
    if op == "H":
        audio.stop()
        mark_connected()
        send("R")
    elif op == "S":
        mark_connected()
        audio.stop()
    elif op == "W":
        mark_connected()
        stream()


def main():
    global pin0_was, logo_was, last_announce

    uart.init(baudrate=BAUD)
    pin0.set_pull(pin0.PULL_UP)
    pin0_was = pin0.read_digital()
    logo_was = pin_logo.is_touched()
    update_volume()

    display.show(Image.ASLEEP)
    send("R")
    last_announce = running_time()

    while True:
        drain_uart()
        line = take_line()
        if line is not None:
            dispatch(line)
            continue

        update_volume()
        if connected:
            event = poll_controls()
            if event:
                send(event)
        elif running_time() - last_announce >= ANNOUNCE_MS:
            send("R")
            last_announce = running_time()


if __name__ == "__main__":
    main()
