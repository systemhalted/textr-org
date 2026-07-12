//! The text model — gedit's `GeditDocument` (a `TeplBuffer`/`GtkSourceBuffer`).
//!
//! Holds the buffer contents and editor-relevant state (modified flag, the
//! file it came from). Knows nothing about how it is displayed.

use std::path::{Path, PathBuf};

use ropey::Rope;

/// Errors arising from document I/O.
#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    /// An underlying filesystem error (file not found, permission denied, …).
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// `save` was called on a document that has no associated file yet.
    /// (gedit analog: an "Untitled" buffer must be given a path via *Save As*
    /// before plain *Save* can work.)
    #[error("document has no associated file; use save_as")]
    NoPath,
}

/// An open text buffer. The model half of gedit's document/view split.
pub struct Document {
    /// The text contents, stored as a rope for efficient edits on large files
    /// (gedit delegates this to `GtkTextBuffer`).
    rope: Rope,
    /// Whether the buffer has unsaved changes (gedit: the buffer's modified bit).
    modified: bool,
    /// The file this buffer is associated with, if any. `None` for a buffer that
    /// has never been loaded from or saved to disk (gedit's "Untitled" document).
    path: Option<PathBuf>,
}

impl Document {
    /// Create a new, empty, unmodified document with no associated file.
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            modified: false,
            path: None,
        }
    }

    /// Create a document pre-populated with `text`. Treated as freshly loaded
    /// content, so the document is *not* marked modified and has no file path.
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            modified: false,
            path: None,
        }
    }

    /// The full buffer contents as a string.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Number of lines. An empty buffer counts as one (empty) line.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Total number of characters in the buffer. This is the one-past-the-end
    /// char index, so a cursor at `char_count()` sits at the very end of the
    /// buffer with nothing to its right (used by the view to make a forward
    /// delete at end-of-buffer a no-op).
    pub fn char_count(&self) -> usize {
        self.rope.len_chars()
    }

    /// Char index of the first character of `line`
    pub fn line_to_char(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    /// Return the line number of the char at `char_idx`
    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx)
    }

    pub fn line_text(&self, line: usize) -> String {
        if line < self.line_count() {
            self.rope.line(line).to_string()
        } else {
            String::new()
        }
    }

    pub fn line_len_chars(&self, line: usize) -> usize {
        if line < self.line_count() {
            let slice = self.rope.line(line);
            let len = slice.len_chars();
            if len > 0 && slice.char(len - 1) == '\n' {
                len - 1
            } else {
                len
            }
        } else {
            0
        }
    }

    /// Whether there are unsaved changes.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// The file this document is associated with, or `None` if it has never
    /// been loaded from or saved to disk (gedit's "Untitled" buffer).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn insert(&mut self, char_idx: usize, text: &str) {
        self.rope.insert(char_idx, text);
        self.modified = true;
    }

    pub fn remove(&mut self, char_range: std::ops::Range<usize>) {
        self.rope.remove(char_range);
        self.modified = true;
    }

    /// Load `path` into a new document, remembering where it came from.
    pub fn open(path: &Path) -> Result<Document, DocumentError> {
        let text = std::fs::read_to_string(path)?;
        Ok(Self {
            rope: Rope::from_str(&text),
            modified: false,
            path: Some(path.to_path_buf()),
        })
    }

    /// Save to this document's associated file. Errors with
    /// [`DocumentError::NoPath`] if the document has never been associated with
    /// a file — use [`Document::save_as`] for that.
    pub fn save(&mut self) -> Result<(), DocumentError> {
        let path = self.path.clone().ok_or(DocumentError::NoPath)?;
        self.write_to(&path)
    }

    /// Save to `path` and make it this document's associated file from now on.
    /// On failure the document keeps its previous association and dirty state.
    pub fn save_as(&mut self, path: &Path) -> Result<(), DocumentError> {
        self.write_to(path)?;
        self.path = Some(path.to_path_buf());
        Ok(())
    }

    /// Stream the rope to `path`, then clear the modified flag. Shared by
    /// [`Document::save`] and [`Document::save_as`]. Streaming (rather than
    /// collecting the whole buffer into a `String` first) keeps saves cheap on
    /// large files — the point of using a rope. The modified flag is cleared
    /// only after a fully successful write, so a failed save stays dirty.
    fn write_to(&mut self, path: &Path) -> Result<(), DocumentError> {
        let file = std::fs::File::create(path)?;
        self.rope.write_to(std::io::BufWriter::new(file))?;
        self.modified = false;
        Ok(())
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp path unique to this process, so two concurrent `cargo test`
    /// invocations can't collide on the same fixed filename.
    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("textr_{}_{}.txt", name, std::process::id()))
    }

    #[test]
    fn new_document_is_empty_unmodified_and_pathless() {
        let doc = Document::new();
        assert_eq!(doc.text(), "");
        assert_eq!(doc.line_count(), 1); // an empty buffer is still one (empty) line
        assert!(!doc.is_modified());
        assert!(doc.path().is_none());
    }

    #[test]
    fn from_text_reports_contents_and_lines() {
        let doc = Document::from_text("alpha\nbeta\ngamma\n");
        assert_eq!(doc.text(), "alpha\nbeta\ngamma\n");
        assert_eq!(doc.line_count(), 4); // three lines + trailing empty line
        assert!(!doc.is_modified()); // loaded content is not a user edit
        assert!(doc.path().is_none()); // an in-memory buffer is tied to no file
    }

    #[test]
    fn insert_changes_text_and_marks_modified() {
        let mut doc = Document::from_text("hello");
        doc.insert(5, " world");
        assert_eq!(doc.text(), "hello world");
        assert!(doc.is_modified());
    }

    #[test]
    fn remove_deletes_range_and_marks_modified() {
        let mut doc = Document::from_text("hello world");
        doc.remove(5..11); // delete " world" — char indices 5 up to (not incl.) 11
        assert_eq!(doc.text(), "hello");
        assert!(doc.is_modified());
    }

    #[test]
    fn open_loads_contents_and_records_path() {
        let path = temp_path("open");
        std::fs::write(&path, "line1\nline2\n").unwrap();

        let doc = Document::open(&path).unwrap();

        assert_eq!(doc.text(), "line1\nline2\n");
        assert!(!doc.is_modified()); // a freshly-loaded file is not "modified"
        assert_eq!(doc.path(), Some(path.as_path())); // it remembers where it came from

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_writes_to_remembered_path_and_clears_modified() {
        let path = temp_path("save_remembered");
        std::fs::write(&path, "original").unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.insert(8, "!"); // "original!", modified == true

        doc.save().unwrap(); // no path argument — uses the remembered one

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original!");
        assert!(!doc.is_modified()); // a successful save clears the dirty flag

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_without_a_path_errors_and_keeps_modified() {
        let mut doc = Document::from_text("untitled buffer");
        doc.insert(0, "!"); // doc is modified, but tied to no file

        let result = doc.save();

        assert!(matches!(result, Err(DocumentError::NoPath)));
        assert!(doc.is_modified()); // nothing was saved, so it stays dirty
    }

    #[test]
    fn save_as_writes_file_and_remembers_path() {
        let path = temp_path("save_as");
        let _ = std::fs::remove_file(&path); // ensure a clean slate
        let mut doc = Document::from_text("brand new");
        assert!(doc.path().is_none());

        doc.save_as(&path).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "brand new");
        assert!(!doc.is_modified());
        assert_eq!(doc.path(), Some(path.as_path())); // save_as adopts the path

        // and a subsequent plain save now works against the remembered path
        doc.insert(9, "!");
        doc.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "brand new!");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn failed_save_as_surfaces_error_and_keeps_modified() {
        let mut doc = Document::from_text("important data");
        doc.insert(0, "!"); // doc is now modified

        // Parent directory does not exist, so the OS write must fail.
        let bad_path = std::env::temp_dir()
            .join("textr_no_such_dir_xyz")
            .join("file.txt");
        let result = doc.save_as(&bad_path);

        assert!(result.is_err()); // the failure must reach the caller, not be swallowed
        assert!(doc.is_modified()); // and the buffer must still be "dirty"
        assert!(doc.path().is_none()); // a failed save_as must not adopt the path
    }

    // --- Read-only line/char accessors -------------------------------------
    // These wrap ropey so the frontend (the View) never touches the rope
    // directly. Edit positions everywhere in textr are CHAR indices, not bytes.

    #[test]
    fn line_len_chars_excludes_the_trailing_newline() {
        let doc = Document::from_text("ab\ncd\n");
        assert_eq!(doc.line_len_chars(0), 2); // "ab" — the '\n' is NOT counted
        assert_eq!(doc.line_len_chars(1), 2); // "cd"
        assert_eq!(doc.line_len_chars(2), 0); // ropey's phantom final empty line
    }

    #[test]
    fn line_len_chars_counts_a_final_line_with_no_newline() {
        let doc = Document::from_text("ab\ncd"); // no trailing '\n'
        assert_eq!(doc.line_len_chars(1), 2); // "cd" is the last line, full length
    }

    #[test]
    fn line_len_chars_is_zero_for_an_out_of_range_line() {
        let doc = Document::from_text("ab\n");
        assert_eq!(doc.line_len_chars(99), 0); // don't panic on a bad index
    }

    #[test]
    fn line_to_char_returns_each_lines_starting_char_index() {
        let doc = Document::from_text("ab\ncd\n");
        assert_eq!(doc.line_to_char(0), 0);
        assert_eq!(doc.line_to_char(1), 3); // just past "ab\n"
        assert_eq!(doc.line_to_char(2), 6); // just past "ab\ncd\n"
    }

    #[test]
    fn char_to_line_maps_an_index_back_to_its_line() {
        let doc = Document::from_text("ab\ncd\n");
        assert_eq!(doc.char_to_line(0), 0);
        assert_eq!(doc.char_to_line(3), 1);
        assert_eq!(doc.char_to_line(6), 2);
    }

    #[test]
    fn line_text_includes_the_trailing_newline() {
        let doc = Document::from_text("ab\ncd\n");
        assert_eq!(doc.line_text(0), "ab\n");
        assert_eq!(doc.line_text(2), ""); // the phantom final line is empty
    }

    #[test]
    fn line_text_is_empty_for_an_out_of_range_line() {
        let doc = Document::from_text("ab\ncd\n"); // 3 lines: valid indices 0, 1, 2
        assert_eq!(doc.line_text(3), ""); // exactly one past the end — must not panic
        assert_eq!(doc.line_text(99), "");
    }

    #[test]
    fn empty_buffer_has_one_zero_length_line() {
        let doc = Document::new();
        assert_eq!(doc.line_count(), 1);
        assert_eq!(doc.line_len_chars(0), 0);
        assert_eq!(doc.line_to_char(0), 0);
    }

    #[test]
    fn char_count_is_the_one_past_the_end_index() {
        assert_eq!(Document::new().char_count(), 0);
        assert_eq!(Document::from_text("ab\ncd\n").char_count(), 6);
        assert_eq!(Document::from_text("é").char_count(), 1); // one CHAR, not two bytes
    }
}
