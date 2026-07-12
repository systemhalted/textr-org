//! Rendering — a pure view of [`App`] into a ratatui `Frame`. No crossterm, no I/O: this
//! tier is driver-agnostic, so a future GUI could drive the same state through a different
//! backend. Everything here derives from `&App`; it never mutates.

use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Mode};

/// Draw the whole editor: the (fold-aware) text body, then the status line, then place the
/// real hardware cursor.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let status = Rect::new(area.x, area.y + body.height, area.width, 1);

    let cursor = draw_body(frame, app, body);
    draw_status(frame, app, status);
    place_cursor(frame, app, body, status, cursor);
}

/// Render the visible document lines, skipping any hidden inside a fold. Returns the cursor's
/// on-screen `(column, row)` within `body`, if the cursor line is visible.
fn draw_body(frame: &mut Frame, app: &App, body: Rect) -> Option<(u16, u16)> {
    let doc = app.document();
    let height = body.height as usize;
    let cursor_line = app.view().cursor_line();

    let mut lines: Vec<Line> = Vec::with_capacity(height);
    let mut cursor: Option<(u16, u16)> = None;
    let mut doc_line = app.scroll_top();

    while lines.len() < height && doc_line < doc.line_count() {
        if app.is_hidden(doc_line) {
            doc_line += 1;
            continue;
        }
        let mut text = doc.line_text(doc_line);
        while text.ends_with('\n') || text.ends_with('\r') {
            text.pop();
        }
        if app.is_folded_heading(doc_line) {
            text.push_str(" …"); // a collapsed subtree
        }
        if doc_line == cursor_line {
            cursor = Some((app.view().cursor_column() as u16, lines.len() as u16));
        }
        let style = if is_heading(app, doc_line) {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::styled(text, style));
        doc_line += 1;
    }

    frame.render_widget(Paragraph::new(lines), body);
    cursor
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let widget =
        Paragraph::new(status_text(app)).style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_widget(widget, area);
}

fn place_cursor(frame: &mut Frame, app: &App, body: Rect, status: Rect, cursor: Option<(u16, u16)>) {
    match app.mode() {
        Mode::Edit => {
            if let Some((col, row)) = cursor {
                frame.set_cursor_position(Position::new(body.x + col, body.y + row));
            }
        }
        Mode::SaveAs { input } => {
            let col = "Save as: ".len() + input.chars().count();
            frame.set_cursor_position(Position::new(status.x + col as u16, status.y));
        }
    }
}

fn is_heading(app: &App, line: usize) -> bool {
    app.outline().headings.iter().any(|h| h.line == line)
}

/// The status-line text: the Save-As prompt, or `name[*] — line:col` plus any transient message.
fn status_text(app: &App) -> String {
    if let Mode::SaveAs { input } = app.mode() {
        return format!("Save as: {input}");
    }
    let name = app
        .document()
        .path()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "[No Name]".to_string());
    let dirty = if app.document().is_modified() { "*" } else { "" };
    let line = app.view().cursor_line() + 1;
    let col = app.view().cursor_column() + 1;
    let mut text = format!(" {name}{dirty} — {line}:{col} ");
    if !app.status().is_empty() {
        text.push_str("  ");
        text.push_str(app.status());
    }
    text
}
