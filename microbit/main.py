"""micro:bit v2 PCM streaming intermediary with transport + on-device volume.

Bridges the Rust TUI's serial protocol (see the host's `serial::protocol`) to the
micro:bit v2 speaker, forwards on-board controls back as transport events, and
manages speaker volume locally from a potentiometer (never involving the host).

    host  -> device:  H                        handshake            -> R
                      S                        stop
                      W <rate> <total> <chunk> begin PCM; the next <total> raw
                                               8-bit samples follow on the wire
                      Z                        end of the PCM stream
    device -> host:   R   ready / handshake ack
                      K   credit: ready for another <chunk> bytes of PCM
                      D   the PCM stream finished playing
                      P/F/B/N  transport: pause / forward / back / next
                      E <msg>  error

Samples are unsigned 8-bit (128 = silence), matching `audio.AudioFrame` and the
host's resampler. Flow control: one `K` per <chunk> bytes consumed.

Breadboard wiring (micro:bit v2):
    Button A     -> back  ('B')
    Button B     -> forward ('F')
    pin0 button  -> pause / resume ('P')   push-button to GND (internal pull-up)
    logo touch   -> next song ('N')
    pin2 pot     -> volume (read locally; never sent to the host)
    pin1         -> audio output, so pin0 stays free as a digital input
                    (the v2 built-in speaker still sounds)

Flash as main.py (e.g. with `uflash` or the Mu editor).
"""

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
)
import audio

BAUD = 115200
FRAME_SIZE = 32  # samples per audio.AudioFrame
SILENCE = 128  # midpoint of unsigned 8-bit audio
VOLUME_STEP = 4  # ignore pot jitter smaller than this (volume is 0..255)

# Bytes read off the UART but not yet consumed.
rx = bytearray()

# Edge-detection state for the controls, and the last volume we applied.
pin0_was = 1  # released, with the pull-up
logo_was = False
volume_last = -1


# --- serial helpers --------------------------------------------------------

def send(text):
    uart.write(text + "\n")


def drain_uart():
    """Move everything waiting in the UART hardware buffer into `rx`."""
    data = uart.read()
    if data:
        rx.extend(data)


def flush_rx():
    """Discard buffered and pending input — used to resync between streams."""
    drain_uart()
    del rx[:]


def take_line():
    """Return one buffered '\\n'-terminated line, or None if none is ready yet."""
    newline = rx.find(b"\n")
    if newline < 0:
        return None
    line = bytes(rx[:newline])
    del rx[: newline + 1]
    return line.decode().strip()


def read_exact(count):
    """Block until `count` bytes are buffered, then remove and return them."""
    while len(rx) < count:
        drain_uart()
    out = bytes(rx[:count])
    del rx[:count]
    return out


# --- controls & volume -----------------------------------------------------

def poll_controls():
    """Return a transport event ('B'/'F'/'P'/'N') if a control fired, else None."""
    global pin0_was, logo_was

    if button_a.was_pressed():
        return "B"  # back / rewind
    if button_b.was_pressed():
        return "F"  # forward / skip

    # pin0 button to GND with pull-up: a falling edge (1 -> 0) is a press.
    pin0_now = pin0.read_digital()
    pin0_pressed = pin0_was == 1 and pin0_now == 0
    pin0_was = pin0_now
    if pin0_pressed:
        return "P"  # pause / resume

    touched = pin_logo.is_touched()
    logo_pressed = touched and not logo_was
    logo_was = touched
    if logo_pressed:
        return "N"  # next song

    return None


def update_volume():
    """Set the speaker volume from the pot on pin2 — entirely on-device."""
    global volume_last
    level = pin2.read_analog() * 255 // 1023  # 0..1023 -> 0..255
    if abs(level - volume_last) > VOLUME_STEP:
        volume_last = level
        set_volume(level)


# --- PCM streaming ---------------------------------------------------------

def pcm_frames(total, chunk, report):
    """Yield AudioFrames read from the wire, pacing reads to playback.

    Tracks the volume pot each frame, and stops early if a control fires
    (recording it in `report`).
    """
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
            frame[i] = SILENCE  # pad the final short frame
        consumed += size
        since_credit += size
        while since_credit >= chunk:
            since_credit -= chunk
            send("K")
        yield frame


def stream(rate, total, chunk):
    audio.set_rate(rate)
    display.show(Image.MUSIC_QUAVER)
    report = {"event": None}
    # Play on pin1 so pin0 is free for the pause button; the v2 speaker sounds too.
    audio.play(pcm_frames(total, chunk, report), wait=True, pin=pin1)
    display.clear()
    flush_rx()  # drop the trailing "Z", or stale chunks after an interrupt
    send(report["event"] or "D")  # forward the control press, or report done


# --- command dispatch ------------------------------------------------------

def dispatch(line):
    if not line:
        return
    parts = line.split()
    op = parts[0]
    if op == "H":
        send("R")
    elif op == "S":
        audio.stop()
    elif op == "W":
        try:
            rate, total, chunk = int(parts[1]), int(parts[2]), int(parts[3])
        except (IndexError, ValueError):
            send("E bad W args")
            return
        stream(rate, total, chunk)
    # Any other line (a stray "Z", or stale PCM during resync) is ignored.


def main():
    global pin0_was, logo_was

    uart.init(baudrate=BAUD)
    pin0.set_pull(pin0.PULL_UP)
    pin0_was = pin0.read_digital()
    logo_was = pin_logo.is_touched()
    update_volume()  # apply the pot's initial position

    display.show(Image.YES)
    send("R")  # announce ready on boot

    while True:
        drain_uart()
        line = take_line()
        if line is not None:
            dispatch(line)
        else:
            update_volume()
            event = poll_controls()  # idle (e.g. paused) control presses
            if event:
                send(event)


main()
