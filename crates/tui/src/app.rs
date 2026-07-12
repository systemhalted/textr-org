//! The editor state and all of its transitions — pure, terminal-free, and unit-tested.
//!
//! `App` owns the [`Document`] and [`View`] and drives them in response to key presses. It
//! knows nothing about ratatui or crossterm beyond the `KeyEvent` *data* type, so every
//! transition below is exercised in-process without a real terminal.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use textr_org_core::document::Document;
use textr_org_core::structure::{next_heading, prev_heading, Outline, OrgProvider, StructureProvider};
use textr_org_core::view::View;

use crate::action::{key_to_action, Action};
use crate::viewport::viewport_top;

/// What the editor is doing right now. In [`Mode::SaveAs`] the keyboard drives the bottom-line
/// path prompt instead of the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Normal editing.
    Edit,
    /// The *Save As* prompt is open; `input` is the path typed so far.
    SaveAs { input: String },
}

/// The whole editor: buffer, cursor, mode, fold state, and a derived outline cache.
pub struct App {
    doc: Document,
    view: View,
    mode: Mode,
    /// Heading start-lines that are currently collapsed.
    folded: HashSet<usize>,
    /// Outline cache, re-derived after every edit so fold ranges stay correct.
    outline: Outline,
    /// Top document line of the viewport (updated before each render).
    scroll_top: usize,
    /// Lines per page, kept in sync with the terminal body height for PageUp/PageDown.
    page: usize,
    /// A transient status-line message (save result, error), cleared on the next key.
    status: String,
    /// For a buffer opened on a not-yet-existing path: where the first save should go.
    stash_path: Option<PathBuf>,
    should_quit: bool,
}

impl App {
    /// Build an editor over `doc`. `stash_path` is `Some` when the file did not exist yet —
    /// the first save writes there without prompting.
    pub fn new(doc: Document, stash_path: Option<PathBuf>) -> Self {
        let outline = OrgProvider.parse(&doc);
        Self {
            doc,
            view: View::new(),
            mode: Mode::Edit,
            folded: HashSet::new(),
            outline,
            scroll_top: 0,
            page: 1,
            status: String::new(),
            stash_path,
            should_quit: false,
        }
    }

    // ---- read-only accessors for the renderer -----------------------------

    pub fn document(&self) -> &Document {
        &self.doc
    }
    pub fn view(&self) -> &View {
        &self.view
    }
    pub fn mode(&self) -> &Mode {
        &self.mode
    }
    pub fn outline(&self) -> &Outline {
        &self.outline
    }
    pub fn scroll_top(&self) -> usize {
        self.scroll_top
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Whether `line` is a collapsed heading (draw a fold marker on it).
    pub fn is_folded_heading(&self, line: usize) -> bool {
        self.folded.contains(&line)
    }

    /// Whether `line` is hidden inside some collapsed heading's subtree (skip it when drawing).
    pub fn is_hidden(&self, line: usize) -> bool {
        self.outline
            .headings
            .iter()
            .any(|h| self.folded.contains(&h.line) && line > h.line && line <= h.last_line)
    }

    // ---- driver seam ------------------------------------------------------

    /// Keep the page size in step with the terminal body height (for PageUp/PageDown).
    pub fn set_page(&mut self, page: usize) {
        self.page = page.max(1);
    }

    /// Recompute the viewport top so the cursor stays visible in a `body_height`-row body.
    pub fn update_scroll(&mut self, body_height: usize) {
        self.scroll_top = viewport_top(self.view.cursor_line(), self.scroll_top, body_height);
    }

    /// Handle one key press, dispatching by mode.
    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.mode {
            Mode::Edit => {
                self.status.clear();
                if let Some(action) = key_to_action(key) {
                    self.apply(action);
                }
            }
            Mode::SaveAs { .. } => self.handle_saveas_key(key),
        }
    }

    // ---- Edit-mode actions ------------------------------------------------

