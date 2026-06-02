//! The serial command interface between the TUI (the "server" end) and the
//! micro:bit.
//!
//! Wire format: newline-terminated ASCII lines at 115200 baud. The first token
//! is a single-letter opcode; the remainder is its argument. The protocol is
//! deliberately human-readable so it can be exercised with any serial monitor.
//!
//! PCM is **length-prefixed**: there is no single fixed-size raw run. Each chunk
//! of sample bytes is announced by its own `C <len>` header, after which exactly
//! `<len>` raw bytes follow on the wire and then the device is back in line mode.
//! This is what keeps the two framings from colliding: the device only ever
//! reads raw bytes for the exact count it was just told, so a command can never
//! be mistaken for PCM (nor PCM for a command). See [`crate::app`] for how the
//! host upholds the invariant (a `C` header is always written immediately
//! followed by its payload, and command lines only ever between whole chunks).
//!
//! TUI -> micro:bit:
//!   `H`              handshake / ping        (expect `R`)
//!   `S`              stop all playback
//!   `W <rate>`       begin a PCM stream at `rate` Hz (8-bit unsigned mono)
//!   `C <len>`        a chunk header: the next `<len>` bytes on the wire are raw
//!                    PCM (not an ASCII line). The device returns `K` per chunk.
//!   `Z`              end of the PCM stream    (expect `D` when playback ends)
//!
//! micro:bit -> TUI:
//!   `R`            ready (on boot, and ack to `H`)
//!   `D`            done — the PCM stream finished playing
//!   `K`            credit — ready to receive another PCM chunk
//!   `P`            transport button: pause / resume toggle
//!   `F`            transport button: forward (seek ahead, or skip at the end)
//!   `B`            transport button: back (rewind, or previous near the start)
//!   `N`            transport button: next song in the queue
//!   `E <message>`  error

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Handshake / ping. The micro:bit replies with [`Response::Ready`].
    Handshake,
    /// Stop playback immediately.
    Stop,
    /// Begin a PCM stream at `rate` Hz. Sample bytes follow as `Chunk`s.
    BeginStream { rate: u32 },
    /// A chunk header; exactly `len` raw PCM bytes follow this line on the wire.
    Chunk { len: u32 },
    /// Mark the end of the PCM sample bytes.
    EndStream,
}

impl Command {
    /// Encode as a newline-terminated line ready to write to the port. For
    /// [`Command::Chunk`] this is only the header; the payload bytes are written
    /// separately (see [`crate::serial::Transport::send_chunk`]).
    pub fn encode(&self) -> String {
        match self {
            Command::Handshake => "H\n".to_string(),
            Command::Stop => "S\n".to_string(),
            Command::BeginStream { rate } => format!("W {rate}\n"),
            Command::Chunk { len } => format!("C {len}\n"),
            Command::EndStream => "Z\n".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Booted, or acknowledging a handshake.
    Ready,
    /// The PCM stream finished playing.
    Done,
    /// Flow-control credit: the device can accept another PCM chunk.
    Credit,
    /// Transport button: pause / resume toggle.
    Pause,
    /// Transport button: forward (seek ahead, or skip to the next song at the end).
    Forward,
    /// Transport button: back (rewind, or previous song near the start).
    Back,
    /// Transport button: skip to the next song in the queue.
    Next,
    /// The micro:bit (or the transport) reported an error.
    Error(String),
    /// A line we did not recognise; kept verbatim for the on-screen log.
    Unknown(String),
}

impl Response {
    /// Parse one line received from the micro:bit (trailing newline optional).
    pub fn parse(line: &str) -> Response {
        let line = line.trim();
        let (op, rest) = match line.split_once(' ') {
            Some((op, rest)) => (op, rest.trim()),
            None => (line, ""),
        };
        match op {
            "R" => Response::Ready,
            "D" => Response::Done,
            "K" => Response::Credit,
            "P" => Response::Pause,
            "F" => Response::Forward,
            "B" => Response::Back,
            "N" => Response::Next,
            "E" => Response::Error(rest.to_string()),
            _ => Response::Unknown(line.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_commands() {
        assert_eq!(Command::Handshake.encode(), "H\n");
        assert_eq!(Command::Stop.encode(), "S\n");
        assert_eq!(Command::BeginStream { rate: 7812 }.encode(), "W 7812\n");
        assert_eq!(Command::Chunk { len: 512 }.encode(), "C 512\n");
        assert_eq!(Command::EndStream.encode(), "Z\n");
    }

    #[test]
    fn parses_responses() {
        assert_eq!(Response::parse("R"), Response::Ready);
        assert_eq!(Response::parse("R\r\n"), Response::Ready);
        assert_eq!(Response::parse("D"), Response::Done);
        assert_eq!(Response::parse("K"), Response::Credit);
        assert_eq!(Response::parse("P"), Response::Pause);
        assert_eq!(Response::parse("F"), Response::Forward);
        assert_eq!(Response::parse("B"), Response::Back);
        assert_eq!(Response::parse("N"), Response::Next);
        assert_eq!(
            Response::parse("E bad note"),
            Response::Error("bad note".into())
        );
        assert_eq!(Response::parse("???"), Response::Unknown("???".into()));
    }
}
