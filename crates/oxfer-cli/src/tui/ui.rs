//! Rendering. Reads the [`App`] state, never mutates it.

use std::time::Duration;

use oxfer_core::Entry;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Gauge, Paragraph, Row, Table, TableState, Wrap,
};
use ratatui::Frame;

use super::app::{App, InputKind, Pane, Popup, TransferState};
use crate::ops::format_time;

const HELP: &str = "\
Tab          switch pane          Up/Down j/k  move
PgUp/PgDn    page                 Enter/l      open directory
Backspace/h  parent directory     Space        select, a select all
F5 / c       copy to other pane   F7 / m       make directory
F8 / d       delete               r            rename
/            filter (Esc clears)  s            cycle sort
.            toggle hidden files  p            pause/resume transfer
Esc          cancel transfer      F10 / q      quit
?            this help";

pub fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n}B")
    } else {
        format!("{v:.1}{}", UNITS[u])
    }
}

pub fn human_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}h{:02}m", s / 3600, s % 3600 / 60)
    } else {
        format!("{}m{:02}s", s / 60, s % 60)
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let panel = app.transfer.is_some();
    let [panes, transfer, status] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(if panel { 8 } else { 0 }),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(panes);
    draw_pane(f, left, &app.panes[0], app.active == 0);
    draw_pane(f, right, &app.panes[1], app.active == 1);
    if let Some(t) = &app.transfer {
        draw_transfer(f, transfer, t);
    }
    draw_status(f, status, app);
    match &app.popup {
        Some(Popup::Help) => popup(f, "Help", HELP, 70),
        Some(Popup::Confirm { paths }) => {
            let names: Vec<_> = paths.iter().take(5).filter_map(|p| p.file_name()).collect();
            let more = paths.len().saturating_sub(5);
            let mut body = format!("Delete {} item(s)?\n\n{}", paths.len(), names.join("\n"));
            if more > 0 {
                body.push_str(&format!("\n... and {more} more"));
            }
            popup(f, "Confirm delete (y/n)", &body, 50);
        }
        Some(Popup::Input { kind, text }) => {
            let title = match kind {
                InputKind::Mkdir => "New directory",
                InputKind::Rename(_) => "Rename",
            };
            popup(f, title, &format!("{text}_"), 50);
        }
        None => {}
    }
}

fn entry_row<'a>(e: &Entry, selected: bool) -> Row<'a> {
    let (name, size) = if e.is_dir() {
        (format!("{}/", e.name()), "<DIR>".to_string())
    } else {
        (e.name().to_string(), human_size(e.size))
    };
    let when = e
        .modified
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or("-".into(), |d| format_time(d.as_secs() as i64));
    let style = if selected {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if e.is_dir() {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new()
    };
    let mark = if selected { "*" } else { " " };
    Row::new([
        Cell::from(format!("{mark}{name}")),
        Cell::from(Line::from(size).alignment(Alignment::Right)),
        Cell::from(when),
    ])
    .style(style)
}

fn draw_pane(f: &mut Frame, area: Rect, pane: &Pane, active: bool) {
    let mut title = format!(" {} {} [{}", pane.label, pane.cwd, pane.sort.label());
    if pane.show_hidden {
        title.push_str(" +hidden");
    }
    if !pane.filter.is_empty() {
        title.push_str(&format!(" /{}", pane.filter));
    }
    title.push_str("] ");
    let border = if active {
        Style::new().fg(Color::Green)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let view = pane.view();
    let rows = view
        .iter()
        .map(|e| entry_row(e, pane.selected.contains(e.name())));
    let hl = if active {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().add_modifier(Modifier::UNDERLINED)
    };
    let table = Table::new(
        rows,
        [
            Constraint::Min(8),
            Constraint::Length(8),
            Constraint::Length(16),
        ],
    )
    .header(Row::new(["Name", "Size", "Modified"]).style(Style::new().add_modifier(Modifier::BOLD)))
    .row_highlight_style(hl)
    .block(
        Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border)
            .title(title),
    );
    let mut state = TableState::default().with_selected(Some(pane.cursor));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_transfer(f: &mut Frame, area: Rect, t: &TransferState) {
    let state = if !t.running() {
        "finished"
    } else if t.control.is_paused() {
        "PAUSED"
    } else {
        "running"
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" Transfer ({state}) "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [gauge, info, current, log] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);
    f.render_widget(
        Gauge::default()
            .ratio(t.ratio())
            .gauge_style(Style::new().fg(Color::Green))
            .label(format!("{:.0}%", t.ratio() * 100.0)),
        gauge,
    );
    let eta = t.eta().map_or("-".into(), human_duration);
    f.render_widget(
        Paragraph::new(format!(
            "{}/{} files  {} / {}  {}/s  ETA {eta}",
            t.files_done,
            t.files,
            human_size(t.done_bytes()),
            human_size(t.bytes),
            human_size(t.speed() as u64),
        )),
        info,
    );
    let names: Vec<_> = t.active.values().map(String::as_str).collect();
    let cur = t.summary.clone().unwrap_or_else(|| names.join(", "));
    f.render_widget(Paragraph::new(format!("Current: {cur}")), current);
    let skip = t.log.len().saturating_sub(log.height as usize);
    let lines: Vec<_> = t.log[skip..]
        .iter()
        .map(|l| Line::styled(l.as_str(), Style::new().fg(Color::Red)))
        .collect();
    f.render_widget(Paragraph::new(lines), log);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let line = if app.filtering {
        Line::from(format!("/{}_", app.pane().filter))
    } else if app.status_err {
        Line::styled(app.status.as_str(), Style::new().fg(Color::Red))
    } else {
        Line::from(vec![
            Span::raw(app.status.as_str()),
            Span::styled(
                "  | Tab pane  F5 copy  F7 mkdir  F8 del  r rename  / filter  ? help  q quit",
                Style::new().fg(Color::DarkGray),
            ),
        ])
    };
    f.render_widget(Paragraph::new(line), area);
}

fn popup(f: &mut Frame, title: &str, body: &str, width: u16) {
    let area = f.area();
    let w = width.min(area.width);
    let h = (body.lines().count() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(body.to_string())
            .wrap(Wrap { trim: false })
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .title(format!(" {title} ")),
            ),
        rect,
    );
}
