# micro:bit Music Streamer — Part 1: What It Does

*A presentation script for the overview / easier section. Grade 11 level.*

---

## Slide 1 — The one-sentence pitch

> "I built a music player that runs in a terminal on a computer, but the sound
> actually comes out of a tiny BBC micro:bit, and you control it with real
> buttons and a volume knob wired up on a breadboard."

The computer does the hard work (reading the music file, getting it ready), and
the micro:bit is the thing that makes noise and takes your button presses. They
talk to each other over a single USB cable.

---

## Slide 2 — The setup

Three pieces:

1. **The computer program** — a text-based app (no mouse, just the keyboard and
   a nice terminal interface). Written in **Rust**.
2. **The micro:bit v2** — a small educational microcontroller with a built-in
   speaker. It runs a small **MicroPython** program.
3. **The breadboard** — where I wired up two push-buttons and a **potentiometer**
   (a volume knob — a dial that changes an electrical signal as you turn it).

A USB cable connects the computer and the micro:bit. That cable is also a
**serial port**: a simple way for two devices to send bytes back and forth, one
after another.

```
  Computer (Rust app)              micro:bit v2
 ┌────────────────────┐  USB /   ┌──────────────┐
 │  terminal music UI │  serial  │  MicroPython │──► speaker
 │  reads + sends WAV │◄────────►│  plays sound │
 └────────────────────┘          └──────────────┘
                                    ▲  buttons + volume knob
```

---

## Slide 3 — Live demo (the script)

1. **Start the app.** It opens a full-screen terminal interface and finds the
   micro:bit automatically. The top bar turns green and says **READY**.
2. **Pick a song.** The left side lists the WAV files in my music folder. I use
   the arrow keys to highlight one.
3. **Press Enter.** The song starts playing *through the micro:bit's speaker*.
   A progress bar fills up and shows the time, like `0:12 / 3:40`.
4. **Use the hardware.** I press the buttons on the breadboard: pause, skip to
   the next song, jump forward and back. The screen updates instantly to match.
5. **Turn the knob.** The volume changes — and notice the computer doesn't do
   anything; the micro:bit handles volume completely on its own.

---

## Slide 4 — The features

- **Terminal interface (TUI).** Built with a Rust library called *ratatui*. It
  shows a song list, the song that's playing, a progress bar, the connection
  status, and a live log of messages between the computer and the micro:bit.
- **Plays real audio files.** It reads standard **WAV** files from a folder.
- **Streams over serial.** The audio is sent over the USB cable a little at a
  time while it plays — it doesn't have to be loaded all at once.
- **Hardware controls** on the breadboard / micro:bit:
  - Button A = back / rewind
  - Button B = forward
  - a button on pin 0 = pause / resume
  - touch the micro:bit logo = next song
  - the knob (potentiometer) = volume
- **On-device volume.** The volume knob is read and applied *by the micro:bit
  itself* — it never involves the computer. This was a specific design goal.
- **Offline mode.** If no micro:bit is plugged in, the app doesn't crash — it
  becomes a read-only browser so you can still look through your songs.
- **Reliable connection.** It notices when the micro:bit is unplugged and shows
  **DISCONNECTED**, and automatically reconnects when it comes back.

---

## Slide 5 — How the two halves "talk" (the simple version)

The computer and micro:bit send each other **short text messages** over the
cable, almost like texting. For example:

- The computer says `H` ("hello, are you there?") and the micro:bit replies `R`
  ("ready!"). That's the **handshake** — how they confirm they're connected.
- To play a song, the computer says `W ...` ("here comes audio") and then sends
  the actual sound data.
- When you press a button, the micro:bit sends back a letter like `P` (pause) or
  `N` (next), and the computer reacts.

Because the messages are just simple text, they're easy to read and debug — you
could literally watch them go by.

---

## Slide 6 — Why this is neat

- It crosses **two different worlds**: a desktop program (Rust) and embedded
  hardware (a microcontroller running MicroPython), made to cooperate.
- It turns a $20 educational board into a real, controllable audio player.
- The controls are **physical** — buttons and a knob you can touch — not just
  keys on a keyboard.
- It's **resilient**: unplug it, plug it back in, and it just keeps working.

*(Part 2 explains the harder pieces: how the audio is converted, how the data is
paced so it doesn't overflow, and the tricks that keep everything in sync.)*
