//! niri-groom — a fullscreen layer-shell overlay that surveys niri workspaces
//! and windows as a proportional map, and lets me kill a whole workspace (`w`)
//! or a single window (`x`) with no confirmation.

mod niri;

use gtk4 as gtk;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, DrawingArea, EventControllerKey};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

const APP_ID: &str = "io.iwd.niri-groom";

/// A workspace together with the windows it holds (sorted by column, then row).
struct WsView {
    ws: niri::Workspace,
    windows: Vec<niri::Window>,
}

/// One output (monitor) and its workspaces, sorted by index. `x`/`y` are niri's
/// logical position (used to order/place outputs) and `w` its logical width
/// (used for proportional horizontal sizing). Height isn't kept — outputs are
/// drawn full-height.
struct OutputView {
    name: String,
    workspaces: Vec<WsView>,
    x: f64,
    y: f64,
    w: f64,
}

/// The full picture I draw, plus a flat navigation order over workspaces.
struct Model {
    outputs: Vec<OutputView>,
    /// `(output index, workspace index within output)` in display order.
    nav: Vec<(usize, usize)>,
}

/// A tiny line editor for the rename field, with a cursor and readline-style
/// (Emacs) editing operations. Stored as chars so the cursor is codepoint-safe.
struct Edit {
    buf: Vec<char>,
    cursor: usize,
}

impl Edit {
    fn new(s: &str) -> Self {
        let buf: Vec<char> = s.chars().collect();
        let cursor = buf.len();
        Edit { buf, cursor }
    }

    fn text(&self) -> String {
        self.buf.iter().collect()
    }

