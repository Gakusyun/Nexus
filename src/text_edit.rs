//! A single-line editable buffer: the text, the caret, the selection and the IME composition.
//!
//! Everything is byte-indexed — `String` semantics, always on a char boundary — with the UTF-16
//! bridges the platform insists on kept in one place, so no view has to think in code units.

use std::ops::Range;

/// A single-line editable buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEdit {
    text: String,
    cursor: usize,
    /// Where the current selection started. `None` means "no selection" — which is why this is
    /// an `Option` rather than a second offset: a collapsed anchor is indistinguishable from a
    /// caret, and keeping them distinct avoids drawing a zero-width highlight everywhere.
    anchor: Option<usize>,
    /// The range the input method is composing right now, in bytes. Drawn underlined, and
    /// replaced wholesale as the composition changes.
    marked: Option<Range<usize>>,
}

impl TextEdit {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Replace the whole value.
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.anchor = None;
        self.marked = None;
    }

    pub fn clear(&mut self) {
        self.set(String::new());
    }

    /// The selection, normalized so the start is never past the end.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        let (start, end) = if anchor < self.cursor {
            (anchor, self.cursor)
        } else {
            (self.cursor, anchor)
        };
        (start != end).then_some(start..end)
    }

    pub fn select_all(&mut self) {
        if self.text.is_empty() {
            return;
        }
        self.anchor = Some(0);
        self.cursor = self.text.len();
    }

    /// Put the caret at `offset`, extending the selection instead of dropping it when asked.
    pub fn set_cursor(&mut self, offset: usize, extend: bool) {
        let offset = boundary(&self.text, offset);
        if extend {
            self.anchor.get_or_insert(self.cursor);
            if self.anchor == Some(offset) {
                self.anchor = None;
            }
        } else {
            self.anchor = None;
        }
        self.cursor = offset;
    }

    /// Move the caret one character left, extending the selection when asked.
    pub fn move_left(&mut self, extend: bool) {
        // With a selection, the first plain move collapses it rather than stepping past it —
        // stepping would overshoot by one character, which is never what the user meant.
        if !extend && let Some(range) = self.selection() {
            self.set_cursor(range.start, false);
            return;
        }
        let target = prev_boundary(&self.text, self.cursor).unwrap_or(0);
        self.set_cursor(target, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend && let Some(range) = self.selection() {
            self.set_cursor(range.end, false);
            return;
        }
        let target = next_boundary(&self.text, self.cursor).unwrap_or(self.text.len());
        self.set_cursor(target, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        self.set_cursor(0, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        let end = self.text.len();
        self.set_cursor(end, extend);
    }

    /// Insert typed, pasted or committed text.
    pub fn insert(&mut self, text: &str) {
        self.replace_impl(self.target_range(), text);
        self.marked = None;
    }

    /// Replace `range` — or, when the platform passes nothing, whatever is being composed or
    /// selected — with `text`.
    pub fn insert_at(&mut self, range: Option<Range<usize>>, text: &str) {
        let range = range
            .or_else(|| self.target_range())
            .map(|range| self.clamp(range));
        self.replace_impl(range, text);
        self.marked = None;
    }

    /// What an edit without an explicit range replaces: the input method's composition in
    /// progress, else the selection, else nothing (a plain insert at the caret).
    ///
    /// Getting this order wrong is why a Chinese commit used to leave its raw input behind: the
    /// platform's `replace_text_in_range(None, ..)` means *replace the composition*, so the
    /// marked range has to win over the selection.
    fn target_range(&self) -> Option<Range<usize>> {
        self.marked.clone().or_else(|| self.selection())
    }

    /// The same, but the inserted text becomes the input method's composition and `caret` (a byte
    /// offset inside `text`) is where the IME put its cursor.
    pub fn insert_marked_at(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        caret: Option<usize>,
    ) {
        let range = range
            .or_else(|| self.target_range())
            .map(|range| self.clamp(range));
        let start = range.as_ref().map_or(self.cursor, |range| range.start);
        self.replace_impl(range, text);
        self.marked = (!text.is_empty()).then(|| start..start + text.len());
        self.cursor = start + caret.unwrap_or(text.len()).min(text.len());
        self.anchor = None;
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    fn replace_impl(&mut self, range: Option<Range<usize>>, text: &str) {
        let start = range.as_ref().map_or(self.cursor, |range| range.start);
        if let Some(range) = range {
            self.text.replace_range(range, text);
        } else {
            self.text.insert_str(start, text);
        }
        self.cursor = start + text.len();
        self.anchor = None;
    }

    /// Backspace: drop the selection, or the character before the caret.
    pub fn backspace(&mut self) {
        let range = match self.selection() {
            Some(range) => range,
            None => match prev_boundary(&self.text, self.cursor) {
                Some(previous) => previous..boundary(&self.text, self.cursor),
                None => return,
            },
        };
        self.replace_impl(Some(range), "");
    }

    /// Delete: drop the selection, or the character after the caret.
    pub fn delete_forward(&mut self) {
        let range = match self.selection() {
            Some(range) => range,
            None => {
                let cursor = boundary(&self.text, self.cursor);
                match next_boundary(&self.text, cursor) {
                    Some(next) => cursor..next,
                    None => return,
                }
            }
        };
        self.replace_impl(Some(range), "");
    }

    /// The selected text, for a copy.
    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|range| self.text[range].to_string())
    }

    /// Cut: the selected text, removed.
    pub fn cut(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.replace_impl(self.selection(), "");
        Some(text)
    }

    /// The platform's window into the text: how long it is in UTF-16 code units.
    pub fn utf16_len(&self) -> usize {
        utf16_len(&self.text)
    }

    /// A caret or selection range expressed in UTF-16 code units, the way the platform wants it.
    pub fn utf16_range(&self) -> (Range<usize>, bool) {
        let (range, reversed) = match self.anchor {
            Some(anchor) => (
                self.selection().unwrap_or(anchor..anchor),
                anchor > self.cursor,
            ),
            None => {
                let caret = self.cursor..self.cursor;
                (caret, false)
            }
        };
        (
            byte_to_utf16(&self.text, range.start)..byte_to_utf16(&self.text, range.end),
            reversed,
        )
    }

    /// Turn a UTF-16 range from the platform into byte offsets within the current text.
    pub fn utf16_to_bytes(&self, range: Range<usize>) -> Range<usize> {
        let len = self.text.len();
        let start = utf16_to_byte(&self.text, range.start).min(len);
        let end = utf16_to_byte(&self.text, range.end).min(len).max(start);
        start..end
    }

    fn clamp(&self, range: Range<usize>) -> Range<usize> {
        let start = boundary(&self.text, range.start.min(self.text.len()));
        let end = boundary(&self.text, range.end.max(start));
        start..end
    }
}

/// Snap an arbitrary offset back to a char boundary so slicing a `&str` never panics.
pub fn boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The byte offset of the character before `cursor`, if there is one.
pub fn prev_boundary(text: &str, cursor: usize) -> Option<usize> {
    let cursor = boundary(text, cursor);
    text[..cursor].char_indices().next_back().map(|(i, _)| i)
}

/// The byte offset just past the character at `cursor`, if there is one.
pub fn next_boundary(text: &str, cursor: usize) -> Option<usize> {
    let cursor = boundary(text, cursor);
    text[cursor..]
        .char_indices()
        .nth(1)
        .map(|(i, _)| cursor + i)
}

pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// A UTF-16 code-unit offset to a byte offset. Offsets inside a surrogate pair round down to the
/// character, which is what every caller wants and never splits a `char`.
pub fn utf16_to_byte(text: &str, units: usize) -> usize {
    let mut remaining = units;
    for (index, ch) in text.char_indices() {
        if remaining < ch.len_utf16() {
            return index;
        }
        remaining -= ch.len_utf16();
    }
    text.len()
}

pub fn byte_to_utf16(text: &str, byte: usize) -> usize {
    utf16_len(&text[..boundary(text, byte)])
}

/// Strip what has no business in a single-line field, so a pasted URL or family list cannot
/// smuggle in a newline the layout would never show.
pub fn one_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(text: &str) -> TextEdit {
        let mut edit = TextEdit::default();
        edit.set(text);
        edit
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut e = edit("hello");
        e.select_all();
        assert_eq!(e.selection(), Some(0..5));
        e.insert("world");
        assert_eq!(e.text(), "world");
        assert_eq!(e.cursor(), 5);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn backspace_and_delete_take_the_selection_first() {
        let mut e = edit("hello world");
        e.set_cursor(5, false);
        e.set_cursor(11, true);
        e.backspace();
        assert_eq!(e.text(), "hello");

        let mut e = edit("hello world");
        e.set_cursor(0, false);
        e.set_cursor(6, true);
        e.delete_forward();
        assert_eq!(e.text(), "world");
    }

    #[test]
    fn a_plain_move_collapses_the_selection_from_the_right_edge() {
        let mut e = edit("hello");
        e.set_cursor(1, false);
        e.set_cursor(4, true);
        e.move_left(false);
        assert_eq!(e.cursor(), 1);
        assert_eq!(e.selection(), None);

        e.set_cursor(1, false);
        e.set_cursor(4, true);
        e.move_right(false);
        assert_eq!(e.cursor(), 4);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn extending_past_the_caret_flips_the_selection_and_collapses_it_back() {
        let mut e = edit("hello");
        e.set_cursor(2, false);
        e.move_left(true);
        assert_eq!(e.cursor(), 1);
        assert_eq!(e.selection(), Some(1..2));
        // Back to where the anchor is: the highlight must disappear rather than linger.
        e.move_right(true);
        assert_eq!(e.cursor(), 2);
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn movement_walks_characters_not_bytes() {
        let mut e = edit("简体abc");
        e.set_cursor(0, false);
        e.move_right(false);
        assert_eq!(e.cursor(), 3);
        e.move_right(false);
        assert_eq!(e.cursor(), 6);
        e.move_left(false);
        assert_eq!(e.cursor(), 3);
        e.move_home(false);
        assert_eq!(e.cursor(), 0);
        e.move_end(false);
        assert_eq!(e.cursor(), e.text().len());
    }

    #[test]
    fn utf16_bridges_agree_with_the_platform() {
        let e = edit("简体a");
        assert_eq!(e.utf16_len(), 3);
        // Two CJK chars are 2 units each, so byte 6 (the 'a') is unit 2.
        assert_eq!(byte_to_utf16(e.text(), 6), 2);
        assert_eq!(utf16_to_byte(e.text(), 2), 6);
        // Past the end clamps rather than panicking.
        assert_eq!(utf16_to_byte(e.text(), 99), e.text().len());
    }

    #[test]
    fn utf16_to_byte_never_splits_a_surrogate_pair() {
        let text = "a\u{1f600}b"; // emoji is one char, two UTF-16 units
        assert_eq!(utf16_to_byte(text, 1), 1);
        assert_eq!(utf16_to_byte(text, 2), 1, "inside the pair rounds down");
        assert_eq!(utf16_to_byte(text, 3), 5);
    }

    #[test]
    fn a_commit_replaces_the_composition_instead_of_following_it() {
        // What the Windows IME does when the user types `z` and then picks the candidate `中`:
        // the composition is marked first, then a commit arrives with no range, which means
        // "replace what is being composed" — not "type after it".
        let mut e = TextEdit::default();
        e.insert_marked_at(None, "z", None);
        assert_eq!(e.text(), "z");

        e.insert_at(None, "中");
        assert_eq!(e.text(), "中", "the raw input must not survive the commit");
        assert_eq!(e.marked(), None);
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn a_commit_followed_by_a_second_word_keeps_both() {
        let mut e = TextEdit::default();
        e.insert_marked_at(None, "zhong", None);
        e.insert_at(None, "中");
        e.insert_marked_at(None, "wen", None);
        e.insert_at(None, "文");
        assert_eq!(e.text(), "中文");
    }

    #[test]
    fn a_new_composition_replaces_the_previous_draft() {
        let mut e = TextEdit::default();
        e.insert_marked_at(None, "n", None);
        e.insert_marked_at(None, "ni", None);
        e.insert_marked_at(None, "nihao", None);
        assert_eq!(e.text(), "nihao");
    }

    #[test]
    fn composition_is_marked_and_then_replaced_wholesale() {
        let mut e = TextEdit::default();
        e.insert_marked_at(None, "nihao", None);
        assert_eq!(e.text(), "nihao");
        assert_eq!(e.marked(), Some(0..5));

        // The IME narrows the composition; the previous draft must be gone, not appended.
        e.insert_marked_at(Some(0..5), "你好", None);
        assert_eq!(e.text(), "你好");
        assert_eq!(e.marked(), Some(0..6));
        assert_eq!(e.cursor(), 6);

        // A platform commit clears the mark and keeps the text.
        e.insert_at(Some(0..6), "你好");
        assert_eq!(e.text(), "你好");
        assert_eq!(e.marked(), None);
    }

    #[test]
    fn cut_takes_the_selection_and_reports_it() {
        let mut e = edit("hello world");
        e.set_cursor(5, false);
        e.set_cursor(11, true);
        assert_eq!(e.cut().as_deref(), Some(" world"));
        assert_eq!(e.text(), "hello");
        // Nothing selected: nothing to hand out.
        assert_eq!(e.cut(), None);
    }

    #[test]
    fn select_all_on_an_empty_field_stays_a_caret() {
        let mut e = TextEdit::default();
        e.select_all();
        assert_eq!(e.selection(), None);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn a_platform_range_in_utf16_lands_on_char_boundaries() {
        let mut e = edit("简体abc");
        // Platform asks for unit 0..1 = the first CJK character (they are one unit each).
        let bytes = e.utf16_to_bytes(0..1);
        assert_eq!(bytes, 0..3);
        e.insert_at(Some(bytes), "中");
        assert_eq!(e.text(), "中体abc");
        // A range past the end clamps instead of panicking ('c' starts at byte 8).
        assert_eq!(e.utf16_to_bytes(4..99), 8..9);
    }
}
