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


def flush_rx():
    drain_uart()
    del rx[:]


def find_newline(buf):
    for i in range(len(buf)):
        if buf[i] == 10:
            return i
    return -1


def take_line():
    nl = find_newline(rx)
    if nl < 0:
        return None
    line = bytes(rx[:nl])
    del rx[: nl + 1]
    return line.decode().strip()


def read_exact(count):
    while len(rx) < count:
        drain_uart()
    out = bytes(rx[:count])
    del rx[:count]
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


def pcm_frames(total, chunk, report):
    frame = audio.AudioFrame()
    consumed = 0
    since_credit = 0
    while consumed < total:
        update_volume()
        event = poll_controls()
        if event:
            report["event"] = event
            return
        size = FRAME_SIZE if total - consumed >= FRAME_SIZE else total - consumed
        data = read_exact(size)
        for i in range(size):
            frame[i] = data[i]
        for i in range(size, FRAME_SIZE):
            frame[i] = SILENCE
        consumed += size
        since_credit += size
        while since_credit >= chunk:
            since_credit -= chunk
            send("K")
        yield frame


def stream(total, chunk):
    display.show(Image.MUSIC_QUAVER)
    report = {"event": None}
    audio.play(pcm_frames(total, chunk, report), wait=True, pin=pin1)
    display.show(Image.YES)
    flush_rx()
    send(report["event"] or "D")


def mark_connected():
    global connected
    if not connected:
        connected = True
        display.show(Image.YES)


def dispatch(line):
    if not line:
        return
    parts = line.split()
    op = parts[0]
    if op == "H":
        audio.stop()
        mark_connected()
        send("R")
    elif op == "S":
        mark_connected()
        audio.stop()
    elif op == "W":
        mark_connected()
        try:
            _, total, chunk = int(parts[1]), int(parts[2]), int(parts[3])
        except (IndexError, ValueError):
            send("E bad W args")
            return
        stream(total, chunk)


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


main()
