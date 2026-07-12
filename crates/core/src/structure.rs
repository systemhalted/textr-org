//! The format-agnostic outline layer — the spine of textr's Org-mode–style structure
//! editing (see `docs/roadmap.md`).
//!
//! A buffer parses into an [`Outline`]: a flat, level-tagged list of [`Heading`]s, each
//! carrying the extent of its subtree so the frontend can fold it. **Format knowledge lives
//! behind the [`StructureProvider`] trait**, so every capability built on top — folding now;
//! promote/demote, agenda, and export later — is written once against the trait and works for
//! every format. [`OrgProvider`] is the first implementer (Org `*` headings); a Markdown
//! provider follows in a later milestone.

use crate::document::Document;

/// A TODO workflow keyword on a heading. M2 ships the two canonical states; custom keyword
/// sets, priorities, and tags come later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoState {
    /// An open item (`TODO`).
    Todo,
    /// A completed item (`DONE`).
    Done,
}

/// One heading in a parsed document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// Nesting depth, 1-based: Org `* ` = 1, `** ` = 2.
    pub level: usize,
    /// The line the heading sits on (0-based).
    pub line: usize,
    /// The heading text, with the leading markers and any TODO keyword stripped.
    pub title: String,
    /// The heading's TODO state, if it carries a keyword.
    pub todo: Option<TodoState>,
    /// The last line of this heading's subtree. The foldable body is the (possibly empty)
    /// range `line + 1 ..= last_line`.
    pub last_line: usize,
}

/// A document's headings in document order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    /// The headings, top to bottom.
    pub headings: Vec<Heading>,
}

impl Outline {
    /// The heading the caret is "inside": the last heading at or above `line`, if any.
    fn enclosing_index(&self, line: usize) -> Option<usize> {
        self.headings.iter().rposition(|h| h.line <= line)
    }
}

/// A parser + editor of a particular document format's structure. One implementer per
/// format (Org today; Markdown next). Everything else in textr talks to structure through
/// this trait, never to a concrete format.
pub trait StructureProvider {
    /// Scan `doc` into an [`Outline`].
    fn parse(&self, doc: &Document) -> Outline;

    /// Cycle the TODO keyword on the heading at `line`: none → `TODO` → `DONE` → none.
    /// A no-op if that line is not a heading.
    fn cycle_todo(&self, doc: &mut Document, line: usize);
}

// ---- navigation (pure free functions over an Outline) ---------------------

/// The line of the first heading strictly below `line`, if any.
pub fn next_heading(outline: &Outline, line: usize) -> Option<usize> {
    outline.headings.iter().find(|h| h.line > line).map(|h| h.line)
}

/// The line of the last heading strictly above `line`, if any.
pub fn prev_heading(outline: &Outline, line: usize) -> Option<usize> {
    outline
        .headings
        .iter()
        .rev()
        .find(|h| h.line < line)
        .map(|h| h.line)
}

/// The line of the parent of the heading enclosing `line` — the nearest preceding heading
/// of a smaller level. `None` at the top level or outside any heading.
pub fn parent_heading(outline: &Outline, line: usize) -> Option<usize> {
    let idx = outline.enclosing_index(line)?;
    let level = outline.headings[idx].level;
    outline.headings[..idx]
        .iter()
        .rev()
        .find(|h| h.level < level)
        .map(|h| h.line)
}

// ---- Org format -----------------------------------------------------------

/// The Org-syntax structure provider: headings are `*`-prefixed lines
/// (`^(\*+) +(?:(TODO|DONE) +)?title`).
pub struct OrgProvider;

impl StructureProvider for OrgProvider {
    fn parse(&self, doc: &Document) -> Outline {
        let mut headings: Vec<Heading> = (0..doc.line_count())
            .filter_map(|line| parse_org_heading(&doc.line_text(line), line))
            .collect();

        // Fill in each heading's subtree extent now that we know its successors: a subtree
        // runs until the next heading of equal-or-shallower level, else to end of buffer.
        let last_doc_line = doc.line_count().saturating_sub(1);
        for i in 0..headings.len() {
            let level = headings[i].level;
            let end = headings[i + 1..]
                .iter()
                .find(|h| h.level <= level)
                .map(|h| h.line - 1)
                .unwrap_or(last_doc_line);
            headings[i].last_line = end;
        }
        Outline { headings }
    }

    fn cycle_todo(&self, doc: &mut Document, line: usize) {
        let raw = doc.line_text(line);
        let Some(heading) = parse_org_heading(&raw, line) else {
            return; // not a heading — leave the buffer untouched
        };
        // The keyword lives right after the stars and their trailing space(s). Stars and
        // spaces are ASCII, so their byte counts equal their char counts.
        let text = raw.strip_suffix('\n').unwrap_or(&raw);
        let after_stars = &text[heading.level..];
        let spaces = after_stars.len() - after_stars.trim_start().len();
        let rest_start = doc.line_to_char(line) + heading.level + spaces;
        let rest = after_stars.trim_start();

        match heading.todo {
            None => doc.insert(rest_start, "TODO "),
            Some(TodoState::Todo) => {
                doc.remove(rest_start..rest_start + "TODO".len());
                doc.insert(rest_start, "DONE");
            }
            Some(TodoState::Done) => {
                // Drop "DONE" plus the one space before the title, if there is a title.
                let len = if rest == "DONE" { 4 } else { 5 };
                doc.remove(rest_start..rest_start + len);
            }
        }
    }
}

