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

/// One output (monitor) and its workspaces, sorted by index.
struct OutputView {
    name: String,
    workspaces: Vec<WsView>,
}

/// The full picture I draw, plus a flat navigation order over workspaces.
struct Model {
    outputs: Vec<OutputView>,
    /// `(output index, workspace index within output)` in display order.
    nav: Vec<(usize, usize)>,
}

struct State {
    model: Model,
    /// Index into `model.nav` — the currently selected workspace.
    sel_nav: usize,
    /// Index into the selected workspace's `windows`.
    sel_win: usize,
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

    // Group workspaces by output (sorted by output name for stability).
    let mut by_output: BTreeMap<String, Vec<niri::Workspace>> = BTreeMap::new();
    for ws in workspaces {
        let out = ws.output.clone().unwrap_or_else(|| "?".to_string());
        by_output.entry(out).or_default().push(ws);
    }

    let mut outputs = Vec::new();
    let mut nav = Vec::new();
    for (name, mut wss) in by_output {
        wss.sort_by_key(|w| w.idx);
        let o_idx = outputs.len();
        let mut workspaces = Vec::new();
        for ws in wss {
            let mut wins = by_ws.remove(&ws.id).unwrap_or_default();
            wins.sort_by_key(|w| (w.column(), w.row(), w.id));
            nav.push((o_idx, workspaces.len()));
            workspaces.push(WsView { ws, windows: wins });
        }
        outputs.push(OutputView { name, workspaces });
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
    let model = build_model().unwrap_or(Model {
        outputs: Vec::new(),
        nav: Vec::new(),
    });
    let state = Rc::new(RefCell::new(State {
        model,
        sel_nav: 0,
        sel_win: 0,
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

    // Make the toplevel transparent so the cairo backdrop blends with the
    // desktop behind, like niri's overview.
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
        key.connect_key_pressed(move |_ctrl, keyval, _code, _mods| {
            let handled = handle_key(&keyval, &state, &app);
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

/// Returns true if the key changed something and a redraw is wanted.
fn handle_key(keyval: &gdk::Key, state: &Rc<RefCell<State>>, app: &Application) -> bool {
    let ch = keyval.to_unicode();
    match (ch, *keyval) {
        (Some('q'), _) | (_, gdk::Key::Escape) => {
            app.quit();
            false
        }
        (Some('r'), _) => {
            refresh(state);
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
        // Remember the compositor's focused workspace; moving requires focusing,
        // so I restore it afterwards to leave focus where the user left it.
        let prev_focus = niri::fetch_workspaces()
            .ok()
            .and_then(|wss| wss.into_iter().find(|w| w.is_focused).map(|w| w.id));

        let _ = niri::move_workspace(&output, idx, down);

        if let Some(id) = prev_focus {
            let _ = niri::focus_workspace_by_id(id);
        }
        refresh(state);
        true
    } else {
        false
    }
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
const OUT_GAP: f64 = 16.0;
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
    // Translucent backdrop.
    set_rgba(cr, 0.06, 0.07, 0.10, 0.92);
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
    let content_bottom = h - PAD - FOOTER_H;

    let n = outputs.len();
    let col_w = (content_w - (n as f64 - 1.0) * OUT_GAP) / n as f64;

    for (i, output) in outputs.iter().enumerate() {
        let ox = content_x + i as f64 * (col_w + OUT_GAP);

        // Output header.
        set_rgba(cr, 0.62, 0.70, 0.85, 1.0);
        cr.set_font_size(15.0);
        text_at(
            cr,
            ox,
            content_y + 16.0,
            &fit_text(cr, &output.name, col_w),
        );

        let wy0 = content_y + OUTPUT_HEADER_H;
        let avail_h = content_bottom - wy0;
        let m = output.workspaces.len();
        if m == 0 {
            continue;
        }
        let ws_h = ((avail_h - (m as f64 - 1.0) * WS_GAP) / m as f64).max(40.0);

        for (j, wsv) in output.workspaces.iter().enumerate() {
            let wy = wy0 + j as f64 * (ws_h + WS_GAP);
            let ws_selected = sel == Some((i, j));
            draw_workspace(cr, ox, wy, col_w, ws_h, wsv, ws_selected, state.sel_win);
        }
    }
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
    let help = "j/k: workspace   J/K: move workspace   h/l: window   Tab: screen   Enter: focus   w: kill workspace   x: kill window   r: refresh   q/Esc: quit";
    let fitted = fit_text(cr, help, w - 2.0 * PAD);
    text_at(cr, PAD, h - 10.0, &fitted);
}
