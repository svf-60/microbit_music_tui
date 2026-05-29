//! Rendering. The TUI is a status view: a song list on the left, live playback
//! state on the right, and a serial log along the bottom.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::App;
use crate::serial::ConnectionState;

pub fn draw(f: &mut Frame, app: &App) {
    let rows = Layout::vertical([
        Constraint::Length(4), // header
        Constraint::Min(5),    // body
        Constraint::Length(8), // serial log
        Constraint::Length(1), // key hints
    ])
    .split(f.area());

    draw_header(f, app, rows[0]);

    let body =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(rows[1]);

    draw_song_list(f, app, body[0]);
    draw_now_playing(f, app, body[1]);

    draw_log(f, app, rows[2]);
    draw_hints(f, app, rows[3]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let color = match app.conn_state {
        ConnectionState::NoPort => Color::DarkGray,
        ConnectionState::Connecting => Color::Yellow,
        ConnectionState::Ready => Color::Green,
        ConnectionState::Disconnected => Color::Red,
    };
    let port = app
        .conn
        .as_ref()
        .map(|c| c.port_name.clone())
        .unwrap_or_else(|| "-".to_string());

    let top = Line::from(vec![
        Span::styled(" micro:bit music streamer ", title_style()),
        Span::raw("  port: "),
        Span::styled(port, bold(Color::White)),
        Span::raw("  status: "),
        Span::styled(app.conn_state.label(), bold(color)),
    ]);
    let bottom = Line::styled(app.status_msg.clone(), Style::new().fg(Color::Gray));

    let para = Paragraph::new(vec![top, bottom]).block(Block::bordered());
    f.render_widget(para, area);
}

fn draw_song_list(f: &mut Frame, app: &App, area: Rect) {
    let playing_idx = app.playback.as_ref().map(|pb| pb.song_index);
    let items: Vec<ListItem> = app
        .songs
        .iter()
        .enumerate()
        .map(|(i, song)| {
            let marker = if playing_idx == Some(i) { "* " } else { "  " };
            ListItem::new(format!("{marker}{}", song.name))
        })
        .collect();

    let list = List::new(items)
        .block(Block::bordered().title(format!(" Songs [{}] ", app.songs.len())))
        .highlight_style(
            Style::new()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = ListState::default();
    if !app.songs.is_empty() {
        state.select(Some(app.selected));
    }
    f.render_stateful_widget(list, area, &mut state);
}

/// The right pane. Three states: streaming (live progress), idle preview of the
/// selected song (the offline "display" mode), or an empty-library message.
fn draw_now_playing(f: &mut Frame, app: &App, area: Rect) {
    let title = if app.playback.is_some() {
        " Now Playing "
    } else if app.is_offline() {
        " Song Preview (offline) "
    } else {
        " Song Preview "
    };
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.songs.is_empty() {
        let msg = Paragraph::new(vec![
            Line::styled("No songs loaded.", Style::new().fg(Color::Gray)),
            Line::raw(format!(
                "Add WAV files under {} and press r.",
                app.dir.display()
            )),
        ])
        .wrap(Wrap { trim: true });
        f.render_widget(msg, inner);
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(4), // info
        Constraint::Length(1), // gauge (playing only)
        Constraint::Min(0),    // PCM status (playing only)
    ])
    .split(inner);

    match app.playback.as_ref() {
        Some(pb) => {
            let song = &app.songs[pb.song_index];
            let state = if pb.paused { "paused" } else { "playing" };
            let info = vec![
                line_kv("Song:  ", &song.name),
                Line::from(format!(
                    "State: {state}   {} / {}",
                    fmt_time(pb.position_secs()),
                    fmt_time(pb.duration_secs())
                )),
                Line::from(format!("WAV:   {} Hz, 8-bit mono", pb.rate)),
            ];
            f.render_widget(Paragraph::new(info).wrap(Wrap { trim: true }), rows[0]);
            render_gauge(f, pb.progress(), rows[1]);
            let note_text = if pb.paused {
                "Paused."
            } else {
                "Streaming raw PCM to the micro:bit speaker…"
            };
            let note = Paragraph::new(Line::styled(note_text, Style::new().fg(Color::Gray)))
                .block(Block::bordered().title(" PCM "))
                .wrap(Wrap { trim: true });
            f.render_widget(note, rows[2]);
        }
        None => {
            let Some(song) = app.selected_song() else {
                return;
            };
            let hint = if app.is_offline() {
                "No micro:bit connected — browsing only."
            } else {
                "Press Enter to stream this WAV."
            };
            let info = vec![
                line_kv("Song:  ", &song.name),
                Line::from("Type:  WAV (PCM stream)"),
                Line::styled(hint, Style::new().fg(Color::Gray)),
            ];
            f.render_widget(Paragraph::new(info).wrap(Wrap { trim: true }), rows[0]);
            // rows[1] (gauge) and rows[2] left blank when idle.
        }
    }
}

fn line_kv(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(label.to_string()),
        Span::styled(value.to_string(), bold(Color::White)),
    ])
}

fn fmt_time(secs: usize) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn render_gauge(f: &mut Frame, ratio: f64, area: Rect) {
    let gauge = Gauge::default()
        .gauge_style(Style::new().fg(Color::Cyan).bg(Color::Black))
        .ratio(ratio.clamp(0.0, 1.0))
        .label(format!("{:.0}%", ratio * 100.0));
    f.render_widget(gauge, area);
}

fn draw_log(f: &mut Frame, app: &App, area: Rect) {
    let visible = area.height.saturating_sub(2) as usize; // account for borders
    let skip = app.log.len().saturating_sub(visible);
    let lines: Vec<Line> = app
        .log
        .iter()
        .skip(skip)
        .map(|entry| {
            let color = if entry.starts_with('>') {
                Color::Yellow
            } else if entry.starts_with("< E") || entry.starts_with('!') {
                Color::Red
            } else if entry.starts_with('<') {
                Color::Green
            } else {
                Color::Gray
            };
            Line::styled(entry.clone(), Style::new().fg(color))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Serial Log ")),
        area,
    );
}

fn draw_hints(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![key("Up/Dn"), Span::raw(" select  ")];
    if !app.is_offline() {
        spans.extend([
            key("Enter"),
            Span::raw(" play  "),
            key("s"),
            Span::raw(" stop  "),
        ]);
    }
    spans.extend([
        key("r"),
        Span::raw(" refresh  "),
        key("q"),
        Span::raw(" quit"),
    ]);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn key(label: &str) -> Span<'_> {
    Span::styled(
        format!(" {label} "),
        Style::new()
            .fg(Color::Black)
            .bg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    )
}

fn title_style() -> Style {
    Style::new()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn bold(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}
