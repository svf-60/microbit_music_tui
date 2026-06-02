//! End-to-end framing tests: drive a real `App` through every control path and
//! prove the exact bytes it puts on the wire can always be decoded by the
//! device, i.e. a command byte is never read as PCM nor PCM as a command — the
//! UnicodeError the fix is about.
//!
//! `MockTransport` records the wire verbatim; `DeviceSim` replays it with the
//! same framing rules `microbit/main.py` implements.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::TryRecvError;

use anyhow::Result;

use microbit_music_tui::app::App;
use microbit_music_tui::audio::Song;
use microbit_music_tui::audio::playback::PCM_RATE;
use microbit_music_tui::serial::Transport;
use microbit_music_tui::serial::protocol::{Command, Response};

// ---------------------------------------------------------------------------
// In-memory transport: captures the exact byte stream the device would receive.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MockTransport {
    wire: Rc<RefCell<Vec<u8>>>,
    inbox: Rc<RefCell<VecDeque<Response>>>,
}

impl Transport for MockTransport {
    fn send(&mut self, cmd: &Command) -> Result<()> {
        self.wire.borrow_mut().extend_from_slice(cmd.encode().as_bytes());
        Ok(())
    }

    fn send_chunk(&mut self, data: &[u8]) -> Result<()> {
        // Identical layout to the real Connection: header then payload, together.
        let header = Command::Chunk {
            len: data.len() as u32,
        }
        .encode();
        let mut wire = self.wire.borrow_mut();
        wire.extend_from_slice(header.as_bytes());
        wire.extend_from_slice(data);
        Ok(())
    }

    fn try_recv(&mut self) -> std::result::Result<Response, TryRecvError> {
        self.inbox.borrow_mut().pop_front().ok_or(TryRecvError::Empty)
    }

    fn port_name(&self) -> &str {
        "mock"
    }
}

// ---------------------------------------------------------------------------
// Device-side parser model, mirroring microbit/main.py.
// ---------------------------------------------------------------------------

enum Mode {
    /// Reading newline-delimited command lines (decoded as UTF-8).
    Line,
    /// Reading exactly N more raw PCM bytes after a `C` header.
    Raw(usize),
}

struct DeviceSim {
    mode: Mode,
    line: Vec<u8>,
    pcm: Vec<u8>,
    commands: Vec<String>,
    /// Command lines that were not a known opcode — a sign of misframing.
    unknown: Vec<String>,
    /// Set on the first line that fails to decode as UTF-8: the UnicodeError.
    error: Option<String>,
}

impl DeviceSim {
    fn new() -> Self {
        DeviceSim {
            mode: Mode::Line,
            line: Vec::new(),
            pcm: Vec::new(),
            commands: Vec::new(),
            unknown: Vec::new(),
            error: None,
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.error.is_some() {
                return;
            }
            match self.mode {
                Mode::Raw(rem) => {
                    self.pcm.push(b);
                    let rem = rem - 1;
                    self.mode = if rem == 0 { Mode::Line } else { Mode::Raw(rem) };
                }
                Mode::Line if b == b'\n' => {
                    let line = std::mem::take(&mut self.line);
                    match std::str::from_utf8(&line) {
                        Ok(s) => self.handle(s.trim()),
                        Err(_) => {
                            self.error =
                                Some(format!("UnicodeError decoding command line: {line:?}"))
                        }
                    }
                }
                Mode::Line => self.line.push(b),
            }
        }
    }

    fn handle(&mut self, line: &str) {
        let mut it = line.split_whitespace();
        let op = match it.next() {
            Some(op) => op,
            None => return, // blank line
        };
        match op {
            "C" => match it.next().and_then(|n| n.parse::<usize>().ok()) {
                Some(len) => {
                    self.commands.push(format!("C {len}"));
                    if len > 0 {
                        self.mode = Mode::Raw(len);
                    }
                }
                None => self.unknown.push(line.to_string()),
            },
            "W" | "S" | "Z" | "H" => self.commands.push(line.to_string()),
            _ => self.unknown.push(line.to_string()),
        }
    }

