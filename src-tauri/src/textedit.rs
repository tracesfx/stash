//! A single-line text field for native (tiny-skia) windows: the quick note and the tag radial.
//! Supports the editing keys SPEC §4 requires: Ctrl+Backspace/Delete word-delete, Ctrl+A/C/X/V/Z/Y, arrows,
//! Ctrl+arrows, Home/End and Shift-select. Rendering is left to `draw`, so it works in any layered window.
use tiny_skia::{Pixmap, Rect, Transform};

use crate::gfx::{self, Weight};

#[derive(Default)]
pub struct TextField {
    pub chars: Vec<char>,
    caret: usize,
    anchor: usize,
    undo: Vec<(Vec<char>, usize)>,
    redo: Vec<(Vec<char>, usize)>,
    last_was_typing: bool,
    scroll: f32,
    max_len: usize,
}

#[derive(PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Key not used by the field (Enter, Esc, Tab, … are the window's business).
    Ignored,
    /// Caret/selection moved; repaint.
    Moved,
    /// Text changed; repaint and react.
    Changed,
}

// virtual-key codes
const VK_BACK: u32 = 0x08;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_RIGHT: u32 = 0x27;
const VK_DELETE: u32 = 0x2E;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' || c == '#' || c == '\'' {
        Class::Word
    } else {
        Class::Punct
    }
}

