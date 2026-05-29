//! Serial transport: open a port, write commands, and read responses on a
//! background thread that pushes them onto an [`mpsc`] channel for the UI loop.
//!
//! Disconnection needs no extra signalling. The reader thread owns the channel's
//! `Sender`, so when the device goes away (EOF or a read error) the thread
//! returns, the sender drops, and the receiver then yields
//! `TryRecvError::Disconnected`. The app turns that into its own connection
//! state — see [`crate::app`].

pub mod protocol;

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use serialport::SerialPort;

use self::protocol::{Command, Response};

/// Lifecycle of the link to the micro:bit, as surfaced in the UI. The transport
/// owns the type; the app drives the transitions from responses and channel
/// closure (see [`crate::app`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// No port was configured or none could be found.
    NoPort,
    /// Port is open; waiting for the first `Ready` from the device.
    Connecting,
    /// Handshake complete / idle.
    Ready,
    /// The device went away mid-session.
    Disconnected,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            ConnectionState::NoPort => "OFFLINE",
            ConnectionState::Connecting => "CONNECTING",
            ConnectionState::Ready => "READY",
            ConnectionState::Disconnected => "DISCONNECTED",
        }
    }
}

/// An open connection to the micro:bit: a writable handle plus a channel of
/// decoded responses produced by the reader thread.
pub struct Connection {
    port: Box<dyn SerialPort>,
    pub responses: Receiver<Response>,
    pub port_name: String,
}

impl Connection {
    /// Open `port_name` at `baud` and start the background reader thread.
    pub fn open(port_name: &str, baud: u32) -> Result<Connection> {
        let port = serialport::new(port_name, baud)
            .timeout(Duration::from_millis(200))
            .open()
            .with_context(|| format!("opening serial port {port_name}"))?;

        let reader = port
            .try_clone()
            .context("cloning serial port for reader thread")?;

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || read_loop(reader, tx));

        Ok(Connection {
            port,
            responses: rx,
            port_name: port_name.to_string(),
        })
    }

    /// Write a command to the micro:bit.
    pub fn send(&mut self, cmd: &Command) -> Result<()> {
        self.port
            .write_all(cmd.encode().as_bytes())
            .and_then(|()| self.port.flush())
            .context("writing to serial port")
    }

    /// Write raw bytes (used for PCM sample data, which is not line-framed).
    pub fn send_raw(&mut self, data: &[u8]) -> Result<()> {
        self.port
            .write_all(data)
            .and_then(|()| self.port.flush())
            .context("writing PCM to serial port")
    }
}

/// Read bytes forever, splitting them into newline-delimited lines and
/// forwarding each parsed [`Response`]. Partial lines are carried across reads,
/// so a read timeout never drops buffered data. EOF or a hard error ends the
/// thread, which drops `tx` and thereby signals disconnection to the receiver.
fn read_loop(mut port: Box<dyn SerialPort>, tx: Sender<Response>) {
    let mut buf = [0u8; 256];
    let mut pending: Vec<u8> = Vec::new();

    loop {
        match port.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=pos).collect();
                    let resp = Response::parse(&String::from_utf8_lossy(&line));
                    if tx.send(resp).is_err() {
                        return; // UI dropped the receiver; nothing left to do.
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(_) => return,
        }
    }
}

/// Names of all serial ports the OS currently reports.
pub fn list_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default()
}

/// Best guess at the micro:bit's port: the first USB/ACM-style device, falling
/// back to the first port of any kind.
pub fn autodetect() -> Option<String> {
    let ports = serialport::available_ports().ok()?;
    ports
        .iter()
        .find(|p| {
            let name = p.port_name.to_lowercase();
            name.contains("ttyacm") || name.contains("usbmodem") || name.contains("ttyusb")
        })
        .or_else(|| ports.first())
        .map(|p| p.port_name.clone())
}
