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
