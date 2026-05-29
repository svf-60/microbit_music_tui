# micro:bit Music Streamer — Part 2: How It Works Under the Hood

*A presentation script for the deeper / harder section. Grade 11 level — a few
terms are defined as we go.*

---

## Slide 1 — The code map

The Rust program is split into focused pieces so each one has a single job:

- **`app`** — the "brain." It owns the song list, what's selected, the
  connection, the current playback, and the on-screen log. It reacts to events.
- **`audio`** — everything about songs:
  - `mod.rs`: finding WAV files in a folder.
  - `stream.rs`: turning a WAV into sound data the micro:bit can play.
  - `playback.rs`: the definitions and the math for seeking/skipping.
- **`serial`** — talking over the USB cable:
  - `mod.rs`: opening the port and reading/writing bytes.
  - `protocol.rs`: the exact "language" (commands and replies).
- **`ui`** — drawing the terminal screen.
- **`microbit/main.py`** — the program that runs *on the micro:bit*.

Keeping these separate means you can understand or change one part without
breaking the others.

---

## Slide 2 — From a WAV file to the speaker

A WAV file is high quality: often **stereo** (two channels), **44,100 samples
per second**, **16 bits** per sample. The micro:bit's little speaker can't do
that. So the computer **converts** the audio in three steps (`audio/stream.rs`):

1. **Downmix to mono.** Average the left and right channels into one.
2. **Resample** from 44,100 Hz down to about **7,812 Hz**. *Resampling* means
   recalculating the samples for a new rate; we use linear interpolation
   (estimating in-between values along a straight line).
3. **Quantize to 8-bit.** Convert each sample to a whole number from 0 to 255,
   where **128 means silence**. *Quantizing* = rounding to a smaller set of
   possible values.

