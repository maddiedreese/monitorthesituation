use std::{
    io::{self, IsTerminal, Stdout},
    time::Duration,
};

use anyhow::Result;
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    style::ResetColor,
    terminal::{
        Clear as ClearTerminal, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::{App, Feed},
    config::{Columns, Config, Renderer},
    media::SourceStatus,
    render::VideoWidget,
};

type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn run(config: Config) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!(
            "the interactive interface needs a terminal; run it directly instead of piping its output"
        );
    }
    let mut terminal = TerminalGuard::enter()?;
    let mut app = App::new(config);
    while app.running {
        app.update();
        terminal.terminal.draw(|frame| draw(frame, &app))?;
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    handle_key(&mut app, key.code, key.modifiers);
                }
                Event::Paste(value) if app.adding_source => {
                    app.source_input.push_str(&value);
                    app.source_error = None;
                }
                _ => {}
            }
        }
    }
    app.stop();
    Ok(())
}

struct TerminalGuard {
    terminal: Tui,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            ClearTerminal(ClearType::All),
            Hide
        ) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = disable_raw_mode();
                let mut stdout = io::stdout();
                let _ = execute!(
                    stdout,
                    ResetColor,
                    Show,
                    DisableBracketedPaste,
                    LeaveAlternateScreen
                );
                return Err(error.into());
            }
        };
        Ok(Self { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            ResetColor,
            Show,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        app.stop();
        return;
    }
    if app.adding_source {
        match code {
            KeyCode::Esc => app.cancel_source_entry(),
            KeyCode::Enter => app.submit_source(),
            KeyCode::Backspace => {
                app.source_input.pop();
                app.source_error = None;
            }
            KeyCode::Char(character) if !modifiers.contains(KeyModifiers::CONTROL) => {
                app.source_input.push(character);
                app.source_error = None;
            }
            _ => {}
        }
        return;
    }
    if code == KeyCode::Char('q') {
        app.stop();
        return;
    }
    if app.show_settings {
        match code {
            KeyCode::Char('s') | KeyCode::Esc => app.show_settings = false,
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right | KeyCode::Up => {
                app.increase_max_panes()
            }
            KeyCode::Char('-') | KeyCode::Left | KeyCode::Down => app.decrease_max_panes(),
            KeyCode::Char(']') | KeyCode::PageDown => app.next_page(),
            KeyCode::Char('[') | KeyCode::PageUp => app.previous_page(),
            _ => {}
        }
        return;
    }
    if app.show_help {
        match code {
            KeyCode::Char('a') => app.open_source_entry(),
            KeyCode::Char('?') | KeyCode::Esc => app.show_help = false,
            _ => {}
        }
        return;
    }
    match code {
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('a') => app.open_source_entry(),
        KeyCode::Char('x') => app.remove_selected(),
        KeyCode::Char('s') => app.open_settings(),
        KeyCode::Char(']') | KeyCode::PageDown => app.next_page(),
        KeyCode::Char('[') | KeyCode::PageUp => app.previous_page(),
        KeyCode::Char('i') => app.show_details = !app.show_details,
        KeyCode::Char(' ') => app.toggle_pause(),
        KeyCode::Char('c') => app.config.ui.color = !app.config.ui.color,
        KeyCode::Char('r') => app.toggle_renderer(),
        KeyCode::Tab | KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j') => {
            app.next()
        }
        KeyCode::BackTab
        | KeyCode::Left
        | KeyCode::Up
        | KeyCode::Char('h')
        | KeyCode::Char('k') => app.previous(),
        KeyCode::Char(number @ '1'..='9') => {
            let index = usize::from(number as u8 - b'1');
            if index < app.feeds.len() {
                app.selected = index;
            }
        }
        _ => {}
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(chrome()), area);
    if area.width < 30 || area.height < 8 {
        frame.render_widget(
            Paragraph::new("Terminal too small\nResize to at least 30 × 8")
                .alignment(Alignment::Center)
                .style(chrome()),
            area,
        );
        return;
    }
    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .split(area);
    draw_header(frame, app, vertical[0]);
    draw_grid(frame, app, vertical[1]);
    draw_footer(frame, app, vertical[2]);
    if app.show_help {
        draw_help(frame, area);
    }
    if app.show_details {
        draw_details(frame, app, area);
    }
    if app.adding_source {
        draw_source_entry(frame, app, area);
    }
    if app.show_settings {
        draw_settings(frame, app, area);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let renderer = match app.config.ui.renderer {
        Renderer::Ascii => "ASCII",
        Renderer::Blocks => "BLOCKS",
    };
    let color = if app.config.ui.color { "COLOR" } else { "MONO" };
    let page_size = usize::from(app.config.ui.max_panes);
    let page_count = app.feeds.len().max(1).div_ceil(page_size);
    let page = if app.feeds.is_empty() {
        1
    } else {
        app.selected / page_size + 1
    };
    let page_label = if page_count > 1 {
        format!("  ·  PAGE {page}/{page_count}")
    } else {
        String::new()
    };
    let line = Line::from(vec![
        Span::styled(
            " MONITOR THE SITUATION ",
            chrome().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {} FEED{}  ·  {renderer}  ·  {color}{page_label}",
            app.feeds.len(),
            if app.feeds.len() == 1 { "" } else { "S" }
        )),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_grid(frame: &mut Frame, app: &App, area: Rect) {
    if app.feeds.is_empty() {
        frame.render_widget(
            Paragraph::new("No feeds yet\n\nPress a to add a live stream, webcam, or video file")
                .alignment(Alignment::Center)
                .style(chrome()),
            area,
        );
        return;
    }
    let page_size = usize::from(app.config.ui.max_panes);
    let page_start = (app.selected / page_size) * page_size;
    let page_end = (page_start + page_size).min(app.feeds.len());
    let count = page_end - page_start;
    let columns = match app.config.ui.columns {
        Columns::Fixed(value) => usize::from(value).min(count).max(1),
        Columns::Auto => auto_columns(count),
    };
    let rows = count.div_ceil(columns);
    let row_constraints = vec![Constraint::Ratio(1, rows as u32); rows];
    let row_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(area);
    for (row, row_area) in row_areas.iter().enumerate() {
        let start = row * columns;
        let items = (count - start).min(columns);
        let constraints = vec![Constraint::Ratio(1, columns as u32); columns];
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(*row_area);
        for column in 0..items {
            let index = page_start + start + column;
            if let Some(feed) = app.feeds.get(index) {
                draw_feed(frame, app, feed, index, cells[column]);
            }
        }
    }
}

fn draw_feed(frame: &mut Frame, app: &App, feed: &Feed, index: usize, area: Rect) {
    let selected = index == app.selected;
    let marker = if feed.paused { " PAUSED" } else { "" };
    let display_name = if feed.automatic_name {
        feed.metadata
            .location
            .as_deref()
            .or(feed.metadata.title.as_deref())
            .unwrap_or(&feed.worker.name)
    } else {
        &feed.worker.name
    };
    let title = format!(" {}  {display_name}{marker} ", index + 1);
    let status = status_label(&feed.status, feed.measured_fps);
    let source_badge = source_badge(feed);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(chrome().add_modifier(if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        }))
        .title(Line::from(title).style(chrome()))
        .title_bottom(Line::from(format!(" {source_badge} ")).style(chrome()))
        .title_bottom(Line::from(format!(" {status} ")).alignment(Alignment::Right));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if let Some(video) = &feed.frame {
        frame.render_widget(
            VideoWidget {
                frame: video,
                renderer: app.config.ui.renderer,
                color: app.config.ui.color,
                ramp: &app.config.ui.ascii_ramp,
            },
            inner,
        );
    } else {
        let text = match &feed.status {
            SourceStatus::Failed(message) => format!("Waiting to reconnect…\n{message}"),
            _ => "Connecting…".into(),
        };
        frame.render_widget(
            Paragraph::new(text)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(chrome()),
            inner,
        );
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let selected = app
        .feeds
        .get(app.selected)
        .map(|feed| feed.worker.name.as_str())
        .unwrap_or("—");
    let line = Line::from(vec![
        Span::styled(
            format!(" {selected} "),
            chrome().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  a add  ·  x remove  ·  [ ] pages  ·  s settings  ·  i info  ·  q quit",
            chrome(),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let popup = centered(
        area,
        58.min(area.width.saturating_sub(4)),
        22.min(area.height.saturating_sub(2)),
    );
    frame.render_widget(Clear, popup);
    let help = [
        "KEYBOARD",
        "",
        "  Tab / arrows / hjkl   Select a feed",
        "  1–9                    Select feed by number",
        "  Space                  Pause selected feed",
        "  a                      Add a feed by URL or path",
        "  x                      Remove selected feed",
        "  [ / ]                  Previous / next page",
        "  s                      Visible-pane settings",
        "  r                      ASCII / block renderer",
        "  c                      Color / monochrome",
        "  i                      Feed details",
        "  ? or Esc               Close this help",
        "  q or Ctrl-C            Quit",
        "",
        "Streams are decoded locally. Nothing is uploaded or recorded.",
    ]
    .join("\n");
    frame.render_widget(
        Paragraph::new(help)
            .style(chrome())
            .block(
                Block::bordered()
                    .title(" HELP ")
                    .style(chrome())
                    .border_style(chrome()),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn draw_details(frame: &mut Frame, app: &App, area: Rect) {
    let Some(feed) = app.feeds.get(app.selected) else {
        return;
    };
    let popup = centered(
        area,
        62.min(area.width.saturating_sub(4)),
        18.min(area.height.saturating_sub(2)),
    );
    frame.render_widget(Clear, popup);
    let age = feed
        .frame
        .as_ref()
        .map(|value| format!("{:.1}s", value.received_at.elapsed().as_secs_f32()))
        .unwrap_or_else(|| "—".into());
    let sequence = feed
        .frame
        .as_ref()
        .map(|value| value.sequence.to_string())
        .unwrap_or_else(|| "—".into());
    let native_size = match (feed.metadata.width, feed.metadata.height) {
        (Some(width), Some(height)) => format!("{width} × {height}"),
        _ => "detecting…".into(),
    };
    let native_fps = feed
        .metadata
        .fps
        .map(|fps| format!("{fps:.2} fps"))
        .unwrap_or_else(|| "detecting…".into());
    let embedded_title = feed.metadata.title.as_deref().unwrap_or("—");
    let location = feed.metadata.location.as_deref().unwrap_or("—");
    let description = feed.metadata.description.as_deref().unwrap_or("—");
    let text = format!(
        "Name: {}\nEmbedded title: {}\nLocation: {}\nDescription: {}\nSource: {}\nKind: {:?}\nFormat: {}\nCodec: {}\nNative video: {} · {}\nStatus: {}\nFrames received: {}\nCurrent sequence: {}\nFrame age: {}\nRendered rate: {:.1} fps",
        feed.worker.name,
        embedded_title,
        location,
        description,
        feed.source.display_input(),
        feed.source.kind,
        feed.metadata.format.as_deref().unwrap_or("detecting…"),
        feed.metadata.codec.as_deref().unwrap_or("detecting…"),
        native_size,
        native_fps,
        status_label(&feed.status, feed.measured_fps),
        feed.frames_seen,
        sequence,
        age,
        feed.measured_fps,
    );
    frame.render_widget(
        Paragraph::new(text).style(chrome()).block(
            Block::bordered()
                .title(" FEED DETAILS · i to close ")
                .style(chrome())
                .border_style(chrome()),
        ),
        popup,
    );
}

fn draw_source_entry(frame: &mut Frame, app: &App, area: Rect) {
    let popup = centered(
        area,
        72.min(area.width.saturating_sub(4)),
        9.min(area.height.saturating_sub(2)),
    );
    frame.render_widget(Clear, popup);
    let max_chars = usize::from(popup.width.saturating_sub(6));
    let input = tail_chars(&app.source_input, max_chars);
    let error = app.source_error.as_deref().unwrap_or("");
    let text = vec![
        Line::from("Paste a URL or path, optionally prefixed with a location:"),
        Line::from("Location name | URL"),
        Line::from(vec![
            Span::styled("> ", chrome().add_modifier(Modifier::BOLD)),
            Span::styled(input, chrome()),
            Span::styled("▏", chrome().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(Span::styled(error, chrome())),
        Line::from(Span::styled(
            "Enter add  ·  Esc cancel  ·  YouTube pages require yt-dlp",
            chrome(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(text).style(chrome()).block(
            Block::bordered()
                .title(" ADD A FEED ")
                .style(chrome())
                .border_style(chrome()),
        ),
        popup,
    );
}

fn draw_settings(frame: &mut Frame, app: &App, area: Rect) {
    let popup = centered(
        area,
        58.min(area.width.saturating_sub(4)),
        11.min(area.height.saturating_sub(2)),
    );
    frame.render_widget(Clear, popup);
    let text = format!(
        "VISIBLE PANES\n\n  {} feeds per page\n\n  + / − or arrows        Change pane count (1–36)\n  [ / ]                  Move between pages\n  s or Esc               Close settings\n\nAdditional feeds remain active and appear on later pages.",
        app.config.ui.max_panes
    );
    frame.render_widget(
        Paragraph::new(text).style(chrome()).block(
            Block::bordered()
                .title(" SETTINGS ")
                .style(chrome())
                .border_style(chrome()),
        ),
        popup,
    );
}

fn tail_chars(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    if count <= limit {
        value.to_owned()
    } else {
        format!(
            "…{}",
            value
                .chars()
                .skip(count - limit.saturating_sub(1))
                .collect::<String>()
        )
    }
}

fn source_badge(feed: &Feed) -> String {
    let protocol = if feed.source.input.starts_with("camera://") {
        "CAMERA"
    } else if let Some((scheme, _)) = feed.source.input.split_once("://") {
        scheme
    } else {
        "FILE"
    };
    let metadata = feed.metadata.compact();
    if metadata.is_empty() {
        protocol.to_uppercase()
    } else {
        format!("{} · {metadata}", protocol.to_uppercase())
    }
}

fn chrome() -> Style {
    Style::default()
}

fn status_label(status: &SourceStatus, fps: f32) -> String {
    match status {
        SourceStatus::Connecting => "CONNECTING".into(),
        SourceStatus::Live => format!("LIVE  {fps:.1} FPS"),
        SourceStatus::Reconnecting => "RECONNECTING".into(),
        SourceStatus::Failed(_) => "OFFLINE".into(),
        SourceStatus::Stopped => "STOPPED".into(),
    }
}

fn auto_columns(count: usize) -> usize {
    (count as f32).sqrt().ceil() as usize
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_grid_is_bounded() {
        assert_eq!(auto_columns(1), 1);
        assert_eq!(auto_columns(4), 2);
        assert_eq!(auto_columns(6), 3);
        assert_eq!(auto_columns(9), 3);
    }

    #[test]
    fn centered_rectangle_stays_inside() {
        let result = centered(Rect::new(2, 3, 80, 20), 40, 10);
        assert_eq!(result, Rect::new(22, 8, 40, 10));
    }

    #[test]
    fn quit_is_available_while_help_is_open() {
        let mut app = App::new(Config::default());
        assert!(app.show_help);
        handle_key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!app.running);
    }

    #[test]
    fn vim_left_key_does_not_open_help() {
        let mut config = Config::default();
        config.ui.show_help = false;
        let mut app = App::new(config);
        handle_key(&mut app, KeyCode::Char('h'), KeyModifiers::NONE);
        assert!(!app.show_help);
    }

    #[test]
    fn add_feed_opens_directly_from_startup_help() {
        let mut app = App::new(Config::default());
        handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(app.adding_source);
        assert!(!app.show_help);
    }
}