    fn apply(&mut self, action: Action) {
        match action {
            Action::MoveLeft => self.view.move_left(&self.doc),
            Action::MoveRight => self.view.move_right(&self.doc),
            Action::MoveUp => self.view.move_up(&self.doc),
            Action::MoveDown => self.view.move_down(&self.doc),
            Action::MoveHome => self.view.move_home(),
            Action::MoveEnd => self.view.move_end(&self.doc),
            Action::PageUp => self.view.move_page_up(&self.doc, self.page),
            Action::PageDown => self.view.move_page_down(&self.doc, self.page),
            Action::InsertChar(c) => self.edit(|v, d| v.insert_char(d, c)),
            Action::Newline => self.edit(|v, d| v.insert_newline(d)),
            Action::Backspace => self.edit(|v, d| v.backspace(d)),
            Action::Delete => self.edit(|v, d| v.delete(d)),
            Action::Save => self.save(),
            Action::Quit => self.should_quit = true,
            Action::ToggleFold => self.toggle_fold(),
            Action::NextHeading => {
                if let Some(line) = next_heading(&self.outline, self.view.cursor_line()) {
                    self.view.move_to_line(&self.doc, line);
                }
            }
            Action::PrevHeading => {
                if let Some(line) = prev_heading(&self.outline, self.view.cursor_line()) {
                    self.view.move_to_line(&self.doc, line);
                }
            }
            Action::CycleTodo => {
                OrgProvider.cycle_todo(&mut self.doc, self.view.cursor_line());
                self.reparse();
            }
        }
    }

    /// Run an editing closure on the view+document, then re-derive the outline.
    fn edit(&mut self, f: impl FnOnce(&mut View, &mut Document)) {
        f(&mut self.view, &mut self.doc);
        self.reparse();
    }

    /// `Tab`: fold/unfold when the caret sits on a heading, otherwise insert a tab.
    fn toggle_fold(&mut self) {
        let line = self.view.cursor_line();
        if self.outline.headings.iter().any(|h| h.line == line) {
            if !self.folded.remove(&line) {
                self.folded.insert(line);
            }
        } else {
            self.edit(|v, d| v.insert_char(d, '\t'));
        }
    }

    /// Re-parse the outline after an edit and drop folds whose heading line no longer exists.
    fn reparse(&mut self) {
        self.outline = OrgProvider.parse(&self.doc);
        let heading_lines: HashSet<usize> =
            self.outline.headings.iter().map(|h| h.line).collect();
        self.folded.retain(|line| heading_lines.contains(line));
    }

    // ---- saving -----------------------------------------------------------

    fn save(&mut self) {
        if self.doc.path().is_some() {
            match self.doc.save() {
                Ok(()) => self.status = "Saved".into(),
                Err(e) => self.status = format!("Save failed: {e}"),
            }
        } else if let Some(path) = self.stash_path.clone() {
            self.save_as(&path);
        } else {
            self.mode = Mode::SaveAs {
                input: String::new(),
            };
        }
    }