    fn insert(&mut self, c: char) {
        self.buf.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Delete the char before the cursor (Backspace / C-h).
    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.buf.remove(self.cursor);
        }
    }

    /// Delete the char at the cursor (Delete / C-d).
    fn delete(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
        }
    }

    fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn right(&mut self) {
        if self.cursor < self.buf.len() {
            self.cursor += 1;
        }
    }

    fn home(&mut self) {
        self.cursor = 0;
    }

    fn end(&mut self) {
        self.cursor = self.buf.len();
    }

    /// Kill from the cursor to the end of the line (C-k).
    fn kill_to_end(&mut self) {
        self.buf.truncate(self.cursor);
    }

    /// Kill from the start of the line to the cursor (C-u).
    fn kill_to_start(&mut self) {
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

    fn word_left(&mut self) {
        self.cursor = self.prev_word();
    }

    fn word_right(&mut self) {
        self.cursor = self.next_word();
    }

    /// Kill the word before the cursor (C-w / M-Backspace).
    fn kill_word_left(&mut self) {
        let start = self.prev_word();
        self.buf.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Kill the word after the cursor (M-d).
    fn kill_word_right(&mut self) {
        let end = self.next_word();
        self.buf.drain(self.cursor..end);
    }
}

struct State {
    model: Model,
    /// Index into `model.nav` — the currently selected workspace.
    sel_nav: usize,
    /// Index into the selected workspace's `windows`.
    sel_win: usize,
    /// When renaming, the in-overlay line editor for the selected workspace.
    editing: Option<Edit>,
    error: Option<String>,
}

impl State {
    /// The currently selected workspace view, if any.
    fn sel_ws(&self) -> Option<&WsView> {
        let (o, w) = *self.model.nav.get(self.sel_nav)?;
        self.model.outputs.get(o)?.workspaces.get(w)
    }

    fn selected_ws_id(&self) -> Option<u64> {
        self.sel_ws().map(|v| v.ws.id)
    }

    fn selected_win_id(&self) -> Option<u64> {
        self.sel_ws()
            .and_then(|v| v.windows.get(self.sel_win))
            .map(|w| w.id)
    }

    /// The nav index of niri's focused workspace, so I can open the overlay
    /// where the user is actually looking (defaults to the first workspace).
    fn focused_nav(&self) -> usize {
        self.model
            .nav
            .iter()
            .position(|&(o, w)| self.model.outputs[o].workspaces[w].ws.is_focused)
            .unwrap_or(0)
    }

    /// Output index of the current selection.
    fn sel_output(&self) -> usize {
        self.model.nav.get(self.sel_nav).map(|&(o, _)| o).unwrap_or(0)
    }
}

/// Build the model from a fresh niri snapshot.
fn build_model() -> Result<Model, String> {
    let workspaces = niri::fetch_workspaces()?;
    let windows = niri::fetch_windows()?;

    // Bucket windows by their workspace id.
    let mut by_ws: BTreeMap<u64, Vec<niri::Window>> = BTreeMap::new();
    for w in windows {
        if let Some(ws_id) = w.workspace_id {
            by_ws.entry(ws_id).or_default().push(w);
        }
    }

    // Logical placement per output, so I can draw screens where niri puts them.
    let geom: BTreeMap<String, (f64, f64, f64)> = niri::fetch_outputs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|o| o.logical.map(|l| (o.name, (l.x, l.y, l.width))))
        .collect();

    // Group workspaces by output.
    let mut by_output: BTreeMap<String, Vec<niri::Workspace>> = BTreeMap::new();
    for ws in workspaces {
        let out = ws.output.clone().unwrap_or_else(|| "?".to_string());
        by_output.entry(out).or_default().push(ws);
    }

    // Build an OutputView per output, falling back to a synthetic horizontal row
    // for any output niri didn't report geometry for (disabled, or no `outputs`).
    let mut fallback_x = 0.0;
    let mut outputs: Vec<OutputView> = Vec::new();
    for (name, mut wss) in by_output {
        wss.sort_by_key(|w| w.idx);
        let workspaces = wss
            .into_iter()
            .filter_map(|ws| {
                let mut wins = by_ws.remove(&ws.id).unwrap_or_default();
                wins.sort_by_key(|w| (w.column(), w.row(), w.id));
                // Hide unnamed empty workspaces: these are niri's scratch space
                // (the permanent trailing one plus any transient empties). They
                // can't be meaningfully killed and only clutter the map. Named
                // empty workspaces stay — `w` unsets the name and niri reclaims.
                if wins.is_empty() && ws.name.is_none() {
                    return None;
                }
                Some(WsView { ws, windows: wins })
            })
            .collect();
        let (x, y, w) = geom.get(&name).copied().unwrap_or_else(|| {
            let g = (fallback_x, 0.0, 1600.0);
            fallback_x += 1600.0;
            g
        });
        outputs.push(OutputView { name, workspaces, x, y, w });
    }

    // Order outputs left-to-right, then top-to-bottom, by logical position.
    outputs.sort_by(|a, b| {
        a.x.partial_cmp(&b.x)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
    });

    // Flat nav order over all workspaces, following the output order above.
    let mut nav = Vec::new();
    for (o_idx, output) in outputs.iter().enumerate() {
        for w_idx in 0..output.workspaces.len() {
            nav.push((o_idx, w_idx));
        }
    }

    Ok(Model { outputs, nav })
}

