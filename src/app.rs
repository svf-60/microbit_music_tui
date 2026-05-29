//! The application: it *is* the music player. It owns the song library, the
//! list cursor, the serial connection, the current stream and the on-screen log,
//! drains device responses, and drives playback over the wire.
//!
//! Streaming is paced by the micro:bit, with no timers or extra threads: we send
//! PCM sample chunks up to a small window and send more whenever the device
//! returns a `Credit`. The stream state and seek maths are defined in
//! [`crate::audio::playback`]; the orchestration lives here.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;

use crate::audio::playback::{self, Playback, Seek};
use crate::audio::{self, Song};
use crate::serial::protocol::{Command, Response};
use crate::serial::{Connection, ConnectionState};

const LOG_CAP: usize = 200;

pub struct App {
    pub dir: PathBuf,

    pub songs: Vec<Song>,
    pub selected: usize,

    pub conn: Option<Connection>,
    pub conn_state: ConnectionState,

    pub playback: Option<Playback>,
    pub log: VecDeque<String>,

    pub status_msg: String,
    pub force_quit: bool,
}

impl App {
    pub fn new(dir: PathBuf, conn: Option<Connection>) -> App {
        let conn_state = if conn.is_some() {
            ConnectionState::Connecting
        } else {
            ConnectionState::NoPort
        };
        let mut app = App {
            dir,
            songs: Vec::new(),
            selected: 0,
            conn,
            conn_state,
            playback: None,
            log: VecDeque::new(),
            status_msg: String::new(),
            force_quit: false,
        };
        app.refresh_songs();
        if app.conn.is_some() {
            app.send(Command::Handshake);
        }
        app
    }

    fn log_line(&mut self, line: impl Into<String>) {
        self.log.push_back(line.into());
        while self.log.len() > LOG_CAP {
            self.log.pop_front();
        }
    }

    /// Re-read the song directory (manual refresh).
    pub fn refresh_songs(&mut self) {
        match audio::load_dir(&self.dir) {
            Ok(songs) => {
                self.status_msg =
                    format!("Loaded {} song(s) from {}", songs.len(), self.dir.display());
                self.songs = songs;
            }
            Err(e) => {
                self.songs.clear();
                self.status_msg = format!("Failed to read {}: {e}", self.dir.display());
            }
        }
        if self.selected >= self.songs.len() {
            self.selected = self.songs.len().saturating_sub(1);
        }
        let msg = self.status_msg.clone();
        self.log_line(msg);
    }