    fn count(&self, op: &str) -> usize {
        self.commands
            .iter()
            .filter(|c| c.split_whitespace().next() == Some(op))
            .count()
    }
}

/// Replay a wire dump through the device model and assert it stays perfectly
/// framed: no decode error, no stray non-command line, and no half-read chunk
/// left dangling.
fn assert_decodable(wire: &[u8]) -> DeviceSim {
    let mut sim = DeviceSim::new();
    sim.feed(wire);
    assert!(
        sim.error.is_none(),
        "device hit a UnicodeError: {:?}",
        sim.error
    );
    assert!(
        sim.unknown.is_empty(),
        "device read non-command bytes as a command line: {:?}",
        sim.unknown
    );
    assert!(
        matches!(sim.mode, Mode::Line) && sim.line.is_empty(),
        "stream left the device mid-chunk — framing desync"
    );
    sim
}

// ---------------------------------------------------------------------------
// Fixtures and a tiny device driver.
// ---------------------------------------------------------------------------

fn write_wav(path: &Path, n: usize) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: PCM_RATE, // identity resample -> n PCM bytes out
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..n {
        // Spread values across the range so PCM contains 0x0A and bytes >= 0x80,
        // which would trip a naive line reader if anything were misframed.
        let v = (i.wrapping_mul(7919) % 65536) as i32 - 32768;
        w.write_sample(v as i16).unwrap();
    }
    w.finalize().unwrap();
}

struct Ctx {
    app: App,
    inbox: Rc<RefCell<VecDeque<Response>>>,
    wire: Rc<RefCell<Vec<u8>>>,
}

fn build(dir: &Path) -> Ctx {
    let wire = Rc::new(RefCell::new(Vec::new()));
    let inbox = Rc::new(RefCell::new(VecDeque::new()));
    let mock = MockTransport {
        wire: wire.clone(),
        inbox: inbox.clone(),
    };
    let app = App::new(dir.to_path_buf(), Some(Box::new(mock)));
    Ctx { app, inbox, wire }
}

impl Ctx {
    fn recv(&mut self, resp: Response) {
        self.inbox.borrow_mut().push_back(resp);
        self.app.poll_serial();
    }

    /// Feed `n` credits (each lets the host send one more chunk).
    fn credits(&mut self, n: usize) {
        for _ in 0..n {
            if self.app.playback.is_some() {
                self.recv(Response::Credit);
            }
        }
    }

    /// Drive credits until the song ends (then ack with `D`), or it stalls
    /// (paused / stopped).
    fn run(&mut self) {
        for _ in 0..1_000_000 {
            match self.app.playback.as_ref() {
                None => return,
                Some(p) if p.paused => return,
                Some(p) if p.ended => {
                    self.recv(Response::Done);
                    return;
                }
                Some(_) => self.recv(Response::Credit),
            }
        }
        panic!("stream did not terminate");
    }