/// Refresh the model in place, preserving the selection by id where possible.
fn refresh(state: &Rc<RefCell<State>>) {
    let (prev_ws_id, prev_win_id) = {
        let s = state.borrow();
        (s.selected_ws_id(), s.selected_win_id())
    };

    let mut s = state.borrow_mut();
    match build_model() {
        Ok(model) => {
            s.model = model;
            s.error = None;
        }
        Err(e) => {
            s.error = Some(e);
        }
    }

    // Re-find the previously selected workspace.
    if let Some(ws_id) = prev_ws_id {
        if let Some(idx) = s.model.nav.iter().position(|&(o, w)| {
            s.model.outputs[o].workspaces[w].ws.id == ws_id
        }) {
            s.sel_nav = idx;
        }
    }
    if s.sel_nav >= s.model.nav.len() {
        s.sel_nav = s.model.nav.len().saturating_sub(1);
    }

    // Re-find the previously selected window within the workspace.
    let win_count = s.sel_ws().map(|v| v.windows.len()).unwrap_or(0);
    if let Some(win_id) = prev_win_id {
        if let Some(idx) = s
            .sel_ws()
            .and_then(|v| v.windows.iter().position(|w| w.id == win_id))
        {
            s.sel_win = idx;
        }
    }
    if win_count == 0 {
        s.sel_win = 0;
    } else if s.sel_win >= win_count {
        s.sel_win = win_count - 1;
    }
}

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    // Single-instance: GApplication forwards a second launch's `activate` to the
    // already-running instance. Pressing the keybind again would otherwise stack
    // a second layer-shell overlay, and two exclusive keyboard grabs deadlock
    // input. So if a window already exists, just raise it and bail.
    if let Some(win) = app.windows().into_iter().next() {
        win.present();
        return;
    }

    let model = build_model().unwrap_or(Model {
        outputs: Vec::new(),
        nav: Vec::new(),
    });
    let state = Rc::new(RefCell::new(State {
        model,
        sel_nav: 0,
        sel_win: 0,
        editing: None,
        error: None,
    }));
    // Open the overlay on the currently focused workspace.
    {
        let mut s = state.borrow_mut();
        s.sel_nav = s.focused_nav();
    }

    let window = ApplicationWindow::builder()
        .application(app)
        .title("niri-groom")
        .build();

    // Let the cairo backdrop own the whole surface (it paints an opaque fill);
    // a transparent GTK window background avoids the theme drawing its own.
    let css = gtk::CssProvider::new();
    css.load_from_data("window { background: transparent; }");
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    // Layer-shell: a fullscreen overlay that grabs the keyboard.
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("niri-groom"));
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
        window.set_anchor(edge, true);
    }

    let area = DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);

    {
        let state = state.clone();
        area.set_draw_func(move |_area, cr, w, h| {
            draw(cr, w as f64, h as f64, &state.borrow());
        });
    }

    // Keyboard handling.
    let key = EventControllerKey::new();
    {
        let state = state.clone();
        let area = area.clone();
        let app = app.clone();
        key.connect_key_pressed(move |_ctrl, keyval, _code, mods| {
            let handled = handle_key(&keyval, mods, &state, &app);
            if handled {
                area.queue_draw();
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(key);

    window.set_child(Some(&area));
    window.present();

    // Live refresh on a timer so the map keeps up with the compositor.
    {
        let state = state.clone();
        let area = area.clone();
        glib::timeout_add_local(Duration::from_millis(800), move || {
            refresh(&state);
            area.queue_draw();
            glib::ControlFlow::Continue
        });
    }
}

/// Snapshot the compositor's focused window and workspace, so an operation that
/// must change focus (rename, move) can restore it afterwards.
fn capture_focus() -> (Option<u64>, Option<u64>) {
    let win = niri::fetch_windows()
        .ok()
        .and_then(|ws| ws.into_iter().find(|w| w.is_focused).map(|w| w.id));
    let ws = niri::fetch_workspaces()
        .ok()
        .and_then(|wss| wss.into_iter().find(|w| w.is_focused).map(|w| w.id));
    (win, ws)
}

fn restore_focus((prev_win, prev_ws): (Option<u64>, Option<u64>)) {
    if let Some(id) = prev_win {
        let _ = niri::focus_window(id);
    } else if let Some(id) = prev_ws {
        let _ = niri::focus_workspace_by_id(id);
    }
}

/// Commit the rename: focus the target workspace, set (or unset, if empty) its
/// name, then restore the prior focus and close the field.
fn commit_rename(state: &Rc<RefCell<State>>) {
    let (target, name) = {
        let s = state.borrow();
        (
            s.selected_ws_id(),
            s.editing.as_ref().map(|e| e.text()).unwrap_or_default(),
        )
    };
    if let Some(id) = target {
        let focus = capture_focus();
        if niri::focus_workspace_by_id(id).is_ok() {
            let _ = niri::rename_focused_workspace(name.trim());
        }
        restore_focus(focus);
    }
    state.borrow_mut().editing = None;
    refresh(state);
}

/// Handle a keystroke while the rename field is open, with readline-style
/// (Emacs) editing bindings. Returns true to request a redraw.
fn handle_edit_key(keyval: &gdk::Key, mods: gdk::ModifierType, state: &Rc<RefCell<State>>) -> bool {
    let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = mods.contains(gdk::ModifierType::ALT_MASK);
    let ch = keyval.to_unicode();
    let lc = ch.map(|c| c.to_ascii_lowercase());

    // Cancel (Esc / C-g) and commit (Enter) end the edit and re-enter `state`,
    // so handle them before borrowing the buffer.
    if matches!(*keyval, gdk::Key::Escape) || (ctrl && lc == Some('g')) {
        state.borrow_mut().editing = None;
        return true;
    }
    if matches!(*keyval, gdk::Key::Return | gdk::Key::KP_Enter) {
        commit_rename(state);
        return true;
    }

    let mut s = state.borrow_mut();
    let e = match s.editing.as_mut() {
        Some(e) => e,
        None => return false,
    };

    if ctrl {
        match lc {
            Some('a') => e.home(),
            Some('e') => e.end(),
            Some('b') => e.left(),
            Some('f') => e.right(),
            Some('d') => e.delete(),
            Some('h') => e.backspace(),
            Some('k') => e.kill_to_end(),
            Some('u') => e.kill_to_start(),
            Some('w') => e.kill_word_left(),
            _ => return false,
        }
        return true;
    }

    if alt {
        match lc {
            Some('b') => e.word_left(),
            Some('f') => e.word_right(),
            Some('d') => e.kill_word_right(),
            _ if matches!(*keyval, gdk::Key::BackSpace) => e.kill_word_left(),
            _ => return false,
        }
        return true;
    }

    match *keyval {
        gdk::Key::BackSpace => e.backspace(),
        gdk::Key::Delete => e.delete(),
        gdk::Key::Left => e.left(),
        gdk::Key::Right => e.right(),
        gdk::Key::Home => e.home(),
        gdk::Key::End => e.end(),
        _ => {
            if let Some(c) = ch.filter(|c| !c.is_control()) {
                e.insert(c);
            } else {
                return false;
            }
        }
    }
    true
}

/// Returns true if the key changed something and a redraw is wanted.
fn handle_key(
    keyval: &gdk::Key,
    mods: gdk::ModifierType,
    state: &Rc<RefCell<State>>,
    app: &Application,
) -> bool {
    // While renaming, the whole keyboard feeds the edit buffer.
    if state.borrow().editing.is_some() {
        return handle_edit_key(keyval, mods, state);
    }

    let ch = keyval.to_unicode();

    // Ctrl+H / Ctrl+L: move the selected window's column within its workspace.
    if mods.contains(gdk::ModifierType::CONTROL_MASK) {
        match ch.map(|c| c.to_ascii_lowercase()) {
            Some('h') => return move_selected_column(state, false),
            Some('l') => return move_selected_column(state, true),
            _ => {}
        }
    }

    match (ch, *keyval) {
        (Some('q'), _) | (_, gdk::Key::Escape) => {
            app.quit();
            false
        }
        // Rename the selected workspace (edit inline; the auto-refresh timer
        // keeps the map current, so there's no separate manual-refresh key).
        (Some('r'), _) => {
            let name = state
                .borrow()
                .sel_ws()
                .map(|ws| ws.ws.name.clone().unwrap_or_default());
            if let Some(name) = name {
                state.borrow_mut().editing = Some(Edit::new(&name));
            }
            true
        }
        // Focus the selected workspace and dismiss the overlay (jump to it).
        (_, gdk::Key::Return) | (_, gdk::Key::KP_Enter) => {
            if let Some(id) = state.borrow().selected_ws_id() {
                let _ = niri::focus_workspace_by_id(id);
            }
            app.quit();
            false
        }
        // Workspace navigation (vertical); crosses to the adjacent screen at
        // the top/bottom boundary of an output's workspace stack.
        (Some('j'), _) | (_, gdk::Key::Down) => move_ws(state, 1),
        (Some('k'), _) | (_, gdk::Key::Up) => move_ws(state, -1),
        // Reorder the selected workspace within its monitor (Shift+J / Shift+K).
        (Some('J'), _) => move_selected_ws(state, true),
        (Some('K'), _) => move_selected_ws(state, false),
        // Window navigation (horizontal, within workspace).
        (Some('l'), _) | (_, gdk::Key::Right) => move_win(state, 1),
        (Some('h'), _) | (_, gdk::Key::Left) => move_win(state, -1),
        // Move the selected workspace to the screen left/right (Shift+H / Shift+L).
        (Some('L'), _) => move_selected_ws_to_monitor(state, false),
        (Some('H'), _) => move_selected_ws_to_monitor(state, true),
        // Jump straight to the next/previous screen (output).
        (_, gdk::Key::Tab) => move_output(state, 1),
        (_, gdk::Key::ISO_Left_Tab) => move_output(state, -1),
        // Kill the selected window.
        (Some('x'), _) => {
            let id = state.borrow().selected_win_id();
            if let Some(id) = id {
                let _ = niri::close_window(id);
                refresh(state);
            }
            true
        }
        // Kill the whole selected workspace: close every window, then drop the
        // workspace name so niri reclaims the now-empty workspace.
        (Some('w'), _) => {
            let (ids, name): (Vec<u64>, Option<String>) = {
                let s = state.borrow();
                match s.sel_ws() {
                    Some(v) => (v.windows.iter().map(|w| w.id).collect(), v.ws.name.clone()),
                    None => (Vec::new(), None),
                }
            };
            for id in ids {
                let _ = niri::close_window(id);
            }
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                let _ = niri::unset_workspace_name(&name);
            }
            refresh(state);
            true
        }
        _ => false,
    }
}

fn move_ws(state: &Rc<RefCell<State>>, delta: i32) -> bool {
    let mut s = state.borrow_mut();
    let n = s.model.nav.len();
    if n == 0 {
        return false;
    }
    let cur = s.sel_nav as i32;
    let next = (cur + delta).clamp(0, n as i32 - 1) as usize;
    if next != s.sel_nav {
        s.sel_nav = next;
        s.sel_win = 0;
        true
    } else {
        false
    }
}

/// Move the selected workspace up/down within its monitor. The selection is
/// preserved by id across the refresh, so the highlight follows the workspace.
fn move_selected_ws(state: &Rc<RefCell<State>>, down: bool) -> bool {
    let target = {
        let s = state.borrow();
        s.sel_ws()
            .and_then(|v| v.ws.output.clone().map(|o| (o, v.ws.idx)))
    };
    if let Some((output, idx)) = target {
        // Moving requires focusing, so restore focus afterwards.
        let focus = capture_focus();
        let _ = niri::move_workspace(&output, idx, down);
        restore_focus(focus);
        refresh(state);
        true
    } else {
        false
    }
}

/// Move the selected window's column left/right within its workspace, then
/// restore focus to where the user left it (moving requires focusing a window
/// in the column). The selection follows the window by id across the refresh.
fn move_selected_column(state: &Rc<RefCell<State>>, right: bool) -> bool {
    let win_id = match state.borrow().selected_win_id() {
        Some(id) => id,
        None => return false,
    };

    // Moving requires focusing a window in the column, so restore focus after.
    let focus = capture_focus();
    let _ = niri::move_column(win_id, right);
    restore_focus(focus);
    refresh(state);
    true
}

/// Move the selected workspace to the monitor on the left/right. The selection
/// follows the workspace by id across the refresh, so it lands on the new screen.
fn move_selected_ws_to_monitor(state: &Rc<RefCell<State>>, left: bool) -> bool {
    let target = match state.borrow().selected_ws_id() {
        Some(id) => id,
        None => return false,
    };
    let focus = capture_focus();
    if niri::focus_workspace_by_id(target).is_ok() {
        let _ = niri::move_workspace_to_monitor(left);
    }
    restore_focus(focus);
    refresh(state);
    true
}

/// Jump the selection to the first workspace of the next/previous output,
/// wrapping around. With two screens this just toggles between them.
fn move_output(state: &Rc<RefCell<State>>, delta: i32) -> bool {
    let mut s = state.borrow_mut();
    let n_out = s.model.outputs.len();
    if n_out < 2 {
        return false;
    }
    let cur = s.sel_output();
    let next = ((cur as i32 + delta).rem_euclid(n_out as i32)) as usize;
    if let Some(idx) = s.model.nav.iter().position(|&(o, _)| o == next) {
        if idx != s.sel_nav {
            s.sel_nav = idx;
            s.sel_win = 0;
            return true;
        }
    }
    false
}

fn move_win(state: &Rc<RefCell<State>>, delta: i32) -> bool {
    let mut s = state.borrow_mut();
    let count = s.sel_ws().map(|v| v.windows.len()).unwrap_or(0);
    if count == 0 {
        return false;
    }
    let cur = s.sel_win as i32;
    let next = (cur + delta).clamp(0, count as i32 - 1) as usize;
    if next != s.sel_win {
        s.sel_win = next;
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

const PAD: f64 = 18.0;
const FOOTER_H: f64 = 30.0;
const OUTPUT_HEADER_H: f64 = 26.0;
const WS_GAP: f64 = 10.0;
const WS_HEADER_H: f64 = 22.0;

fn set_rgba(cr: &gtk::cairo::Context, r: f64, g: f64, b: f64, a: f64) {
    cr.set_source_rgba(r, g, b, a);
}

/// Truncate `text` with an ellipsis so it fits within `max_w`.
fn fit_text(cr: &gtk::cairo::Context, text: &str, max_w: f64) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    let fits = |s: &str| {
        cr.text_extents(s)
            .map(|e| e.width() <= max_w)
            .unwrap_or(true)
    };
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut hi = chars.len();
    while hi > 0 {
        hi -= 1;
        let candidate: String = chars[..hi].iter().collect::<String>() + "…";
        if fits(&candidate) {
            return candidate;
        }
    }
    "…".to_string()
}

fn text_at(cr: &gtk::cairo::Context, x: f64, y: f64, s: &str) {
    cr.move_to(x, y);
    let _ = cr.show_text(s);
}

fn draw(cr: &gtk::cairo::Context, w: f64, h: f64, state: &State) {
    // Opaque backdrop (a layer surface can't be forced opaque from niri config,
    // so the app draws it fully opaque for readability).
    set_rgba(cr, 0.06, 0.07, 0.10, 1.0);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );

    draw_footer(cr, w, h);

    if let Some(err) = &state.error {
        set_rgba(cr, 1.0, 0.45, 0.45, 1.0);
        cr.set_font_size(16.0);
        text_at(cr, PAD, PAD + 16.0, &format!("niri error: {err}"));
        return;
    }

    let outputs = &state.model.outputs;
    if outputs.is_empty() {
        set_rgba(cr, 0.8, 0.8, 0.85, 1.0);
        cr.set_font_size(16.0);
        text_at(cr, PAD, PAD + 16.0, "no workspaces found");
        return;
    }

    let sel = state.model.nav.get(state.sel_nav).copied();

    let content_x = PAD;
    let content_y = PAD;
    let content_w = w - 2.0 * PAD;
    let content_h = (h - PAD - FOOTER_H) - content_y;

    // Place outputs by their real horizontal position and proportional width,
    // so the left/right arrangement is faithful. Vertically the axes are
    // decoupled: tops align to the content top and every output uses the full
    // height (the configured y-offset between screens isn't reproduced — it'd
    // just waste space in a survey view).
    let min_x = outputs.iter().map(|o| o.x).fold(f64::INFINITY, f64::min);
    let max_x = outputs.iter().map(|o| o.x + o.w).fold(f64::NEG_INFINITY, f64::max);
    let span_w = (max_x - min_x).max(1.0);
    let scale_x = content_w / span_w;

    for (i, output) in outputs.iter().enumerate() {
        let ox = content_x + (output.x - min_x) * scale_x;
        let oy = content_y;
        let ow = output.w * scale_x;
        let oh = content_h;

        // Faint outline of the screen's extent.
        set_rgba(cr, 1.0, 1.0, 1.0, 0.05);
        cr.rectangle(ox, oy, ow, oh);
        let _ = cr.fill();

        // Output header inside the top of its rectangle.
        set_rgba(cr, 0.62, 0.70, 0.85, 1.0);
        cr.set_font_size(15.0);
        text_at(cr, ox + 2.0, oy + 16.0, &fit_text(cr, &output.name, ow - 4.0));

        let wy0 = oy + OUTPUT_HEADER_H;
        let avail_h = oh - OUTPUT_HEADER_H;
        let m = output.workspaces.len();
        if m == 0 {
            continue;
        }
        let ws_h = ((avail_h - (m as f64 - 1.0) * WS_GAP) / m as f64).max(28.0);

        for (j, wsv) in output.workspaces.iter().enumerate() {
            let wy = wy0 + j as f64 * (ws_h + WS_GAP);
            let ws_selected = sel == Some((i, j));
            draw_workspace(cr, ox, wy, ow, ws_h, wsv, ws_selected, state.sel_win);
        }
    }

    if let Some(edit) = &state.editing {
        draw_rename(cr, w, h, edit);
    }
}

/// A centered modal text field for renaming the selected workspace, with a
/// caret drawn at the edit cursor.
fn draw_rename(cr: &gtk::cairo::Context, w: f64, h: f64, edit: &Edit) {
    // Dim everything behind the field.
    set_rgba(cr, 0.0, 0.0, 0.0, 0.45);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    let bw = (w * 0.5).clamp(320.0, 560.0);
    let bh = 92.0;
    let bx = (w - bw) / 2.0;
    let by = (h - bh) / 2.0;

    set_rgba(cr, 0.12, 0.14, 0.20, 0.98);
    cr.rectangle(bx, by, bw, bh);
    let _ = cr.fill();
    set_rgba(cr, 0.30, 0.66, 1.0, 1.0);
    cr.set_line_width(2.0);
    cr.rectangle(bx, by, bw, bh);
    let _ = cr.stroke();

    set_rgba(cr, 0.60, 0.66, 0.78, 1.0);
    cr.set_font_size(12.0);
    text_at(
        cr,
        bx + 16.0,
        by + 24.0,
        "Rename workspace  (Enter: confirm · Esc: cancel · empty: unset)",
    );

    let text_x = bx + 16.0;
    let text_y = by + 62.0;
    cr.set_font_size(20.0);

    // Clip to the field so a long name can't spill past the border.
    let _ = cr.save();
    cr.rectangle(bx + 8.0, by + 32.0, bw - 16.0, bh - 40.0);
    cr.clip();

    set_rgba(cr, 0.95, 0.96, 0.99, 1.0);
    text_at(cr, text_x, text_y, &edit.text());

    // Caret: a thin vertical bar at the cursor's x-advance.
    let prefix: String = edit.buf[..edit.cursor].iter().collect();
    let caret_x = text_x + cr.text_extents(&prefix).map(|e| e.x_advance()).unwrap_or(0.0);
    set_rgba(cr, 0.40, 0.80, 1.0, 1.0);
    cr.set_line_width(1.5);
    cr.move_to(caret_x, text_y - 17.0);
    cr.line_to(caret_x, text_y + 4.0);
    let _ = cr.stroke();

    let _ = cr.restore();
}

#[allow(clippy::too_many_arguments)]
fn draw_workspace(
    cr: &gtk::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    wsv: &WsView,
    selected: bool,
    sel_win: usize,
) {
    // Workspace background.
    set_rgba(cr, 0.13, 0.14, 0.18, 0.85);
    cr.rectangle(x, y, w, h);
    let _ = cr.fill();

    // Border: accent if selected, faint otherwise.
    if selected {
        set_rgba(cr, 0.30, 0.66, 1.0, 1.0);
        cr.set_line_width(2.5);
    } else if wsv.ws.is_focused {
        set_rgba(cr, 0.45, 0.55, 0.65, 0.9);
        cr.set_line_width(1.5);
    } else {
        set_rgba(cr, 1.0, 1.0, 1.0, 0.08);
        cr.set_line_width(1.0);
    }
    cr.rectangle(x, y, w, h);
    let _ = cr.stroke();

    // Workspace header label.
    cr.set_font_size(13.0);
    if wsv.ws.is_urgent {
        set_rgba(cr, 1.0, 0.6, 0.4, 1.0);
    } else if selected {
        set_rgba(cr, 0.55, 0.80, 1.0, 1.0);
    } else {
        set_rgba(cr, 0.85, 0.88, 0.94, 1.0);
    }
    let header = format!("{}  ({} win)", wsv.ws.label(), wsv.windows.len());
    text_at(cr, x + 7.0, y + 15.0, &fit_text(cr, &header, w - 14.0));

    // Window area.
    let inner_x = x + 6.0;
    let inner_y = y + WS_HEADER_H;
    let inner_w = w - 12.0;
    let inner_h = h - WS_HEADER_H - 6.0;

    if wsv.windows.is_empty() {
        set_rgba(cr, 0.5, 0.5, 0.55, 0.8);
        cr.set_font_size(12.0);
        text_at(cr, inner_x + 4.0, inner_y + 18.0, "(empty)");
        return;
    }

    // Group windows into columns (preserving the sorted order = linear index).
    let mut columns: Vec<Vec<(usize, &niri::Window)>> = Vec::new();
    let mut last_col: Option<i64> = None;
    for (idx, win) in wsv.windows.iter().enumerate() {
        if last_col != Some(win.column()) {
            columns.push(Vec::new());
            last_col = Some(win.column());
        }
        columns.last_mut().unwrap().push((idx, win));
    }

    let col_count = columns.len();
    let cw = inner_w / col_count as f64;
    for (k, column) in columns.iter().enumerate() {
        let cx = inner_x + k as f64 * cw;
        let rows = column.len();
        let rh = inner_h / rows as f64;
        for (r, (lin_idx, win)) in column.iter().enumerate() {
            let ry = inner_y + r as f64 * rh;
            let win_selected = selected && *lin_idx == sel_win;
            draw_window(cr, cx + 2.0, ry + 2.0, cw - 4.0, rh - 4.0, win, win_selected);
        }
    }
}

fn draw_window(
    cr: &gtk::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    win: &niri::Window,
    selected: bool,
) {
    if w <= 2.0 || h <= 2.0 {
        return;
    }

    if selected {
        set_rgba(cr, 0.30, 0.66, 1.0, 0.92);
    } else if win.is_focused {
        set_rgba(cr, 0.26, 0.30, 0.40, 0.95);
    } else {
        set_rgba(cr, 0.20, 0.22, 0.27, 0.95);
    }
    cr.rectangle(x, y, w, h);
    let _ = cr.fill();

    // Border.
    if win.is_urgent {
        set_rgba(cr, 1.0, 0.45, 0.35, 1.0);
        cr.set_line_width(1.8);
    } else {
        set_rgba(cr, 1.0, 1.0, 1.0, 0.12);
        cr.set_line_width(1.0);
    }
    cr.rectangle(x, y, w, h);
    let _ = cr.stroke();

    // Labels: app id (small) then title.
    let text_w = w - 12.0;
    if text_w <= 0.0 {
        return;
    }

    if selected {
        set_rgba(cr, 0.04, 0.08, 0.14, 1.0);
    } else {
        set_rgba(cr, 0.66, 0.72, 0.82, 1.0);
    }
    cr.set_font_size(10.0);
    if let Some(app_id) = &win.app_id {
        text_at(cr, x + 6.0, y + 14.0, &fit_text(cr, app_id, text_w));
    }

    if selected {
        set_rgba(cr, 0.02, 0.05, 0.10, 1.0);
    } else {
        set_rgba(cr, 0.90, 0.92, 0.96, 1.0);
    }
    cr.set_font_size(12.0);
    if h > 26.0 {
        text_at(cr, x + 6.0, y + 30.0, &fit_text(cr, &win.label(), text_w));
    }
}

fn draw_footer(cr: &gtk::cairo::Context, w: f64, h: f64) {
    set_rgba(cr, 0.60, 0.66, 0.78, 1.0);
    cr.set_font_size(13.0);
    let help = "j/k ws · J/K move ws · H/L ws to screen · h/l win · ^H/^L move col · Tab screen · Enter focus · r rename · w kill ws · x kill win · q quit";
    let fitted = fit_text(cr, help, w - 2.0 * PAD);
    text_at(cr, PAD, h - 10.0, &fitted);
}