    pub fn select_next(&mut self) {
        if !self.songs.is_empty() {
            self.selected = (self.selected + 1) % self.songs.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.songs.is_empty() {
            self.selected = (self.selected + self.songs.len() - 1) % self.songs.len();
        }
    }

    /// No serial connection: the app runs as a read-only song browser.
    pub fn is_offline(&self) -> bool {
        self.conn.is_none()
    }

    /// The song under the cursor, if any.
    pub fn selected_song(&self) -> Option<&Song> {
        self.songs.get(self.selected)
    }

    fn send(&mut self, cmd: Command) {
        let line = cmd.encode().trim_end().to_string();
        match self.conn.as_mut() {
            // A write failure clears the connection's liveness flag; poll_serial
            // observes that and transitions to Disconnected, so we only log here.
            Some(conn) => match conn.send(&cmd) {
                Ok(()) => self.log_line(format!("> {line}")),
                Err(e) => self.log_line(format!("! send failed: {e}")),
            },
            None => self.log_line(format!("! no port; dropped {line}")),
        }
    }

    /// Begin streaming the selected song from its start.
    pub fn play_selected(&mut self) {
        if self.songs.is_empty() {
            self.status_msg = "No songs to play".to_string();
            return;
        }
        if self.is_offline() {
            self.status_msg = "Offline — connect a micro:bit to play".to_string();
            return;
        }
        let idx = self.selected;
        self.start_song(idx);
    }

    /// Decode song `idx` and start a fresh stream from its first sample.
    fn start_song(&mut self, idx: usize) {
        let decoded = match self.songs[idx].stream(playback::PCM_RATE) {
            Ok(decoded) => decoded,
            Err(e) => {
                self.playback = None;
                self.status_msg = format!("WAV decode failed: {e}");
                self.log_line(format!("! wav: {e}"));
                return;
            }
        };
        let total = decoded.samples.len();
        if total == 0 {
            self.playback = None;
            self.status_msg = "WAV has no samples".to_string();
            return;
        }
        self.status_msg = format!("Playing {} ({} Hz)", self.songs[idx].name, decoded.rate);
        self.send(Command::BeginStream {
            rate: decoded.rate,
            total: total as u32,
            chunk: playback::PCM_CHUNK as u32,
        });
        self.playback = Some(Playback {
            song_index: idx,
            samples: decoded.samples,
            pos: 0,
            in_flight: 0,
            ended: false,
            paused: false,
            rate: decoded.rate,
        });
        self.pump_pcm();
    }

    /// Restart the current song's stream from byte `pos` (seek / resume). Reuses
    /// the already-decoded samples — no re-decode.
    fn seek_to(&mut self, pos: usize) {
        let (remaining, rate) = match self.playback.as_mut() {
            Some(pb) => {
                pb.pos = pos.min(pb.samples.len());
                pb.in_flight = 0;
                pb.ended = false;
                pb.paused = false;
                ((pb.samples.len() - pb.pos) as u32, pb.rate)
            }
            None => return,
        };
        self.send(Command::BeginStream {
            rate,
            total: remaining,
            chunk: playback::PCM_CHUNK as u32,
        });
        self.pump_pcm();
    }

    /// Forward control: seek ahead, or skip to the next song at the end.
    pub fn forward(&mut self) {
        let (pos, total, rate, idx) = match self.playback.as_ref() {
            Some(pb) => (pb.pos, pb.samples.len(), pb.rate, pb.song_index),
            None => return,
        };
        match playback::forward_target(pos, total, playback::SEEK_SECS * rate as usize) {
            Seek::To(target) => self.seek_to(target),
            Seek::Boundary => {
                if idx + 1 < self.songs.len() {
                    self.selected = idx + 1;
                    self.start_song(idx + 1);
                } else {
                    self.stop();
                    self.status_msg = "End of library".to_string();
                }
            }
        }
    }

    /// Back control: rewind, or skip to the previous song near the start.
    pub fn back(&mut self) {
        let (pos, rate, idx) = match self.playback.as_ref() {
            Some(pb) => (pb.pos, pb.rate, pb.song_index),
            None => return,
        };
        match playback::back_target(pos, playback::SEEK_SECS * rate as usize) {
            Seek::To(target) => self.seek_to(target),
            Seek::Boundary => {
                if idx > 0 {
                    self.selected = idx - 1;
                    self.start_song(idx - 1);
                } else {
                    self.seek_to(0); // first song already: restart it
                }
            }
        }
    }

    /// Next control: skip to the next song in the queue, wrapping at the end.
    pub fn next_song(&mut self) {
        let idx = match self.playback.as_ref() {
            Some(pb) => pb.song_index,
            None => return,
        };
        if self.songs.is_empty() {
            return;
        }
        let next = (idx + 1) % self.songs.len();
        self.selected = next;
        self.start_song(next);
    }

    /// Pause control: hold streaming, or resume from the current position.
    pub fn pause_toggle(&mut self) {
        let (paused, pos) = match self.playback.as_ref() {
            Some(pb) => (pb.paused, pb.pos),
            None => return,
        };
        if paused {
            self.status_msg = "Resumed".to_string();
            self.seek_to(pos);
        } else {
            if let Some(pb) = self.playback.as_mut() {
                pb.paused = true;
            }
            self.status_msg = "Paused".to_string();
            self.send(Command::Stop);
        }
    }

    /// Send PCM chunks while the device window has room and data remains; once
    /// everything is sent, write the end marker exactly once.
    fn pump_pcm(&mut self) {
        loop {
            let (pos, total, in_flight, ended, paused) = match &self.playback {
                Some(p) => (p.pos, p.samples.len(), p.in_flight, p.ended, p.paused),
                None => return,
            };
            if paused || ended || in_flight >= playback::PCM_WINDOW {
                return;
            }
            if pos >= total {
                self.send(Command::EndStream);
                if let Some(p) = self.playback.as_mut() {
                    p.ended = true;
                }
                return;
            }
            let end = (pos + playback::PCM_CHUNK).min(total);
            let chunk: Vec<u8> = match &self.playback {
                Some(p) => p.samples[pos..end].to_vec(),
                None => return,
            };
            if !self.send_raw(&chunk) {
                return;
            }
            if let Some(p) = self.playback.as_mut() {
                p.pos = end;
                p.in_flight += 1;
            }
        }
    }

    fn send_raw(&mut self, data: &[u8]) -> bool {
        match self.conn.as_mut() {
            Some(conn) => match conn.send_raw(data) {
                Ok(()) => true,
                Err(e) => {
                    self.log_line(format!("! pcm send failed: {e}"));
                    false
                }
            },
            None => false,
        }
    }

    /// Stop any current stream.
    pub fn stop(&mut self) {
        if self.playback.take().is_some() {
            self.status_msg = "Stopped".to_string();
            self.send(Command::Stop);
        }
    }

    /// Drain every response the reader thread has queued and apply it. A closed
    /// channel means the reader thread exited — i.e. the device disconnected.
    pub fn poll_serial(&mut self) {
        let mut responses = Vec::new();
        let mut disconnected = false;
        if let Some(conn) = self.conn.as_ref() {
            loop {
                match conn.responses.try_recv() {
                    Ok(resp) => responses.push(resp),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        for resp in responses {
            self.apply_response(resp);
        }
        if disconnected && self.conn_state != ConnectionState::Disconnected {
            self.conn_state = ConnectionState::Disconnected;
            self.playback = None;
            self.status_msg = "Device disconnected".to_string();
            self.log_line("! device disconnected");
        }
    }

    fn apply_response(&mut self, resp: Response) {
        match resp {
            Response::Ready => {
                if self.conn_state != ConnectionState::Ready {
                    self.conn_state = ConnectionState::Ready;
                }
                self.log_line("< R (ready)");
            }
            Response::Done => {
                // A `D` only means "finished" once we've sent EndStream. A `D`
                // while ended == false is a leftover from a stream we already
                // seeked/skipped past, so ignore it.
                if self.playback.as_ref().is_some_and(|pb| pb.ended) {
                    self.log_line("< D (done)");
                    self.finish();
                }
            }
            Response::Credit => {
                // Not logged: there's one credit per chunk (~15/s) and it's pure
                // flow-control plumbing.
                if let Some(p) = self.playback.as_mut() {
                    p.in_flight = p.in_flight.saturating_sub(1);
                }
                self.pump_pcm();
            }
            Response::Pause => {
                self.log_line("< P (device pause)");
                self.pause_toggle();
            }
            Response::Forward => {
                self.log_line("< F (device forward)");
                self.forward();
            }
            Response::Back => {
                self.log_line("< B (device back)");
                self.back();
            }
            Response::Next => {
                self.log_line("< N (device next)");
                self.next_song();
            }
            Response::Error(msg) => self.log_line(format!("< E {msg}")),
            Response::Unknown(line) => self.log_line(format!("< {line}")),
        }
    }

    /// Clear the current stream and report the finished song.
    fn finish(&mut self) {
        let name = self
            .playback
            .as_ref()
            .map(|pb| pb.song_index)
            .and_then(|i| self.songs.get(i))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        self.playback = None;
        self.status_msg = format!("Finished {name}");
    }
}
