//! Pure vertical-scroll math. Scroll lives in the TUI (not the core) because it depends on
//! the terminal height, which the core must never know about — but keeping it a plain
//! function keeps it fully tested.

/// Given the cursor's line, the current top-of-viewport line, and the visible body height,
/// return the new top line that keeps the cursor visible with minimal scrolling.
///
/// The cursor is nudged into view only when it falls outside `[top, top + height)`; otherwise
/// the viewport holds still (no jitter). Coordinates are document lines.
pub fn viewport_top(cursor_line: usize, top: usize, height: usize) -> usize {
    if height == 0 {
        top
    } else if cursor_line < top {
        cursor_line
    } else if cursor_line >= top + height {
        cursor_line - height + 1
    } else {
        top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_cursor_leaves_the_viewport_unchanged() {
        assert_eq!(viewport_top(5, 3, 10), 3);
    }

    #[test]
    fn cursor_above_the_viewport_scrolls_up_to_it() {
        assert_eq!(viewport_top(2, 5, 10), 2);
    }

    #[test]
    fn cursor_below_the_viewport_scrolls_so_it_is_the_last_row() {
        assert_eq!(viewport_top(20, 5, 10), 11); // 20 - 10 + 1
    }

    #[test]
    fn exactly_one_past_the_bottom_scrolls_by_one() {
        // top=0, height=10 shows lines 0..=9; line 10 is one past → top becomes 1.
        assert_eq!(viewport_top(10, 0, 10), 1);
    }

    #[test]
    fn zero_height_is_a_noop() {
        assert_eq!(viewport_top(42, 7, 0), 7);
    }
}
