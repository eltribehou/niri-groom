//! A tiny line editor for the rename field, with a cursor and readline-style
//! (Emacs) editing operations. Stored as chars so the cursor is codepoint-safe.

pub struct Edit {
    pub buf: Vec<char>,
    pub cursor: usize,
}

impl Edit {
    pub fn new(s: &str) -> Self {
        let buf: Vec<char> = s.chars().collect();
        let cursor = buf.len();
        Edit { buf, cursor }
    }

    pub fn text(&self) -> String {
        self.buf.iter().collect()
    }

    pub fn insert(&mut self, c: char) {
        self.buf.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Delete the char before the cursor (Backspace / C-h).
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.buf.remove(self.cursor);
        }
    }

    /// Delete the char at the cursor (Delete / C-d).
    pub fn delete(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.buf.len() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buf.len();
    }

    /// Kill from the cursor to the end of the line (C-k).
    pub fn kill_to_end(&mut self) {
        self.buf.truncate(self.cursor);
    }

    /// Kill from the start of the line to the cursor (C-u).
    pub fn kill_to_start(&mut self) {
        self.buf.drain(0..self.cursor);
        self.cursor = 0;
    }

    /// Word boundary to the left of the cursor (skip separators, then word).
    fn prev_word(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && !self.buf[i - 1].is_alphanumeric() {
            i -= 1;
        }
        while i > 0 && self.buf[i - 1].is_alphanumeric() {
            i -= 1;
        }
        i
    }

    /// Word boundary to the right of the cursor.
    fn next_word(&self) -> usize {
        let n = self.buf.len();
        let mut i = self.cursor;
        while i < n && !self.buf[i].is_alphanumeric() {
            i += 1;
        }
        while i < n && self.buf[i].is_alphanumeric() {
            i += 1;
        }
        i
    }

    pub fn word_left(&mut self) {
        self.cursor = self.prev_word();
    }

    pub fn word_right(&mut self) {
        self.cursor = self.next_word();
    }

    /// Kill the word before the cursor (C-w / M-Backspace).
    pub fn kill_word_left(&mut self) {
        let start = self.prev_word();
        self.buf.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Kill the word after the cursor (M-d).
    pub fn kill_word_right(&mut self) {
        let end = self.next_word();
        self.buf.drain(self.cursor..end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The buffer with `|` marking the cursor, so a test says what the user
    /// would see rather than asserting on two fields at once.
    fn render(e: &Edit) -> String {
        let mut s: String = e.buf[..e.cursor].iter().collect();
        s.push('|');
        s.extend(&e.buf[e.cursor..]);
        s
    }

    fn at(text: &str) -> Edit {
        let cursor = text.find('|').expect("marker");
        let buf: Vec<char> = text.chars().filter(|&c| c != '|').collect();
        Edit {
            cursor: text[..cursor].chars().count(),
            buf,
        }
    }

    #[test]
    fn opens_with_the_cursor_after_the_existing_name() {
        assert_eq!(render(&Edit::new("code")), "code|");
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut e = at("co|de");
        e.insert('r');
        assert_eq!(render(&e), "cor|de");
    }

    #[test]
    fn backspace_and_delete_take_the_char_on_either_side() {
        let mut e = at("ab|cd");
        e.backspace();
        assert_eq!(render(&e), "a|cd");
        e.delete();
        assert_eq!(render(&e), "a|d");
    }

    #[test]
    fn deleting_past_either_end_leaves_the_line_alone() {
        let mut e = at("|ab");
        e.backspace();
        assert_eq!(render(&e), "|ab");
        let mut e = at("ab|");
        e.delete();
        assert_eq!(render(&e), "ab|");
    }

    #[test]
    fn the_cursor_stops_at_both_ends() {
        let mut e = at("|a");
        e.left();
        assert_eq!(render(&e), "|a");
        e.right();
        e.right();
        assert_eq!(render(&e), "a|");
    }

    #[test]
    fn home_and_end_jump_to_the_edges() {
        let mut e = at("ab|cd");
        e.home();
        assert_eq!(render(&e), "|abcd");
        e.end();
        assert_eq!(render(&e), "abcd|");
    }

    #[test]
    fn kill_clears_the_side_the_cursor_points_at() {
        let mut e = at("ab|cd");
        e.kill_to_end();
        assert_eq!(render(&e), "ab|");
        let mut e = at("ab|cd");
        e.kill_to_start();
        assert_eq!(render(&e), "|cd");
    }

    #[test]
    fn word_motion_skips_the_separators_before_the_word() {
        let mut e = at("alpha  beta|");
        e.word_left();
        assert_eq!(render(&e), "alpha  |beta");
        e.word_left();
        assert_eq!(render(&e), "|alpha  beta");
        e.word_right();
        assert_eq!(render(&e), "alpha|  beta");
    }

    #[test]
    fn killing_a_word_takes_its_separators_with_it() {
        let mut e = at("alpha  beta|");
        e.kill_word_left();
        assert_eq!(render(&e), "alpha  |");
        let mut e = at("|alpha  beta");
        e.kill_word_right();
        assert_eq!(render(&e), "|  beta");
    }

    #[test]
    fn the_cursor_counts_characters_not_bytes() {
        let mut e = Edit::new("héllo");
        assert_eq!(e.cursor, 5);
        e.backspace();
        assert_eq!(e.text(), "héll");
        let mut e = at("é|à");
        e.backspace();
        assert_eq!(e.text(), "à");
    }
}
