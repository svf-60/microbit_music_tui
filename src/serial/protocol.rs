//! The serial command interface between the TUI (the "server" end) and the
//! micro:bit.
//!
//! Wire format: newline-terminated ASCII lines at 115200 baud. The first token
//! is a single-letter opcode; the remainder is its argument. The protocol is
//! deliberately human-readable so it can be exercised with any serial monitor.
//!
//! TUI -> micro:bit:
//!   `H`                     handshake / ping        (expect `R`)
//!   `S`                     stop all playback
//!   `W <rate> <total> <chunk>`  begin a raw PCM stream: `total` 8-bit samples at
//!                           `rate` Hz, sent in `chunk`-byte runs. After this
//!                           line the next `total` bytes on the wire are raw PCM
//!                           (not ASCII lines). The device returns `K` whenever
//!                           it has room for another chunk.
//!   `Z`                     end of the PCM stream    (expect `D` when playback ends)
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
    /// Begin a raw PCM stream; the raw sample bytes follow this line on the wire.
    BeginStream { rate: u32, total: u32, chunk: u32 },
    /// Mark the end of the PCM sample bytes.
    EndStream,
}

impl Command {
    /// Encode as a newline-terminated line ready to write to the port.
    pub fn encode(&self) -> String {
        match self {
            Command::Handshake => "H\n".to_string(),
            Command::Stop => "S\n".to_string(),
            Command::BeginStream { rate, total, chunk } => format!("W {rate} {total} {chunk}\n"),
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
        assert_eq!(
            Command::BeginStream {
                rate: 7812,
                total: 4096,
                chunk: 512
            }
            .encode(),
            "W 7812 4096 512\n"
        );
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
