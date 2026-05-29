use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

use microbit_music_tui::app::App;
use microbit_music_tui::serial::{self, Connection};

const DEFAULT_BAUD: u32 = 115_200;
const DEFAULT_DIR: &str = "melodies";

const TICK: Duration = Duration::from_millis(50);

struct Args {
    dir: PathBuf,
    port: Option<String>,

    baud: u32,
}

fn main() -> Result<()> {
    let args = match parse_args() {
        Some(args) => args,
        None => return Ok(()), // --help / --list-ports already printed
    };

    // Resolve and open the port up front. A missing or unopenable port is not
    // fatal: the TUI still runs as a viewer so you can browse songs offline.
    let port_name = args.port.clone().or_else(serial::autodetect);
    let conn = match &port_name {
        Some(name) => match Connection::open(name, args.baud) {
            Ok(conn) => Some(conn),
            Err(e) => {
                eprintln!("warning: could not open {name}: {e}");
                eprintln!("starting in offline view mode.");
                None
            }
        },
        None => {
            eprintln!("warning: no serial port found; starting in offline view mode.");
            None
        }
    };

    let mut app = App::new(args.dir, conn);
    let mut terminal =
        ratatui::try_init().context("initializing terminal (must be run in a TTY)")?;

    let result = run(&mut terminal, &mut app);

    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.force_quit {
        app.poll_serial();

        terminal.draw(|f| microbit_music_tui::ui::draw(f, app))?;

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            handle_key(app, key.code);
        }
    }

    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.force_quit = true,
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Enter => app.play_selected(),
        KeyCode::Char('s') => app.stop(),
        KeyCode::Char('r') => app.refresh_songs(),
        _ => {}
    }
}

fn parse_args() -> Option<Args> {
    let mut dir = PathBuf::from(DEFAULT_DIR);
    let mut port = None;
    let mut baud = DEFAULT_BAUD;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return None;
            }
            "--list-ports" => {
                let ports = serial::list_ports();
                if ports.is_empty() {
                    println!("no serial ports found");
                } else {
                    for p in ports {
                        println!("{p}");
                    }
                }
                return None;
            }
            "--dir" => dir = PathBuf::from(it.next().unwrap_or_default()),
            "--port" => port = it.next(),
            "--baud" => {
                baud = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_BAUD);
            }
            other => eprintln!("ignoring unknown argument: {other}"),
        }
    }
    Some(Args { dir, port, baud })
}

fn print_help() {
    println!(
        "micro:bit music streamer\n\n\
         Streams WAV files to a micro:bit v2 as 8-bit PCM over serial.\n\n\
         USAGE:\n    \
         microbit_music_tui [OPTIONS]\n\n\
         OPTIONS:\n    \
         --dir <PATH>     Directory of WAV files (default: ./{DEFAULT_DIR})\n    \
         --port <NAME>    Serial port (default: auto-detect)\n    \
         --baud <RATE>    Baud rate (default: {DEFAULT_BAUD})\n    \
         --list-ports     List available serial ports and exit\n    \
         -h, --help       Show this help and exit"
    );
}