The result is small, simple audio that the micro:bit can play directly. (Yes,
it sounds lo-fi — roughly old-telephone quality — but that's the point.)

---

## Slide 3 — Why so low-quality? The bandwidth wall

The USB serial link runs at **115200 baud** (~11.5 kilobytes per second). Our
audio needs about **7.8 kilobytes per second** (7,812 samples × 1 byte). That
*barely* fits. If we tried CD quality (~176 KB/s), it would be impossible over
this cable. So the low sample rate and 8-bit size aren't laziness — they're what
the connection can actually carry in real time.

This tight budget is the reason the next piece — flow control — matters so much.

---

## Slide 4 — The protocol (the shared language)

Everything is **newline-terminated text** (`serial/protocol.rs`), so it's
human-readable. First letter = the command.

**Computer → micro:bit:**
| Message | Meaning |
|---|---|
| `H` | handshake ("are you there?") |
| `W <rate> <total> <chunk>` | "starting audio: this many samples, in chunks this big" |
| `Z` | end of the audio |
| `S` | stop |

**micro:bit → computer:**
| Message | Meaning |
|---|---|
| `R` | ready |
| `K` | "credit" — I'm ready for another chunk |
| `D` | done playing |
| `P` / `F` / `B` / `N` | button: pause / forward / back / next |

After a `W` line, the next bytes on the wire are the **raw audio** (not text) —
the one exception to the text rule.

---

## Slide 5 — Flow control: credits and a "window" (the clever part)

**The problem:** the micro:bit has very little memory, and it plays audio at a
fixed speed. If the computer sends data faster than the micro:bit plays it, the
extra data is lost and the audio breaks up.

**The solution — a credit system:**
- The computer sends audio in small **chunks** (512 bytes each).
- It only allows a few chunks "in flight" at once (a **window** of 4).
- Every time the micro:bit finishes playing a chunk, it sends back a **`K`
  (credit)**, meaning "I have room for one more."
- The computer waits for credits before sending more.

**Analogy:** it's like a kitchen that only has space for 4 plates. The waiter
(computer) doesn't bring a 5th plate until the cook (micro:bit) hands one back.
This automatically matches the sending speed to the playing speed — no
guessing, no timers.

---

## Slide 6 — Event-driven streaming (no timers, almost no threads)

The computer side is built as **one main loop** that, over and over:
1. checks for messages from the micro:bit,
2. redraws the screen,
3. checks the keyboard.

There's only **one extra thread**: a background reader that listens to the
cable so the program never freezes waiting for data. (*A thread is a second line
of execution running at the same time.*)

Crucially, there are **no timers** for the audio. Sending more data is triggered
*by* the `K` credits arriving. This is called being **event-driven**: things
happen in response to events, not on a clock. It keeps the on-screen progress
bar perfectly in step with the real sound.

---

## Slide 7 — The micro:bit side: a "generator" feeding the speaker

On the micro:bit (`microbit/main.py`), playing works through a **generator** — a
function that produces values one at a time, only when asked. The speaker
library asks the generator for the next 32 samples whenever it needs them, and
the generator reads exactly that many bytes off the cable.

This is elegant because **the speaker sets the pace.** Reading only happens when
the speaker is hungry, so we never read too fast.

Between each batch of samples, the micro:bit also:
- checks the buttons and the volume knob, and
- sends a `K` credit when it has consumed a chunk.

If you press a button mid-song, the generator simply **stops**, the micro:bit
clears its buffer, and it tells the computer which button you pressed.

---

## Slide 8 — Transport controls: seeking, skipping, and staying in sync

The **computer** is the single source of truth for "where are we in the song,"
tracked as a byte position (`audio/playback.rs`).

- **Seek** (jump forward/back a few seconds) = move that position and restart
  the stream from the new spot. The audio is already in memory, so no re-reading
  the file.
- **Skip** at the very end of a song moves to the next; rewinding at the very
  start moves to the previous. **Next** always jumps to the next song and wraps
  around at the end of the list.

**A subtle problem:** because a few chunks are always "in flight," the computer
is about a quarter-second *ahead* of the actual sound, and old `K`/`D` messages
can arrive right after you skip. The fix is a small rule: a "done" message only
counts if we've actually sent the end-marker. Anything left over from a stream
we already jumped past is **ignored**. This avoids the player thinking a song
finished when you just skipped it.

---

## Slide 9 — Detecting disconnects (an elegant trick)

How does the computer know if you unplug the micro:bit? It doesn't poll or guess.

The background reader thread *owns* one end of an internal channel (a pipe
between threads). When the cable drops, that thread ends, its end of the channel
closes, and the main loop's next read returns a special "disconnected" result.
The app then flips to **DISCONNECTED**. No constant checking — the closed
channel *is* the signal. It's clean and uses a guarantee the language already
gives us.

---

## Slide 10 — On-device volume (independent from the computer)

The volume **potentiometer** is wired to one of the micro:bit's analog pins. The
micro:bit reads it as a number from 0 to 1023 (*analog* = a continuously varying
value, unlike a simple on/off *digital* signal), scales it to the 0–255 the
speaker expects, and sets the volume itself.

The computer is never told about volume — exactly as intended. To avoid tiny
jitters causing constant updates, it only changes the volume when the reading
moves by more than a small threshold.

---

## Slide 11 — Handshake and reconnection

On boot, the micro:bit announces `R` and keeps re-announcing about once a second
until the computer connects, so it doesn't matter which device powered on first.
If the computer reconnects later and sends `H`, the micro:bit stops whatever it
was doing and acknowledges — a clean resync. (USB unplugging can't be detected
directly on the micro:bit, so the handshake is how it recovers.)

---

## Slide 12 — Quality and testing

- **Automated tests** cover the tricky pure logic: the WAV conversion math,
  the seek/skip calculations, and parsing the protocol messages.
- The Rust code passes **clippy** (the linter) with **zero warnings** and is
  formatted consistently.
- The micro:bit program is kept small and dependency-free so it fits and runs on
  the limited hardware.

**Key takeaways to end on:**
- Match the data rate to what the hardware and cable can handle.
- Let the consumer set the pace (credits/generators) instead of guessing.
- Keep one source of truth (the computer owns the position).
- Use the language's guarantees (a closed channel = a disconnect) instead of
  hand-rolled polling.