    fn wire(&self) -> Vec<u8> {
        self.wire.borrow().clone()
    }
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mb_sync_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Scenarios — every one asserts the captured wire is fully decodable.
// ---------------------------------------------------------------------------

#[test]
fn clean_playthrough_is_decodable_and_lossless() {
    let dir = tmp_dir("clean");
    write_wav(&dir.join("a.wav"), 5000);
    let mut ctx = build(&dir);

    ctx.app.play_selected();
    ctx.run();

    let sim = assert_decodable(&ctx.wire());
    let expected = Song {
        name: "a".into(),
        path: dir.join("a.wav"),
    }
    .stream(PCM_RATE)
    .unwrap()
    .samples;
    assert_eq!(sim.pcm, expected, "device received different PCM than sent");
    assert_eq!(sim.count("W"), 1);
    assert_eq!(sim.count("Z"), 1);
    assert!(sim.count("C") >= 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pause_then_resume_is_decodable() {
    let dir = tmp_dir("pause");
    write_wav(&dir.join("a.wav"), 8000);
    let mut ctx = build(&dir);

    ctx.app.play_selected();
    ctx.credits(3);
    ctx.app.pause_toggle(); // pause -> sends S
    assert!(ctx.app.playback.as_ref().unwrap().paused);
    ctx.app.pause_toggle(); // resume -> sends S then W
    ctx.run();

    assert_decodable(&ctx.wire());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stop_midstream_is_decodable() {
    let dir = tmp_dir("stop");
    write_wav(&dir.join("a.wav"), 8000);
    let mut ctx = build(&dir);

    ctx.app.play_selected();
    ctx.credits(4);
    ctx.app.stop();
    assert!(ctx.app.playback.is_none());

    assert_decodable(&ctx.wire());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn seek_forward_within_song_is_decodable() {
    let dir = tmp_dir("seek");
    // Long enough that a 5s forward seek lands within the song.
    write_wav(&dir.join("a.wav"), 60_000);
    let mut ctx = build(&dir);

    ctx.app.play_selected();
    ctx.credits(5);
    ctx.app.forward(); // within-song seek -> S then W from the new offset
    ctx.run();

    assert_decodable(&ctx.wire());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn next_song_midstream_is_decodable() {
    let dir = tmp_dir("next");
    write_wav(&dir.join("a.wav"), 6000);
    write_wav(&dir.join("b.wav"), 6000);
    let mut ctx = build(&dir);

    ctx.app.play_selected();
    ctx.credits(3);
    ctx.app.next_song();
    assert_eq!(ctx.app.playback.as_ref().unwrap().song_index, 1);
    ctx.run();

    assert_decodable(&ctx.wire());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn device_initiated_controls_are_decodable() {
    let dir = tmp_dir("devctl");
    write_wav(&dir.join("a.wav"), 60_000);
    write_wav(&dir.join("b.wav"), 6000);
    let mut ctx = build(&dir);

    ctx.app.play_selected();
    ctx.credits(3);
    ctx.recv(Response::Pause); // device button: pause
    ctx.recv(Response::Pause); // device button: resume
    ctx.credits(3);
    ctx.recv(Response::Forward); // device button: forward (seek within)
    ctx.credits(3);
    ctx.recv(Response::Next); // device button: next song
    ctx.run();

    assert_decodable(&ctx.wire());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rapid_control_burst_is_decodable() {
    let dir = tmp_dir("burst");
    write_wav(&dir.join("a.wav"), 60_000);
    write_wav(&dir.join("b.wav"), 60_000);
    let mut ctx = build(&dir);

    // Fire controls back-to-back with no credits in between — the worst case for
    // splicing commands into the byte stream.
    ctx.app.play_selected();
    ctx.app.pause_toggle();
    ctx.app.pause_toggle();
    ctx.app.forward();
    ctx.app.next_song();
    ctx.app.stop();
    ctx.app.play_selected();
    ctx.run();

    assert_decodable(&ctx.wire());
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// The model has teeth: it must catch the very desync the fix prevents.
// ---------------------------------------------------------------------------

#[test]
fn device_sim_flags_non_ascii_in_command_position() {
    let mut sim = DeviceSim::new();
    sim.feed(b"W 7812\n");
    sim.feed(&[0xC3, 0x28, b'\n']); // invalid UTF-8 line
    assert!(sim.error.is_some());
}

#[test]
fn unframed_pcm_after_w_would_crash_device() {
    // The pre-fix host wrote raw PCM straight after `W` with no `C` framing.
    // Against the only desync-free contract those bytes land in line position
    // and blow up — exactly the reported bug.
    let mut sim = DeviceSim::new();
    sim.feed(b"W 7812 5000 512\n");
    sim.feed(&[200u8, 50, 130, b'\n', 90, 255, b'\n']);
    assert!(sim.error.is_some() || !sim.unknown.is_empty());
}