impl TextField {
    pub fn new(max_len: usize) -> Self {
        Self { max_len, ..Default::default() }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn set_text(&mut self, s: &str) {
        self.snapshot();
        self.chars = s.chars().take(self.max_len).collect();
        self.caret = self.chars.len();
        self.anchor = self.caret;
    }

    pub fn clear(&mut self) {
        self.set_text("");
    }

    fn sel(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    fn has_sel(&self) -> bool {
        self.caret != self.anchor
    }

    fn snapshot(&mut self) {
        self.undo.push((self.chars.clone(), self.caret));
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn delete_sel(&mut self) {
        let (a, b) = self.sel();
        self.chars.drain(a..b);
        self.caret = a;
        self.anchor = a;
    }

    fn insert(&mut self, s: &str) {
        if self.has_sel() {
            self.delete_sel();
        }
        let room = self.max_len.saturating_sub(self.chars.len());
        let ins: Vec<char> = s.chars().filter(|c| !c.is_control()).take(room).collect();
        let n = ins.len();
        self.chars.splice(self.caret..self.caret, ins);
        self.caret += n;
        self.anchor = self.caret;
    }

    /// Start of the word before `i` (Ctrl+Left / Ctrl+Backspace): skip spaces, then one run of a class.
    fn word_left(&self, mut i: usize) -> usize {
        while i > 0 && class(self.chars[i - 1]) == Class::Space {
            i -= 1;
        }
        if i > 0 {
            let k = class(self.chars[i - 1]);
            while i > 0 && class(self.chars[i - 1]) == k {
                i -= 1;
            }
        }
        i
    }

    /// End of the word after `i` (Ctrl+Right / Ctrl+Delete): one run of a class, then trailing spaces.
    fn word_right(&self, mut i: usize) -> usize {
        let n = self.chars.len();
        if i < n {
            let k = class(self.chars[i]);
            if k != Class::Space {
                while i < n && class(self.chars[i]) == k {
                    i += 1;
                }
            }
        }
        while i < n && class(self.chars[i]) == Class::Space {
            i += 1;
        }
        i
    }

    fn move_to(&mut self, i: usize, shift: bool) -> Outcome {
        self.caret = i.min(self.chars.len());
        if !shift {
            self.anchor = self.caret;
        }
        self.last_was_typing = false;
        Outcome::Moved
    }

    /// WM_KEYDOWN.
    pub fn key_down(&mut self, vk: u32, ctrl: bool, shift: bool) -> Outcome {
        match vk {
            VK_LEFT => {
                if self.has_sel() && !shift && !ctrl {
                    let a = self.sel().0;
                    return self.move_to(a, false);
                }
                let to = if ctrl { self.word_left(self.caret) } else { self.caret.saturating_sub(1) };
                self.move_to(to, shift)
            }
            VK_RIGHT => {
                if self.has_sel() && !shift && !ctrl {
                    let b = self.sel().1;
                    return self.move_to(b, false);
                }
                let to = if ctrl { self.word_right(self.caret) } else { (self.caret + 1).min(self.chars.len()) };
                self.move_to(to, shift)
            }
            VK_HOME => self.move_to(0, shift),
            VK_END => self.move_to(self.chars.len(), shift),
            VK_BACK => {
                if !self.has_sel() {
                    if self.caret == 0 {
                        return Outcome::Moved;
                    }
                    self.anchor = if ctrl { self.word_left(self.caret) } else { self.caret - 1 };
                }
                self.snapshot();
                self.delete_sel();
                self.last_was_typing = false;
                Outcome::Changed
            }
            VK_DELETE => {
                if !self.has_sel() {
                    if self.caret >= self.chars.len() {
                        return Outcome::Moved;
                    }
                    self.anchor = if ctrl { self.word_right(self.caret) } else { self.caret + 1 };
                }
                self.snapshot();
                self.delete_sel();
                self.last_was_typing = false;
                Outcome::Changed
            }
            0x41 if ctrl => {
                // Ctrl+A
                self.anchor = 0;
                self.caret = self.chars.len();
                Outcome::Moved
            }
            0x43 | 0x58 if ctrl => {
                // Ctrl+C / Ctrl+X
                if !self.has_sel() {
                    return Outcome::Moved;
                }
                let (a, b) = self.sel();
                let s: String = self.chars[a..b].iter().collect();
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(s);
                }
                if vk == 0x58 {
                    self.snapshot();
                    self.delete_sel();
                    return Outcome::Changed;
                }
                Outcome::Moved
            }
            0x56 if ctrl => {
                // Ctrl+V: first line only (single-line field)
                let Some(t) = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()) else { return Outcome::Moved };
                let line = t.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string();
                if line.is_empty() {
                    return Outcome::Moved;
                }
                self.snapshot();
                self.insert(&line);
                self.last_was_typing = false;
                Outcome::Changed
            }
            0x5A if ctrl => {
                // Ctrl+Z (Ctrl+Shift+Z = redo)
                let (from, to) = if shift { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
                let Some((c, k)) = from.pop() else { return Outcome::Moved };
                to.push((std::mem::replace(&mut self.chars, c), self.caret));
                self.caret = k.min(self.chars.len());
                self.anchor = self.caret;
                self.last_was_typing = false;
                Outcome::Changed
            }
            0x59 if ctrl => self.key_down(0x5A, true, true), // Ctrl+Y = redo
            _ => Outcome::Ignored,
        }
    }

    /// WM_CHAR: printable characters only (control chars arrive for Ctrl+letters, Enter, Tab, Esc, Backspace).
    pub fn char_input(&mut self, c: char) -> Outcome {
        if c.is_control() {
            return Outcome::Ignored;
        }
        if self.chars.len() >= self.max_len && !self.has_sel() {
            return Outcome::Moved;
        }
        if !self.last_was_typing || c == ' ' {
            self.snapshot(); // typing a run of letters undoes as one step; a space starts a new step
        }
        self.insert(&c.to_string());
        self.last_was_typing = true;
        Outcome::Changed
    }

    /// Draws the text (or `placeholder`) inside `r`, with selection and an optional caret, scrolling
    /// horizontally to keep the caret visible.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, pm: &mut Pixmap, r: Rect, px: f32, col: [u8; 3], placeholder: &str, caret_on: bool, accent: [u8; 3]) {
        let cy = r.top() + r.height() / 2.0;
        if self.chars.is_empty() {
            gfx::draw_text_at(pm, placeholder, px, r.left(), cy, col, 110, Weight::Regular, Some((r.left(), r.right())));
            if caret_on {
                caret(pm, r.left(), cy, px, col);
            }
            return;
        }
        let adv = gfx::advances(&self.chars, px, Weight::Regular);
        let cx = adv[self.caret];
        let view = r.width() - 2.0;
        if cx - self.scroll > view {
            self.scroll = cx - view;
        }
        if cx - self.scroll < 0.0 {
            self.scroll = cx;
        }
        let total = *adv.last().unwrap_or(&0.0);
        self.scroll = self.scroll.min((total - view).max(0.0)).max(0.0);
        let x0 = r.left() - self.scroll;
        if self.has_sel() {
            let (a, b) = self.sel();
            let (sx0, sx1) = ((x0 + adv[a]).max(r.left()), (x0 + adv[b]).min(r.right()));
            if let Some(p) = gfx::rrect(sx0, cy - px * 0.72, (sx1 - sx0).max(1.0), px * 1.44, 3.0) {
                pm.fill_path(&p, &gfx::paint([accent[0], accent[1], accent[2], 120]), tiny_skia::FillRule::Winding, Transform::identity(), None);
            }
        }
        let s = self.text();
        gfx::draw_text_at(pm, &s, px, x0, cy, col, 255, Weight::Regular, Some((r.left(), r.right())));
        if caret_on {
            caret(pm, x0 + cx, cy, px, col);
        }
    }
}

fn caret(pm: &mut Pixmap, x: f32, cy: f32, px: f32, col: [u8; 3]) {
    if let Some(p) = gfx::rrect(x.round(), cy - px * 0.7, 1.6, px * 1.4, 0.8) {
        pm.fill_path(&p, &gfx::paint([col[0], col[1], col[2], 235]), tiny_skia::FillRule::Winding, Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(s: &str) -> TextField {
        let mut f = TextField::new(500);
        f.set_text(s);
        f
    }

    #[test]
    fn ctrl_backspace_deletes_word() {
        let mut f = field("hello brave world");
        f.key_down(VK_BACK, true, false);
        assert_eq!(f.text(), "hello brave ");
        f.key_down(VK_BACK, true, false);
        assert_eq!(f.text(), "hello ");
        f.key_down(VK_BACK, true, false);
        assert_eq!(f.text(), "");
    }

    #[test]
    fn ctrl_delete_and_arrows() {
        let mut f = field("one two three");
        f.key_down(VK_HOME, false, false);
        f.key_down(VK_DELETE, true, false);
        assert_eq!(f.text(), "two three");
        f.key_down(VK_RIGHT, true, false); // after "two "
        f.key_down(VK_RIGHT, true, true); // select "three"
        f.char_input('X');
        assert_eq!(f.text(), "two X");
    }

    #[test]
    fn select_all_and_undo() {
        let mut f = field("abc");
        f.key_down(0x41, true, false);
        f.char_input('z');
        assert_eq!(f.text(), "z");
        f.key_down(0x5A, true, false);
        assert_eq!(f.text(), "abc");
        f.key_down(0x59, true, false);
        assert_eq!(f.text(), "z");
    }

    #[test]
    fn typing_run_undoes_as_one_step() {
        let mut f = TextField::new(100);
        for c in "tag".chars() {
            f.char_input(c);
        }
        f.key_down(0x5A, true, false);
        assert_eq!(f.text(), "");
    }

    #[test]
    fn control_chars_ignored_and_limit() {
        let mut f = TextField::new(3);
        assert_eq!(f.char_input('\u{7f}'), Outcome::Ignored);
        for c in "abcdef".chars() {
            f.char_input(c);
        }
        assert_eq!(f.text(), "abc");
    }
}