    fn save_as(&mut self, path: &Path) {
        match self.doc.save_as(path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.stash_path = None;
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    // ---- SaveAs-mode prompt -----------------------------------------------

    fn handle_saveas_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match key.code {
            KeyCode::Esc => self.mode = Mode::Edit,
            KeyCode::Enter => {
                if let Mode::SaveAs { input } = &self.mode {
                    let path = PathBuf::from(input.clone());
                    self.mode = Mode::Edit;
                    if !path.as_os_str().is_empty() {
                        self.save_as(&path);
                    }
                }
            }
            KeyCode::Backspace => {
                if let Mode::SaveAs { input } = &mut self.mode {
                    input.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Mode::SaveAs { input } = &mut self.mode {
                    input.push(c);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    // Small helpers to drive the app the way the event loop does.
    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }
    fn ctrl(app: &mut App, c: char) {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }
    fn typ(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("textr_app_{}_{}.org", name, std::process::id()))
    }

    #[test]
    fn ctrl_s_with_a_path_saves_and_clears_modified() {
        let path = temp_path("save");
        std::fs::write(&path, "* a\n").unwrap();
        let mut app = App::new(Document::open(&path).unwrap(), None);
        typ(&mut app, "x"); // modify
        assert!(app.document().is_modified());

        ctrl(&mut app, 's');

        assert!(!app.document().is_modified());
        assert_eq!(app.status(), "Saved");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ctrl_s_without_a_path_opens_the_saveas_prompt() {
        let mut app = App::new(Document::from_text("hello"), None);
        ctrl(&mut app, 's');
        assert!(matches!(app.mode(), Mode::SaveAs { .. }));
    }

    #[test]
    fn saveas_prompt_types_writes_on_enter_and_returns_to_edit() {
        let path = temp_path("saveas");
        let _ = std::fs::remove_file(&path);
        let mut app = App::new(Document::from_text("brand new"), None);

        ctrl(&mut app, 's'); // open prompt
        typ(&mut app, path.to_str().unwrap());
        typ(&mut app, "z"); // a stray char...
        press(&mut app, KeyCode::Backspace); // ...that Backspace removes → path restored
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.mode(), &Mode::Edit);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "brand new");
        assert!(!app.document().is_modified()); // a successful save clears the dirty flag
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saveas_esc_cancels_and_leaves_the_buffer_unsaved() {
        let mut app = App::new(Document::from_text("data"), None);
        typ(&mut app, "!"); // now modified
        ctrl(&mut app, 's'); // prompt
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode(), &Mode::Edit);
        assert!(app.document().is_modified()); // nothing was written
    }

    #[test]
    fn missing_file_buffer_saves_to_the_stashed_path_without_prompting() {
        let path = temp_path("stash");
        let _ = std::fs::remove_file(&path);
        let mut app = App::new(Document::new(), Some(path.clone()));
        typ(&mut app, "* hi\n");

        ctrl(&mut app, 's'); // should save_as(stash), NOT open a prompt

        assert_eq!(app.mode(), &Mode::Edit);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "* hi\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tab_on_a_heading_toggles_a_fold_and_hides_its_subtree() {
        let mut app = App::new(Document::from_text("* A\nbody\n* B\n"), None);
        // caret at (0,0), on heading A
        press(&mut app, KeyCode::Tab);
        assert!(app.is_folded_heading(0));
        assert!(app.is_hidden(1)); // "body" is inside A's subtree
        assert!(!app.is_hidden(2)); // heading B is not
        press(&mut app, KeyCode::Tab); // unfold
        assert!(!app.is_folded_heading(0));
        assert!(!app.is_hidden(1));
    }

    #[test]
    fn tab_off_a_heading_inserts_a_tab() {
        let mut app = App::new(Document::from_text("plain\n"), None);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.document().text(), "\tplain\n");
    }

    #[test]
    fn ctrl_n_and_ctrl_p_jump_between_headings() {
        let mut app = App::new(Document::from_text("* A\nx\n* B\ny\n* C\n"), None);
        ctrl(&mut app, 'n'); // A → B (line 2)
        assert_eq!(app.view().cursor_line(), 2);
        ctrl(&mut app, 'n'); // B → C (line 4)
        assert_eq!(app.view().cursor_line(), 4);
        ctrl(&mut app, 'p'); // C → B
        assert_eq!(app.view().cursor_line(), 2);
    }

    #[test]
    fn ctrl_t_cycles_the_heading_todo_keyword() {
        let mut app = App::new(Document::from_text("* task\n"), None);
        ctrl(&mut app, 't');
        assert_eq!(app.document().text(), "* TODO task\n");
        ctrl(&mut app, 't');
        assert_eq!(app.document().text(), "* DONE task\n");
    }

    #[test]
    fn editing_reparses_so_a_new_heading_is_recognized() {
        let mut app = App::new(Document::from_text("plain\n"), None);
        assert!(app.outline().headings.is_empty());
        press(&mut app, KeyCode::Home);
        typ(&mut app, "* "); // turn the line into a heading
        assert_eq!(app.outline().headings.len(), 1);
    }
}