/// Parse a single raw line (newline included) into a heading, or `None` if it isn't one.
/// `last_line` is left as `line` and filled in later by the caller.
fn parse_org_heading(raw: &str, line: usize) -> Option<Heading> {
    let text = raw.strip_suffix('\n').unwrap_or(raw);
    let level = text.bytes().take_while(|&b| b == b'*').count();
    if level == 0 {
        return None; // no stars, or a '*' not at column 0
    }
    let after = &text[level..];
    if !after.starts_with(' ') {
        return None; // "*bold" — stars must be followed by a space
    }
    let rest = after.trim_start();
    if rest.is_empty() {
        return None; // "* " with no title is body, not a heading
    }
    let (todo, title) = split_todo_keyword(rest);
    Some(Heading {
        level,
        line,
        title: title.to_string(),
        todo,
        last_line: line,
    })
}

/// Split a leading `TODO`/`DONE` keyword off the heading text. The keyword only counts as
/// the first whole word — `TODOitem` is a plain title.
fn split_todo_keyword(rest: &str) -> (Option<TodoState>, &str) {
    for (state, word) in [(TodoState::Todo, "TODO"), (TodoState::Done, "DONE")] {
        if let Some(tail) = rest.strip_prefix(word) {
            if tail.is_empty() || tail.starts_with(' ') {
                return (Some(state), tail.trim_start());
            }
        }
    }
    (None, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(text: &str) -> Outline {
        OrgProvider.parse(&Document::from_text(text))
    }

    // ---- B1: parsing ------------------------------------------------------

    #[test]
    fn parses_levels_and_titles() {
        let o = outline("* A\n** B\n* C\n");
        let levels: Vec<_> = o.headings.iter().map(|h| h.level).collect();
        let titles: Vec<_> = o.headings.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(levels, [1, 2, 1]);
        assert_eq!(titles, ["A", "B", "C"]);
        assert!(o.headings.iter().all(|h| h.todo.is_none()));
    }

    #[test]
    fn parses_todo_and_done_keywords() {
        assert_eq!(outline("* TODO write\n").headings[0].todo, Some(TodoState::Todo));
        assert_eq!(outline("* TODO write\n").headings[0].title, "write");
        assert_eq!(outline("* DONE ship").headings[0].todo, Some(TodoState::Done));
        assert_eq!(outline("* DONE ship").headings[0].title, "ship");
    }

    #[test]
    fn non_headings_are_body() {
        assert!(outline("* \n").headings.is_empty()); // no title
        assert!(outline("*not a heading\n").headings.is_empty()); // no space after stars
        assert!(outline("  * indented\n").headings.is_empty()); // star not at column 0
        assert!(outline("TODOitem is prose\n").headings.is_empty());
        assert!(outline("").headings.is_empty()); // empty buffer
    }

    // ---- B2: subtree extent + navigation ----------------------------------

    #[test]
    fn subtree_extent_stops_at_the_next_equal_or_shallower_heading() {
        let o = outline("* A\ntext\n** B\n* C");
        // A(line0) owns through line 2; B(line2) has no body; C(line3) runs to the end.
        assert_eq!(o.headings[0].last_line, 2); // A
        assert_eq!(o.headings[1].last_line, 2); // B (line 2 only)
        assert_eq!(o.headings[2].last_line, 3); // C
    }

    #[test]
    fn next_and_prev_heading_skip_to_adjacent_headings() {
        let o = outline("* A\ntext\n** B\n* C");
        assert_eq!(next_heading(&o, 0), Some(2)); // A → B
        assert_eq!(next_heading(&o, 2), Some(3)); // B → C
        assert_eq!(next_heading(&o, 3), None); // past the last heading
        assert_eq!(prev_heading(&o, 3), Some(2)); // C → B
        assert_eq!(prev_heading(&o, 0), None); // before the first heading
    }

    #[test]
    fn parent_heading_climbs_one_level() {
        let o = outline("* A\ntext\n** B\n* C");
        assert_eq!(parent_heading(&o, 2), Some(0)); // B (level 2) → A (level 1)
        assert_eq!(parent_heading(&o, 0), None); // A is top level
        assert_eq!(parent_heading(&o, 3), None); // C is top level
    }

    // ---- B3: cycle_todo ---------------------------------------------------

    #[test]
    fn cycle_todo_rotates_none_todo_done() {
        let mut doc = Document::from_text("* task\n");
        OrgProvider.cycle_todo(&mut doc, 0);
        assert_eq!(doc.text(), "* TODO task\n");
        OrgProvider.cycle_todo(&mut doc, 0);
        assert_eq!(doc.text(), "* DONE task\n");
        OrgProvider.cycle_todo(&mut doc, 0);
        assert_eq!(doc.text(), "* task\n");
        assert!(doc.is_modified());
    }

    #[test]
    fn cycle_todo_on_a_non_heading_is_a_noop() {
        let mut doc = Document::from_text("just prose\n");
        OrgProvider.cycle_todo(&mut doc, 0);
        assert_eq!(doc.text(), "just prose\n");
        assert!(!doc.is_modified()); // nothing changed, so the buffer stays clean
    }

    #[test]
    fn cycle_todo_preserves_nesting_level() {
        let mut doc = Document::from_text("** deep\n");
        OrgProvider.cycle_todo(&mut doc, 0);
        assert_eq!(doc.text(), "** TODO deep\n");
    }
}
