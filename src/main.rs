//! niri-groom — a fullscreen layer-shell overlay that surveys niri workspaces
//! and windows as a proportional map, and lets me kill a whole workspace (`w`)
//! or a single window (`x`) with no confirmation.

mod badges;
mod config;
mod niri;
mod theme;

use crate::theme::{Rgb, Theme};
use gtk4 as gtk;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, DrawingArea, EventControllerKey};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::BufRead;
use std::rc::Rc;
use std::time::Duration;

const APP_ID: &str = "io.iwd.niri-groom";
/// Default layer-shell namespace (niri matches the surface by this); `--app-id`
/// overrides it.
const APP_NAMESPACE: &str = "niri-groom";

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

/// What the pointer is dragging.
#[derive(Clone)]
enum DragKind {
    /// A whole workspace (grabbed by its header), identified by id.
    Workspace { id: u64 },
    /// A column within a workspace: the source workspace + the niri column index,
    /// plus a representative window id to focus the column on drop.
    Column { ws_id: u64, col: i64, win_id: u64 },
}

/// Where a drag would land if dropped now.
#[derive(Clone, Copy, PartialEq)]
enum DropTarget {
    /// Insert the workspace into output `o` at slot `idx` (0-based among the
    /// other workspaces there).
    Workspace { o: usize, idx: usize },
    /// Insert the column into workspace (o, wi) at column slot `idx`.
    Column { o: usize, wi: usize, idx: usize },
}

struct Drag {
    kind: DragKind,
    /// Pointer offset within the grabbed item's rect, so it doesn't jump.
    grab: (f64, f64),
    /// Press point and current pointer, in widget coordinates.
    start: (f64, f64),
    cursor: (f64, f64),
    /// True once the pointer has moved past the start threshold.
    active: bool,
    target: Option<DropTarget>,
}

struct State {
    model: Model,
    /// Index into `model.nav` — the currently selected workspace.
    sel_nav: usize,
    /// Index into the selected workspace's `windows`.
    sel_win: usize,
    /// When renaming, the in-overlay line editor for the selected workspace.
    editing: Option<Edit>,
    /// All bundled themes, and the index of the saved (committed) one.
    themes: Vec<Theme>,
    theme_idx: usize,
    /// When the theme picker is open, the highlighted theme index (applied live).
    picker: Option<usize>,
    /// Whether the key legend panel is shown (toggled with `?`).
    show_help: bool,
    /// Whether the overlay currently has keyboard focus. When it doesn't (e.g.
    /// it's left running as a map on another monitor), the groom selection is
    /// hidden so only niri's own focus highlight remains.
    active: bool,
    /// When `Some(o)`, "solo" mode: only output `o` is shown, full-width.
    solo: Option<usize>,
    /// Optional command that supplies per-workspace badges (e.g. my niri
    /// bookmarks), and its last-fetched result keyed by lowercased name.
    badge_cmd: Option<String>,
    badges: HashMap<String, badges::Badge>,
    /// In-progress pointer drag (freezes refresh so the layout stays put).
    drag: Option<Drag>,
    /// Eased on-screen top-left per workspace id and per column, so neighbours
    /// slide to open a gap during a drag instead of jumping.
    anim_ws: HashMap<u64, (f64, f64)>,
    anim_col: HashMap<(u64, i64), (f64, f64)>,
    error: Option<String>,
}

impl State {
    /// The theme to draw with: the picker's live highlight if open, else the
    /// saved one.
    fn theme(&self) -> &Theme {
        &self.themes[self
            .picker
            .unwrap_or(self.theme_idx)
            .min(self.themes.len() - 1)]
    }
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
        self.model
            .nav
            .get(self.sel_nav)
            .map(|&(o, _)| o)
            .unwrap_or(0)
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
        outputs.push(OutputView {
            name,
            workspaces,
            x,
            y,
            w,
        });
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
    // Don't rebuild the model mid-drag — the grabbed indices/geometry must stay
    // stable until the drop is applied.
    if state.borrow().drag.is_some() {
        return;
    }

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

    // Re-pull badges (cheap external command) so a freshly-bookmarked workspace
    // shows up without restarting the overlay.
    if let Some(cmd) = s.badge_cmd.clone() {
        s.badges = badges::load(&cmd);
    }

    // Re-find the previously selected workspace.
    if let Some(ws_id) = prev_ws_id {
        if let Some(idx) = s
            .model
            .nav
            .iter()
            .position(|&(o, w)| s.model.outputs[o].workspaces[w].ws.id == ws_id)
        {
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

/// Parsed command-line options.
struct Opts {
    /// Start in solo mode on the output with this name (if it exists).
    solo_monitor: Option<String>,
    /// Place the overlay surface on this output (niri can't position a layer
    /// surface from config, so the client must request it).
    output: Option<String>,
    /// The layer-shell namespace (what niri matches the surface by). `--app-id`
    /// sets it, so niri config can target a given instance for its rules.
    namespace: String,
    /// `--toggle`: a second launch of the same instance closes it instead of
    /// re-presenting, so one keybind opens and closes the overlay.
    toggle: bool,
    /// `--focus`: move niri's focus onto the overlay's output at launch, so the
    /// exclusive-keyboard surface grabs the keyboard even when opened elsewhere.
    focus: bool,
}

fn parse_args() -> Opts {
    let argv: Vec<String> = std::env::args().collect();
    let mut namespace = APP_NAMESPACE.to_string();
    let mut solo_monitor = None;
    let mut output = None;
    let mut toggle = false;
    let mut focus = false;
    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].clone();
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (arg.as_str(), None),
        };
        if key == "--toggle" {
            toggle = true;
        } else if key == "--focus" {
            focus = true;
        } else if matches!(key, "--app-id" | "--solo" | "--open-on-monitor") {
            let val = if inline.is_some() {
                inline
            } else {
                i += 1;
                argv.get(i).cloned()
            };
            match key {
                "--app-id" => {
                    if let Some(v) = val {
                        namespace = v;
                    }
                }
                "--solo" => solo_monitor = val,
                "--open-on-monitor" => output = val,
                _ => unreachable!(),
            }
        }
        i += 1;
    }
    Opts {
        solo_monitor,
        output,
        namespace,
        toggle,
        focus,
    }
}

/// A valid, unique GApplication id derived from the namespace (single-instance
/// is keyed on it, so distinct namespaces must yield distinct ids).
fn derive_app_id(namespace: &str) -> String {
    let is_valid = |s: &str| gtk::gio::Application::id_is_valid(s);
    if is_valid(namespace) {
        return namespace.to_string();
    }
    let candidate = format!("io.iwd.{namespace}");
    if is_valid(&candidate) {
        candidate
    } else {
        APP_ID.to_string()
    }
}

fn main() -> glib::ExitCode {
    let opts = parse_args();
    let app = Application::builder()
        .application_id(derive_app_id(&opts.namespace))
        .build();
    let opts = Rc::new(opts);
    app.connect_activate(move |a| build_ui(a, &opts));
    // Don't let GApplication try to parse our own flags.
    let argv0 = std::env::args()
        .next()
        .unwrap_or_else(|| "niri-groom".into());
    app.run_with_args(&[argv0])
}

/// The gdk monitor whose connector matches `name`.
fn gdk_monitor_by_name(name: &str) -> Option<gdk::Monitor> {
    let display = gdk::Display::default()?;
    let monitors = display.monitors();
    for i in 0..monitors.n_items() {
        if let Some(mon) = monitors
            .item(i)
            .and_then(|o| o.downcast::<gdk::Monitor>().ok())
        {
            if mon.connector().as_deref() == Some(name) {
                return Some(mon);
            }
        }
    }
    None
}

fn build_ui(app: &Application, opts: &Opts) {
    // Single-instance: GApplication forwards a second launch's `activate` to the
    // already-running instance (so a second keybind press can't stack a second
    // exclusive-keyboard overlay and deadlock input). With `--toggle` that second
    // press closes the overlay; otherwise it just re-presents the existing one.
    if let Some(win) = app.windows().into_iter().next() {
        if opts.toggle {
            app.quit();
        } else {
            win.present();
        }
        return;
    }

    let model = build_model().unwrap_or(Model {
        outputs: Vec::new(),
        nav: Vec::new(),
    });
    let themes = theme::all();
    let theme_idx = config::load_theme()
        .and_then(|name| theme::index_of(&name))
        .unwrap_or(0);
    let badge_cmd = config::load_badge_command();
    let badges = badge_cmd.as_deref().map(badges::load).unwrap_or_default();
    // --solo <name>: start with only that output shown, if it exists.
    let solo = opts
        .solo_monitor
        .as_ref()
        .and_then(|name| model.outputs.iter().position(|o| &o.name == name));
    let state = Rc::new(RefCell::new(State {
        model,
        sel_nav: 0,
        sel_win: 0,
        editing: None,
        themes,
        theme_idx,
        picker: None,
        show_help: false,
        // Corrected from the real focus state just after present() — the overlay
        // may open on a non-focused output, where it never gains focus.
        active: false,
        solo,
        badge_cmd,
        badges,
        drag: None,
        anim_ws: HashMap::new(),
        anim_col: HashMap::new(),
        error: None,
    }));
    // Start the selection on the solo'd output (if any), else where focus is.
    {
        let mut s = state.borrow_mut();
        s.sel_nav = match solo {
            Some(o) => s
                .model
                .nav
                .iter()
                .position(|&(oo, _)| oo == o)
                .unwrap_or_else(|| s.focused_nav()),
            None => s.focused_nav(),
        };
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
    window.set_namespace(Some(&opts.namespace));
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
        window.set_anchor(edge, true);
    }
    // --open-on-monitor: niri can't place a layer surface from config, so the
    // client requests the output here.
    if let Some(mon) = opts
        .output
        .as_ref()
        .and_then(|name| gdk_monitor_by_name(name))
    {
        window.set_monitor(Some(&mon));
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

    // Mouse drag-and-drop: grab a workspace header to reorder/move it, or a
    // column body to move it. Applied via niri actions on drop.
    let drag_gesture = gtk::GestureDrag::new();
    {
        let state = state.clone();
        let area = area.clone();
        drag_gesture.connect_drag_begin(move |_g, x, y| {
            let w = area.width() as f64;
            let h = area.height() as f64;
            let hit = {
                let s = state.borrow();
                if s.editing.is_some() || s.picker.is_some() {
                    None
                } else {
                    let layout = compute_layout(&s.model, w, h, s.solo);
                    hit_workspace_header(&layout, &s.model, x, y)
                        .or_else(|| hit_column(&layout, &s.model, x, y))
                }
            };
            if let Some((kind, rect)) = hit {
                state.borrow_mut().drag = Some(Drag {
                    kind,
                    grab: (x - rect.0, y - rect.1),
                    start: (x, y),
                    cursor: (x, y),
                    active: false,
                    target: None,
                });
            }
        });
    }
    {
        let state = state.clone();
        let area = area.clone();
        drag_gesture.connect_drag_update(move |_g, ox, oy| {
            let w = area.width() as f64;
            let h = area.height() as f64;
            let mut start_tick = false;
            {
                let mut s = state.borrow_mut();
                let Some(d) = s.drag.as_mut() else {
                    return;
                };
                d.cursor = (d.start.0 + ox, d.start.1 + oy);
                if !d.active && (ox * ox + oy * oy).sqrt() > DRAG_THRESHOLD {
                    d.active = true;
                    start_tick = true;
                }
            }
            if start_tick {
                let state = state.clone();
                let area2 = area.clone();
                area.add_tick_callback(move |a, _| {
                    let w = a.width() as f64;
                    let h = a.height() as f64;
                    if animate_step(&state, w, h) {
                        area2.queue_draw();
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                });
            }
            recompute_target(&state, w, h);
            area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let area = area.clone();
        drag_gesture.connect_drag_end(move |_g, _ox, _oy| {
            let outcome = {
                let s = state.borrow();
                s.drag
                    .as_ref()
                    .map(|d| (d.active, d.kind.clone(), d.target))
            };
            match outcome {
                Some((true, kind, Some(target))) => apply_drop(&state, &kind, target),
                _ => {
                    let mut s = state.borrow_mut();
                    s.drag = None;
                    s.anim_ws.clear();
                    s.anim_col.clear();
                }
            }
            area.queue_draw();
        });
    }
    area.add_controller(drag_gesture);

    // Track keyboard focus so the groom selection can hide when the overlay is
    // left running unfocused (e.g. as a map on a second monitor).
    {
        let state = state.clone();
        let area = area.clone();
        window.connect_is_active_notify(move |win| {
            state.borrow_mut().active = win.is_active();
            area.queue_draw();
        });
    }

    window.set_child(Some(&area));
    window.present();

    // Once the surface has settled: optionally move niri's focus onto the
    // overlay's output (--focus), then capture the real focus state. The overlay
    // can open on a non-focused output where it never gains focus, and is-active
    // only fires on changes — so without this the selection state would be wrong.
    // (Ongoing changes use the notify above.)
    {
        let state = state.clone();
        let area = area.clone();
        let window = window.clone();
        let app = app.clone();
        let want_focus = opts.focus;
        glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
            if want_focus {
                if let Some(out) = overlay_output(&app) {
                    let _ = niri::focus_monitor(&out);
                }
            }
            state.borrow_mut().active = window.is_active();
            area.queue_draw();
        });
    }

    // Refresh on niri's event stream: a background thread reads one JSON line
    // per change and pings a bounded(1) channel; the channel coalesces bursts
    // (extra pings are dropped while one is queued) so the UI re-fetches once,
    // near-instantly, per change instead of polling.
    {
        let (tx, rx) = async_channel::bounded::<()>(1);
        std::thread::spawn(move || {
            let Some(mut child) = niri::spawn_event_stream() else {
                return;
            };
            if let Some(out) = child.stdout.take() {
                let reader = std::io::BufReader::new(out);
                for line in reader.lines() {
                    if line.is_err() {
                        break;
                    }
                    let _ = tx.try_send(());
                }
            }
            let _ = child.wait();
        });

        let state = state.clone();
        let area = area.clone();
        glib::spawn_future_local(async move {
            while rx.recv().await.is_ok() {
                refresh(&state);
                area.queue_draw();
            }
        });
    }

    // Slow fallback poll, in case the event stream is unavailable or misses a
    // change niri doesn't emit an event for (e.g. an output reconfiguration).
    {
        let state = state.clone();
        let area = area.clone();
        glib::timeout_add_local(Duration::from_millis(2000), move || {
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

/// The connector name (e.g. "HDMI-A-1") of the monitor the overlay is on, to
/// compare against a target workspace's output.
fn overlay_output(app: &Application) -> Option<String> {
    let window = app.windows().into_iter().next()?;
    let surface = window.surface()?;
    let display = gtk::prelude::WidgetExt::display(&window);
    let monitor = display.monitor_at_surface(&surface)?;
    monitor.connector().map(|s| s.to_string())
}

/// Returns true if the key changed something and a redraw is wanted.
fn handle_key(
    keyval: &gdk::Key,
    mods: gdk::ModifierType,
    state: &Rc<RefCell<State>>,
    app: &Application,
) -> bool {
    // While a modal is open it captures the keyboard.
    if state.borrow().editing.is_some() {
        return handle_edit_key(keyval, mods, state);
    }
    if state.borrow().picker.is_some() {
        return handle_picker_key(keyval, state);
    }

    let ch = keyval.to_unicode();

    // `?` toggles the key legend; while it's up, Esc closes it (rather than quit).
    if ch == Some('?') {
        let mut s = state.borrow_mut();
        s.show_help = !s.show_help;
        return true;
    }
    if state.borrow().show_help && matches!(*keyval, gdk::Key::Escape) {
        state.borrow_mut().show_help = false;
        return true;
    }

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
        // Open the theme picker.
        (Some('t'), _) => {
            let idx = state.borrow().theme_idx;
            state.borrow_mut().picker = Some(idx);
            true
        }
        // Solo the selected monitor (toggle): show only it, full-width.
        (Some('s'), _) => {
            let mut s = state.borrow_mut();
            s.solo = if s.solo.is_some() {
                None
            } else {
                Some(s.sel_output())
            };
            true
        }
        // Focus the selected window (or the workspace if it's empty), then jump
        // to it. Only dismiss the overlay if the target is on the overlay's own
        // monitor — otherwise the overlay stays as a map on its screen while you
        // work on the other one.
        (_, gdk::Key::Return) | (_, gdk::Key::KP_Enter) => {
            let (win_id, ws_id, target_output) = {
                let s = state.borrow();
                (
                    s.selected_win_id(),
                    s.selected_ws_id(),
                    s.sel_ws().and_then(|v| v.ws.output.clone()),
                )
            };
            if let Some(id) = win_id {
                let _ = niri::focus_window(id);
            } else if let Some(id) = ws_id {
                let _ = niri::focus_workspace_by_id(id);
            }
            let same_monitor = match (overlay_output(app), target_output) {
                (Some(overlay), Some(target)) => overlay == target,
                // Unknown monitor → behave as before (dismiss).
                _ => true,
            };
            if same_monitor {
                app.quit();
                false
            } else {
                // Keep the overlay; losing focus flips `is_active` and hides the
                // selection, so it reads as a passive map.
                true
            }
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
    // Candidate nav indices: all, or only the solo output's.
    let solo = s.solo;
    let nav: Vec<usize> = (0..s.model.nav.len())
        .filter(|&i| solo.is_none_or(|o| s.model.nav[i].0 == o))
        .collect();
    if nav.is_empty() {
        return false;
    }
    let pos = nav.iter().position(|&i| i == s.sel_nav).unwrap_or(0) as i32;
    let next = nav[(pos + delta).clamp(0, nav.len() as i32 - 1) as usize];
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
            // In solo mode, switching screens swaps which one is shown.
            if s.solo.is_some() {
                s.solo = Some(next);
            }
            return true;
        }
    }
    false
}

/// Handle a keystroke while the theme picker is open. Up/Down (or k/j) move and
/// apply the theme live; Enter saves it to the config; Esc cancels and reverts.
fn handle_picker_key(keyval: &gdk::Key, state: &Rc<RefCell<State>>) -> bool {
    match *keyval {
        gdk::Key::Escape => {
            state.borrow_mut().picker = None;
            true
        }
        gdk::Key::Return | gdk::Key::KP_Enter => {
            let sel = state.borrow().picker;
            if let Some(i) = sel {
                let name = state.borrow().themes[i].name;
                config::save_theme(name);
                let mut s = state.borrow_mut();
                s.theme_idx = i;
                s.picker = None;
            }
            true
        }
        gdk::Key::Down => move_picker(state, 1),
        gdk::Key::Up => move_picker(state, -1),
        _ => match keyval.to_unicode() {
            Some('j') => move_picker(state, 1),
            Some('k') => move_picker(state, -1),
            Some('q') => {
                state.borrow_mut().picker = None;
                true
            }
            _ => false,
        },
    }
}

fn move_picker(state: &Rc<RefCell<State>>, delta: i32) -> bool {
    let mut s = state.borrow_mut();
    let n = s.themes.len() as i32;
    if let Some(cur) = s.picker {
        let next = ((cur as i32 + delta).rem_euclid(n)) as usize;
        if next != cur {
            s.picker = Some(next);
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

const PAD: f64 = 20.0;
/// Reserved strip at the bottom for the small "? keys" hint.
const FOOTER_H: f64 = 22.0;
const OUTPUT_HEADER_H: f64 = 30.0;
const WS_GAP: f64 = 12.0;
const WS_HEADER_H: f64 = 28.0;

fn set_rgba(cr: &gtk::cairo::Context, r: f64, g: f64, b: f64, a: f64) {
    cr.set_source_rgba(r, g, b, a);
}

/// Set the source to a theme color at alpha `a`.
fn set(cr: &gtk::cairo::Context, c: Rgb, a: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, a);
}

/// Trace a rounded-rectangle path (caller fills/strokes it).
fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    use std::f64::consts::PI;
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
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

/// A column's on-screen slot within a workspace card.
struct ColLayout {
    /// niri column index (1-based) — used for `move-column-to-index`.
    #[allow(dead_code)]
    col: i64,
    /// Linear indices into the workspace's `windows`, top to bottom.
    win_lin: Vec<usize>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// A workspace card's on-screen rect, with its columns.
struct WsLayout {
    o: usize,
    wi: usize,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cols: Vec<ColLayout>,
}

struct OutLayout {
    o: usize,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// The positioned boxes for the whole map. Computed once per frame and shared by
/// rendering and (later) pointer hit-testing, so geometry lives in one place.
struct Layout {
    outputs: Vec<OutLayout>,
    workspaces: Vec<WsLayout>,
}

/// Lay out one output's workspace cards (and their columns) into `rect`.
fn layout_output(model: &Model, o: usize, rect: (f64, f64, f64, f64), layout: &mut Layout) {
    let (ox, oy, ow, oh) = rect;
    let output = &model.outputs[o];
    let wx = ox + 8.0;
    let ww = ow - 16.0;
    let wy0 = oy + OUTPUT_HEADER_H;
    let avail_h = oh - OUTPUT_HEADER_H - 8.0;
    let m = output.workspaces.len();
    if m == 0 {
        return;
    }
    let ws_h = ((avail_h - (m as f64 - 1.0) * WS_GAP) / m as f64).max(28.0);

    for (j, wsv) in output.workspaces.iter().enumerate() {
        let wy = wy0 + j as f64 * (ws_h + WS_GAP);
        let mut cols: Vec<ColLayout> = Vec::new();
        if !wsv.windows.is_empty() {
            let inner_x = wx + 9.0;
            let inner_y = wy + WS_HEADER_H;
            let inner_w = ww - 18.0;
            let inner_h = ws_h - WS_HEADER_H - 9.0;

            // Group consecutive windows sharing a niri column.
            let mut groups: Vec<(i64, Vec<usize>)> = Vec::new();
            let mut last: Option<i64> = None;
            for (idx, win) in wsv.windows.iter().enumerate() {
                let c = win.column();
                if last != Some(c) {
                    groups.push((c, Vec::new()));
                    last = Some(c);
                }
                groups.last_mut().unwrap().1.push(idx);
            }
            let cw = inner_w / groups.len() as f64;
            for (k, (col, lins)) in groups.into_iter().enumerate() {
                cols.push(ColLayout {
                    col,
                    win_lin: lins,
                    x: inner_x + k as f64 * cw,
                    y: inner_y,
                    w: cw,
                    h: inner_h,
                });
            }
        }
        layout.workspaces.push(WsLayout {
            o,
            wi: j,
            x: wx,
            y: wy,
            w: ww,
            h: ws_h,
            cols,
        });
    }
}

fn compute_layout(model: &Model, w: f64, h: f64, solo: Option<usize>) -> Layout {
    let mut layout = Layout {
        outputs: Vec::new(),
        workspaces: Vec::new(),
    };
    let outputs = &model.outputs;
    if outputs.is_empty() {
        return layout;
    }

    let content_x = PAD;
    let content_y = PAD;
    let content_w = w - 2.0 * PAD;
    let content_h = (h - PAD - FOOTER_H) - content_y;

    // Solo mode: one output takes the whole content width.
    if let Some(o) = solo.filter(|o| *o < outputs.len()) {
        let rect = (content_x, content_y, content_w, content_h);
        layout.outputs.push(OutLayout {
            o,
            x: rect.0,
            y: rect.1,
            w: rect.2,
            h: rect.3,
        });
        layout_output(model, o, rect, &mut layout);
        return layout;
    }

    let min_x = outputs.iter().map(|o| o.x).fold(f64::INFINITY, f64::min);
    let max_x = outputs
        .iter()
        .map(|o| o.x + o.w)
        .fold(f64::NEG_INFINITY, f64::max);
    let span_w = (max_x - min_x).max(1.0);
    let scale_x = content_w / span_w;

    for (i, output) in outputs.iter().enumerate() {
        let rect = (
            content_x + (output.x - min_x) * scale_x,
            content_y,
            output.w * scale_x,
            content_h,
        );
        layout.outputs.push(OutLayout {
            o: i,
            x: rect.0,
            y: rect.1,
            w: rect.2,
            h: rect.3,
        });
        layout_output(model, i, rect, &mut layout);
    }
    layout
}

const DRAG_THRESHOLD: f64 = 6.0;

/// Output index whose column the cursor x is in (nearest if between/outside).
fn output_under_x(layout: &Layout, x: f64) -> Option<usize> {
    if let Some(ol) = layout.outputs.iter().find(|o| x >= o.x && x <= o.x + o.w) {
        return Some(ol.o);
    }
    layout
        .outputs
        .iter()
        .min_by(|a, b| {
            let da = (x - (a.x + a.w / 2.0)).abs();
            let db = (x - (b.x + b.w / 2.0)).abs();
            da.total_cmp(&db)
        })
        .map(|o| o.o)
}

/// If (x,y) is on a workspace header, return the workspace drag and its rect.
fn hit_workspace_header(
    layout: &Layout,
    model: &Model,
    x: f64,
    y: f64,
) -> Option<(DragKind, (f64, f64, f64, f64))> {
    for wl in &layout.workspaces {
        if x >= wl.x && x <= wl.x + wl.w && y >= wl.y && y <= wl.y + WS_HEADER_H {
            let id = model.outputs[wl.o].workspaces[wl.wi].ws.id;
            return Some((DragKind::Workspace { id }, (wl.x, wl.y, wl.w, wl.h)));
        }
    }
    None
}

/// Workspace drop slot for the cursor: which output and insertion index among
/// that output's other workspaces.
fn workspace_drop_target(
    layout: &Layout,
    model: &Model,
    dragged: u64,
    cursor: (f64, f64),
) -> Option<DropTarget> {
    let o = output_under_x(layout, cursor.0)?;
    let others: Vec<&WsLayout> = layout
        .workspaces
        .iter()
        .filter(|wl| wl.o == o && model.outputs[wl.o].workspaces[wl.wi].ws.id != dragged)
        .collect();
    let mut idx = others.len();
    for (i, wl) in others.iter().enumerate() {
        if cursor.1 < wl.y + wl.h / 2.0 {
            idx = i;
            break;
        }
    }
    Some(DropTarget::Workspace { o, idx })
}

/// Target on-screen top-left for each non-dragged workspace, with a gap opened
/// at the drop slot — the basis for the slide animation.
fn workspace_reflow(layout: &Layout, model: &Model, drag: &Drag) -> HashMap<u64, (f64, f64)> {
    let mut targets = HashMap::new();
    let DragKind::Workspace { id: dragged } = drag.kind else {
        return targets;
    };
    let gap = match drag.target {
        Some(DropTarget::Workspace { o, idx }) => Some((o, idx)),
        _ => None,
    };
    let ws_id = |wl: &WsLayout| model.outputs[wl.o].workspaces[wl.wi].ws.id;

    for ol in &layout.outputs {
        let o = ol.o;
        let all: Vec<&WsLayout> = layout.workspaces.iter().filter(|wl| wl.o == o).collect();
        if all.is_empty() {
            continue;
        }
        let base = all.iter().map(|wl| wl.y).fold(f64::INFINITY, f64::min);
        let step = all[0].h + WS_GAP;
        let x = all[0].x;

        let mut items: Vec<&&WsLayout> = all.iter().filter(|wl| ws_id(wl) != dragged).collect();
        items.sort_by(|a, b| a.y.total_cmp(&b.y));
        let gap_idx = gap.filter(|(go, _)| *go == o).map(|(_, i)| i);

        let mut slot = 0usize;
        for (i, wl) in items.iter().enumerate() {
            if gap_idx == Some(i) {
                slot += 1;
            }
            targets.insert(ws_id(wl), (x, base + slot as f64 * step));
            slot += 1;
        }
    }
    targets
}

/// If (x,y) is on a column's body, return the column drag and its slot rect.
fn hit_column(
    layout: &Layout,
    model: &Model,
    x: f64,
    y: f64,
) -> Option<(DragKind, (f64, f64, f64, f64))> {
    for wl in &layout.workspaces {
        let wsv = &model.outputs[wl.o].workspaces[wl.wi];
        for col in &wl.cols {
            if x >= col.x && x <= col.x + col.w && y >= col.y && y <= col.y + col.h {
                let win_id = wsv.windows[col.win_lin[0]].id;
                return Some((
                    DragKind::Column {
                        ws_id: wsv.ws.id,
                        col: col.col,
                        win_id,
                    },
                    (col.x, col.y, col.w, col.h),
                ));
            }
        }
    }
    None
}

/// Column drop slot: which workspace card the cursor is over, and the insertion
/// index among that workspace's columns.
fn column_drop_target(
    layout: &Layout,
    model: &Model,
    drag_ws: u64,
    drag_col: i64,
    cursor: (f64, f64),
) -> Option<DropTarget> {
    let wl = layout.workspaces.iter().find(|wl| {
        cursor.0 >= wl.x && cursor.0 <= wl.x + wl.w && cursor.1 >= wl.y && cursor.1 <= wl.y + wl.h
    })?;
    let same_ws = model.outputs[wl.o].workspaces[wl.wi].ws.id == drag_ws;
    let cols: Vec<&ColLayout> = wl
        .cols
        .iter()
        .filter(|c| !(same_ws && c.col == drag_col))
        .collect();
    let mut idx = cols.len();
    for (i, c) in cols.iter().enumerate() {
        if cursor.0 < c.x + c.w / 2.0 {
            idx = i;
            break;
        }
    }
    Some(DropTarget::Column {
        o: wl.o,
        wi: wl.wi,
        idx,
    })
}

/// Target on-screen top-left per (workspace id, column) with a gap opened at the
/// drop slot — basis for the column slide animation.
fn column_reflow(layout: &Layout, model: &Model, drag: &Drag) -> HashMap<(u64, i64), (f64, f64)> {
    let mut targets = HashMap::new();
    let DragKind::Column {
        ws_id: src_ws,
        col: dragged_col,
        ..
    } = drag.kind
    else {
        return targets;
    };
    let gap = match drag.target {
        Some(DropTarget::Column { o, wi, idx }) => {
            Some((model.outputs[o].workspaces[wi].ws.id, idx))
        }
        _ => None,
    };

    for wl in &layout.workspaces {
        if wl.cols.is_empty() {
            continue;
        }
        let wid = model.outputs[wl.o].workspaces[wl.wi].ws.id;
        let is_src = wid == src_ws;
        let cols: Vec<&ColLayout> = wl
            .cols
            .iter()
            .filter(|c| !(is_src && c.col == dragged_col))
            .collect();
        let base_x = wl.x + 9.0;
        let stepw = wl.cols[0].w;
        let gap_idx = gap.filter(|(gw, _)| *gw == wid).map(|(_, i)| i);

        let mut slot = 0usize;
        for (i, c) in cols.iter().enumerate() {
            if gap_idx == Some(i) {
                slot += 1;
            }
            targets.insert((wid, c.col), (base_x + slot as f64 * stepw, c.y));
            slot += 1;
        }
    }
    targets
}

/// Recompute the drop target from the current cursor.
fn recompute_target(state: &Rc<RefCell<State>>, w: f64, h: f64) {
    let mut s = state.borrow_mut();
    let Some(cursor) = s.drag.as_ref().map(|d| d.cursor) else {
        return;
    };
    let kind = s.drag.as_ref().unwrap().kind.clone();
    let layout = compute_layout(&s.model, w, h, s.solo);
    let target = match &kind {
        DragKind::Workspace { id } => workspace_drop_target(&layout, &s.model, *id, cursor),
        DragKind::Column { ws_id, col, .. } => {
            column_drop_target(&layout, &s.model, *ws_id, *col, cursor)
        }
    };
    if let Some(d) = s.drag.as_mut() {
        d.target = target;
    }
}

/// Ease the per-item animated positions toward their reflow targets. Returns
/// false when there's no drag left (stops the tick).
fn animate_step(state: &Rc<RefCell<State>>, w: f64, h: f64) -> bool {
    enum Targets {
        Ws(HashMap<u64, (f64, f64)>),
        Col(HashMap<(u64, i64), (f64, f64)>),
    }
    let targets = {
        let s = state.borrow();
        let Some(drag) = s.drag.as_ref() else {
            return false;
        };
        if !drag.active {
            return true;
        }
        let layout = compute_layout(&s.model, w, h, s.solo);
        match &drag.kind {
            DragKind::Workspace { .. } => Targets::Ws(workspace_reflow(&layout, &s.model, drag)),
            DragKind::Column { .. } => Targets::Col(column_reflow(&layout, &s.model, drag)),
        }
    };
    let mut s = state.borrow_mut();
    match targets {
        Targets::Ws(m) => {
            for (id, tgt) in m {
                let cur = s.anim_ws.entry(id).or_insert(tgt);
                cur.0 += (tgt.0 - cur.0) * 0.35;
                cur.1 += (tgt.1 - cur.1) * 0.35;
            }
        }
        Targets::Col(m) => {
            for (k, tgt) in m {
                let cur = s.anim_col.entry(k).or_insert(tgt);
                cur.0 += (tgt.0 - cur.0) * 0.35;
                cur.1 += (tgt.1 - cur.1) * 0.35;
            }
        }
    }
    true
}

/// Apply a drop via niri actions, then clear the drag and refresh.
fn apply_drop(state: &Rc<RefCell<State>>, kind: &DragKind, target: DropTarget) {
    if let (DragKind::Workspace { id }, DropTarget::Workspace { o, idx }) = (kind, target) {
        let (out_name, niri_idx, same) = {
            let s = state.borrow();
            let m = &s.model;
            let out_name = m.outputs[o].name.clone();
            let src_o = m
                .outputs
                .iter()
                .position(|out| out.workspaces.iter().any(|wv| wv.ws.id == *id));
            // Map the visible insertion slot to a niri index using neighbours'
            // real (1-based) indices, so hidden trailing empties don't offset it.
            let vis: Vec<i64> = m.outputs[o]
                .workspaces
                .iter()
                .filter(|wv| wv.ws.id != *id)
                .map(|wv| wv.ws.idx)
                .collect();
            let niri_idx = if idx < vis.len() {
                vis[idx]
            } else {
                vis.last().map(|v| v + 1).unwrap_or(1)
            };
            (out_name, niri_idx, src_o == Some(o))
        };
        let focus = capture_focus();
        if niri::focus_workspace_by_id(*id).is_ok() {
            if !same {
                let _ = niri::move_workspace_to_monitor_named(&out_name);
            }
            let _ = niri::move_workspace_to_index(niri_idx);
        }
        restore_focus(focus);
    } else if let (DragKind::Column { ws_id, col, win_id }, DropTarget::Column { o, wi, idx }) =
        (kind, target)
    {
        let (same_ws, same_monitor, out_name, target_ws_idx, col_niri) = {
            let s = state.borrow();
            let m = &s.model;
            let same_ws = m.outputs[o].workspaces[wi].ws.id == *ws_id;
            let src_o = m
                .outputs
                .iter()
                .position(|out| out.workspaces.iter().any(|wv| wv.ws.id == *ws_id));
            let out_name = m.outputs[o].name.clone();
            let target_ws_idx = m.outputs[o].workspaces[wi].ws.idx;
            // distinct column indices of the target workspace (drop dragged if same ws)
            let mut tcols: Vec<i64> = m.outputs[o].workspaces[wi]
                .windows
                .iter()
                .map(|wv| wv.column())
                .collect();
            tcols.sort_unstable();
            tcols.dedup();
            if same_ws {
                tcols.retain(|c| c != col);
            }
            let col_niri = if idx < tcols.len() {
                tcols[idx]
            } else {
                tcols.last().map(|v| v + 1).unwrap_or(1)
            };
            (same_ws, src_o == Some(o), out_name, target_ws_idx, col_niri)
        };
        let focus = capture_focus();
        if niri::focus_window(*win_id).is_ok() {
            if same_ws {
                let _ = niri::move_column_to_index(col_niri);
            } else if same_monitor {
                let _ = niri::move_column_to_workspace(&target_ws_idx.to_string());
            } else {
                let _ = niri::move_column_to_monitor(&out_name);
                let _ = niri::move_column_to_workspace(&target_ws_idx.to_string());
            }
        }
        restore_focus(focus);
    }
    {
        let mut s = state.borrow_mut();
        s.drag = None;
        s.anim_ws.clear();
        s.anim_col.clear();
    }
    refresh(state);
}

fn draw(cr: &gtk::cairo::Context, w: f64, h: f64, state: &State) {
    let t = state.theme();

    // Opaque backdrop (a layer surface can't be forced opaque from niri config,
    // so the app draws it fully opaque for readability).
    set(cr, t.bg, 1.0);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );

    if let Some(err) = &state.error {
        set(cr, t.urgent, 1.0);
        cr.set_font_size(16.0);
        text_at(cr, PAD, PAD + 16.0, &format!("niri error: {err}"));
        return;
    }

    let outputs = &state.model.outputs;
    if outputs.is_empty() {
        set(cr, t.subtext, 1.0);
        cr.set_font_size(16.0);
        text_at(cr, PAD, PAD + 16.0, "no workspaces found");
        return;
    }

    // Only show the groom selection while the overlay is focused; when it's an
    // unfocused background map, drop it so only niri's own highlight shows.
    let sel = if state.active {
        state.model.nav.get(state.sel_nav).copied()
    } else {
        None
    };

    let layout = compute_layout(&state.model, w, h, state.solo);

    // Output panels + headers.
    for ol in &layout.outputs {
        set(cr, t.text, 0.03);
        rounded_rect(cr, ol.x, ol.y, ol.w, ol.h, 14.0);
        let _ = cr.fill();

        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Bold,
        );
        set(cr, t.text, 0.85);
        cr.set_font_size(17.0);
        text_at(
            cr,
            ol.x + 12.0,
            ol.y + 21.0,
            &fit_text(cr, &outputs[ol.o].name, ol.w - 24.0),
        );
    }

    // Active drag (if any), by kind.
    let active = state.drag.as_ref().filter(|d| d.active);
    let dragged_ws = active.and_then(|d| match &d.kind {
        DragKind::Workspace { id } => Some((d, *id)),
        _ => None,
    });
    let dragged_col = active.and_then(|d| match &d.kind {
        DragKind::Column { ws_id, col, .. } => Some((d, *ws_id, *col)),
        _ => None,
    });

    if let Some((d, dragged)) = dragged_ws {
        // Workspace drag: neighbours reflow to their animated slots; the grabbed
        // card floats under the cursor.
        for wl in &layout.workspaces {
            let wsv = &outputs[wl.o].workspaces[wl.wi];
            if wsv.ws.id == dragged {
                continue;
            }
            let (ax, ay) = state
                .anim_ws
                .get(&wsv.ws.id)
                .copied()
                .unwrap_or((wl.x, wl.y));
            draw_workspace(
                cr,
                wl,
                ax - wl.x,
                ay - wl.y,
                wsv,
                false,
                state.sel_win,
                badge_for(state, wsv),
                t,
            );
        }
        if let Some(wl) = layout
            .workspaces
            .iter()
            .find(|wl| outputs[wl.o].workspaces[wl.wi].ws.id == dragged)
        {
            let wsv = &outputs[wl.o].workspaces[wl.wi];
            let (fx, fy) = (d.cursor.0 - d.grab.0, d.cursor.1 - d.grab.1);
            set_rgba(cr, 0.0, 0.0, 0.0, 0.35);
            rounded_rect(cr, fx + 3.0, fy + 6.0, wl.w, wl.h, 10.0);
            let _ = cr.fill();
            draw_workspace(
                cr,
                wl,
                fx - wl.x,
                fy - wl.y,
                wsv,
                true,
                state.sel_win,
                badge_for(state, wsv),
                t,
            );
        }
    } else if let Some((d, src_ws, dcol)) = dragged_col {
        // Column drag: cards stay put; columns reflow; the grabbed column floats.
        for wl in &layout.workspaces {
            let wsv = &outputs[wl.o].workspaces[wl.wi];
            draw_workspace_chrome(cr, wl, 0.0, 0.0, wsv, false, badge_for(state, wsv), t);
            for col in &wl.cols {
                if wsv.ws.id == src_ws && col.col == dcol {
                    continue;
                }
                let (ax, ay) = state
                    .anim_col
                    .get(&(wsv.ws.id, col.col))
                    .copied()
                    .unwrap_or((col.x, col.y));
                draw_column(
                    cr,
                    col,
                    ax - col.x,
                    ay - col.y,
                    wsv,
                    state.sel_win,
                    false,
                    t,
                );
            }
        }
        if let Some((wl, col)) = layout.workspaces.iter().find_map(|wl| {
            let wsv = &outputs[wl.o].workspaces[wl.wi];
            (wsv.ws.id == src_ws)
                .then(|| wl.cols.iter().find(|c| c.col == dcol).map(|c| (wl, c)))
                .flatten()
        }) {
            let wsv = &outputs[wl.o].workspaces[wl.wi];
            let (fx, fy) = (d.cursor.0 - d.grab.0, d.cursor.1 - d.grab.1);
            // Lift the floating column onto its own card.
            set_rgba(cr, 0.0, 0.0, 0.0, 0.35);
            rounded_rect(cr, fx + 3.0, fy + 6.0, col.w, col.h, 8.0);
            let _ = cr.fill();
            set(cr, t.surface, 1.0);
            rounded_rect(cr, fx, fy, col.w, col.h, 8.0);
            let _ = cr.fill();
            draw_column(
                cr,
                col,
                fx - col.x,
                fy - col.y,
                wsv,
                state.sel_win,
                false,
                t,
            );
        }
    } else {
        for wl in &layout.workspaces {
            let wsv = &outputs[wl.o].workspaces[wl.wi];
            let ws_selected = sel == Some((wl.o, wl.wi));
            draw_workspace(
                cr,
                wl,
                0.0,
                0.0,
                wsv,
                ws_selected,
                state.sel_win,
                badge_for(state, wsv),
                t,
            );
        }
    }

    if state.picker.is_some() {
        draw_picker(cr, w, h, state);
    } else if let Some(edit) = &state.editing {
        draw_rename(cr, w, h, edit, t);
    } else if state.show_help {
        draw_help(cr, w, h, t);
    } else if state.active {
        // The hint is interaction guidance; hide it on the passive map too.
        draw_hint(cr, w, h, t);
    }
}

/// A centered modal text field for renaming the selected workspace, with a
/// caret drawn at the edit cursor.
fn draw_rename(cr: &gtk::cairo::Context, w: f64, h: f64, edit: &Edit, t: &Theme) {
    // Dim everything behind the field.
    set_rgba(cr, 0.0, 0.0, 0.0, 0.45);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    let bw = (w * 0.5).clamp(320.0, 560.0);
    let bh = 92.0;
    let bx = (w - bw) / 2.0;
    let by = (h - bh) / 2.0;

    set(cr, t.surface, 1.0);
    rounded_rect(cr, bx, by, bw, bh, 12.0);
    let _ = cr.fill();
    set(cr, t.accent, 1.0);
    cr.set_line_width(2.0);
    rounded_rect(cr, bx, by, bw, bh, 12.0);
    let _ = cr.stroke();

    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    set(cr, t.subtext, 1.0);
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

    set(cr, t.text, 1.0);
    text_at(cr, text_x, text_y, &edit.text());

    // Caret: a thin vertical bar at the cursor's x-advance.
    let prefix: String = edit.buf[..edit.cursor].iter().collect();
    let caret_x = text_x
        + cr.text_extents(&prefix)
            .map(|e| e.x_advance())
            .unwrap_or(0.0);
    set(cr, t.accent, 1.0);
    cr.set_line_width(1.5);
    cr.move_to(caret_x, text_y - 17.0);
    cr.line_to(caret_x, text_y + 4.0);
    let _ = cr.stroke();

    let _ = cr.restore();
}

/// The theme picker modal: a list of themes with color swatches; the highlight
/// is applied live to the whole overlay, so the panel itself recolors too.
fn draw_picker(cr: &gtk::cairo::Context, w: f64, h: f64, state: &State) {
    let t = state.theme();
    let hi = state.picker.unwrap_or(0);
    let n = state.themes.len();

    // Dim everything behind the panel.
    set_rgba(cr, 0.0, 0.0, 0.0, 0.5);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    let row_h = 36.0;
    let head_h = 50.0;
    let bw = 400.0;
    let bh = head_h + n as f64 * row_h + 12.0;
    let bx = (w - bw) / 2.0;
    let by = (h - bh) / 2.0;

    set(cr, t.surface, 1.0);
    rounded_rect(cr, bx, by, bw, bh, 12.0);
    let _ = cr.fill();
    set(cr, t.accent, 1.0);
    cr.set_line_width(2.0);
    rounded_rect(cr, bx, by, bw, bh, 12.0);
    let _ = cr.stroke();

    // Header.
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    set(cr, t.text, 1.0);
    cr.set_font_size(16.0);
    text_at(cr, bx + 16.0, by + 26.0, "Theme");
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    set(cr, t.subtext, 1.0);
    cr.set_font_size(11.0);
    text_at(
        cr,
        bx + 16.0,
        by + 42.0,
        "j/k select · Enter save · Esc cancel",
    );

    // Rows.
    for (i, th) in state.themes.iter().enumerate() {
        let ry = by + head_h + i as f64 * row_h;

        if i == hi {
            set(cr, t.accent, 0.20);
            rounded_rect(cr, bx + 6.0, ry, bw - 12.0, row_h - 2.0, 7.0);
            let _ = cr.fill();
        }

        // Swatches: backdrop, accent, text.
        let ss = 14.0;
        let sy = ry + (row_h - ss) / 2.0 - 1.0;
        for (k, c) in [th.bg, th.accent, th.text].into_iter().enumerate() {
            let sx = bx + 16.0 + k as f64 * (ss + 4.0);
            set(cr, c, 1.0);
            rounded_rect(cr, sx, sy, ss, ss, 3.0);
            let _ = cr.fill();
            set(cr, t.text, 0.18);
            cr.set_line_width(1.0);
            rounded_rect(cr, sx, sy, ss, ss, 3.0);
            let _ = cr.stroke();
        }

        // Name.
        if i == hi {
            cr.select_font_face(
                "sans-serif",
                gtk::cairo::FontSlant::Normal,
                gtk::cairo::FontWeight::Bold,
            );
            set(cr, t.accent, 1.0);
        } else {
            cr.select_font_face(
                "sans-serif",
                gtk::cairo::FontSlant::Normal,
                gtk::cairo::FontWeight::Normal,
            );
            set(cr, t.text, 0.95);
        }
        cr.set_font_size(14.0);
        text_at(
            cr,
            bx + 16.0 + 3.0 * (ss + 4.0) + 8.0,
            ry + row_h / 2.0 + 5.0,
            th.name,
        );
    }
}

/// Draw a workspace card's chrome (background, border, header, separator), but
/// not its columns. `(dx, dy)` shifts it from its home position.
/// The badge to draw on `wsv`, if its name is flagged by the badge command.
fn badge_for<'a>(state: &'a State, wsv: &WsView) -> Option<&'a badges::Badge> {
    let name = wsv.ws.name.as_deref()?;
    state.badges.get(&name.to_lowercase())
}

#[allow(clippy::too_many_arguments)]
fn draw_workspace_chrome(
    cr: &gtk::cairo::Context,
    wl: &WsLayout,
    dx: f64,
    dy: f64,
    wsv: &WsView,
    selected: bool,
    badge: Option<&badges::Badge>,
    t: &Theme,
) {
    const R: f64 = 10.0;
    let (x, y, w, h) = (wl.x + dx, wl.y + dy, wl.w, wl.h);
    let marker = badge.map(|b| b.color.unwrap_or(t.marker));

    if selected {
        set(cr, t.selected_card(), 1.0);
    } else {
        set(cr, t.surface, 1.0);
    }
    rounded_rect(cr, x, y, w, h, R);
    let _ = cr.fill();

    // Border cascade: the groom selection, then niri's focus, else the faint
    // default edge. Badges are shown by the pill alone — no colored border.
    if selected {
        set(cr, t.accent, 1.0);
        cr.set_line_width(2.0);
    } else if wsv.ws.is_focused {
        set(cr, t.accent, 0.45);
        cr.set_line_width(1.2);
    } else {
        set(cr, t.text, 0.07);
        cr.set_line_width(1.0);
    }
    rounded_rect(cr, x, y, w, h, R);
    let _ = cr.stroke();

    // Badge pill in the top-right, drawn before the header so the header text
    // can be truncated to leave room for it.
    let mut header_max = w - 22.0;
    if let (Some(b), Some(m)) = (badge, marker) {
        if !b.label.is_empty() {
            cr.select_font_face(
                "sans-serif",
                gtk::cairo::FontSlant::Normal,
                gtk::cairo::FontWeight::Bold,
            );
            cr.set_font_size(12.0);
            let tw = cr.text_extents(&b.label).map(|e| e.width()).unwrap_or(0.0);
            let pill_w = tw + 14.0;
            let pill_h = 18.0;
            let px = x + w - pill_w - 8.0;
            let py = y + 5.0;
            set(cr, m, 1.0);
            rounded_rect(cr, px, py, pill_w, pill_h, 6.0);
            let _ = cr.fill();
            // Readable label color for the pill, by the marker's luminance.
            let lum = 0.299 * m.0 + 0.587 * m.1 + 0.114 * m.2;
            let ink = if lum > 0.6 { (0.0, 0.0, 0.0) } else { (1.0, 1.0, 1.0) };
            set(cr, ink, 1.0);
            text_at(cr, px + 7.0, py + 13.0, &b.label);
            header_max -= pill_w + 6.0;
        }
    }

    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    cr.set_font_size(15.0);
    if wsv.ws.is_urgent {
        set(cr, t.urgent, 1.0);
    } else if selected {
        set(cr, t.accent, 1.0);
    } else {
        set(cr, t.text, 0.95);
    }
    let header = wsv.ws.label();
    text_at(cr, x + 11.0, y + 19.0, &fit_text(cr, &header, header_max));

    set(cr, t.text, 0.08);
    cr.set_line_width(1.0);
    cr.move_to(x + 10.0, y + WS_HEADER_H - 5.0);
    cr.line_to(x + w - 10.0, y + WS_HEADER_H - 5.0);
    let _ = cr.stroke();

    if wsv.windows.is_empty() {
        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
        set(cr, t.subtext, 0.75);
        cr.set_font_size(12.0);
        text_at(cr, x + 11.0, y + WS_HEADER_H + 18.0, "(empty)");
    }
}

/// Draw one column's windows, shifted by `(dx, dy)` from its home slot.
#[allow(clippy::too_many_arguments)]
fn draw_column(
    cr: &gtk::cairo::Context,
    col: &ColLayout,
    dx: f64,
    dy: f64,
    wsv: &WsView,
    sel_win: usize,
    ws_selected: bool,
    t: &Theme,
) {
    let rows = col.win_lin.len().max(1);
    let rh = col.h / rows as f64;
    for (r, &lin) in col.win_lin.iter().enumerate() {
        let cx = col.x + dx;
        let ry = col.y + dy + r as f64 * rh;
        let win = &wsv.windows[lin];
        let win_selected = ws_selected && lin == sel_win;
        draw_window(
            cr,
            cx + 3.0,
            ry + 3.0,
            col.w - 6.0,
            rh - 6.0,
            win,
            win_selected,
            t,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_workspace(
    cr: &gtk::cairo::Context,
    wl: &WsLayout,
    dx: f64,
    dy: f64,
    wsv: &WsView,
    selected: bool,
    sel_win: usize,
    badge: Option<&badges::Badge>,
    t: &Theme,
) {
    draw_workspace_chrome(cr, wl, dx, dy, wsv, selected, badge, t);
    for col in &wl.cols {
        draw_column(cr, col, dx, dy, wsv, sel_win, selected, t);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_window(
    cr: &gtk::cairo::Context,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    win: &niri::Window,
    selected: bool,
    t: &Theme,
) {
    if w <= 2.0 || h <= 2.0 {
        return;
    }

    const R: f64 = 7.0;

    // Card fill. Selected windows get an accent tint (not a solid block) so the
    // labels stay readable; the accent border carries the "selected" signal.
    if selected {
        set(cr, t.selected_window(), 1.0);
    } else if win.is_focused {
        set(cr, t.focused_window(), 1.0);
    } else {
        set(cr, t.window, 1.0);
    }
    rounded_rect(cr, x, y, w, h, R);
    let _ = cr.fill();

    // Border.
    if win.is_urgent {
        set(cr, t.urgent, 1.0);
        cr.set_line_width(1.8);
    } else if selected {
        set(cr, t.accent, 1.0);
        cr.set_line_width(2.0);
    } else {
        set(cr, t.text, 0.07);
        cr.set_line_width(1.0);
    }
    rounded_rect(cr, x, y, w, h, R);
    let _ = cr.stroke();

    // Labels: app id (secondary, above) then title (primary). The title is the
    // important one, so in a short box I drop the app id and show only the title.
    let text_w = w - 16.0;
    if text_w <= 0.0 {
        return;
    }
    let tx = x + 8.0;

    if h >= 46.0 {
        // app id — secondary context line (regular weight).
        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
        set(cr, t.subtext, 1.0);
        cr.set_font_size(13.0);
        if let Some(app_id) = &win.app_id {
            text_at(cr, tx, y + 20.0, &fit_text(cr, app_id, text_w));
        }

        // title — primary, bold and larger.
        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Bold,
        );
        set(cr, t.text, 1.0);
        cr.set_font_size(15.0);
        text_at(cr, tx, y + 42.0, &fit_text(cr, &win.label(), text_w));
        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
    } else if h >= 20.0 {
        // Tight box: just the title, vertically centred.
        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Bold,
        );
        set(cr, t.text, 1.0);
        cr.set_font_size(14.0);
        text_at(
            cr,
            tx,
            y + h / 2.0 + 5.0,
            &fit_text(cr, &win.label(), text_w),
        );
        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
    }
}

/// The grouped key legend shown when `?` is pressed.
fn key_legend() -> [(&'static str, &'static str); 3] {
    [
        (
            "Workspace",
            "j/k prev/next · Shift+J/K reorder · Shift+H/L to screen · r rename · w kill",
        ),
        ("Window", "h/l prev/next · Ctrl+H/L move column · x kill"),
        (
            "General",
            "Tab switch screen · s solo screen · Enter focus · t theme · q quit",
        ),
    ]
}

/// A small unobtrusive hint in the bottom-right so the legend is discoverable.
fn draw_hint(cr: &gtk::cairo::Context, w: f64, h: f64, t: &Theme) {
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    set(cr, t.subtext, 0.5);
    cr.set_font_size(12.0);
    let s = "? keys";
    let tw = cr.text_extents(s).map(|e| e.x_advance()).unwrap_or(40.0);
    text_at(cr, w - PAD - tw, h - 7.0, s);
}

/// The key legend as a centered panel (toggled with `?`), grouped by target.
fn draw_help(cr: &gtk::cairo::Context, w: f64, h: f64, t: &Theme) {
    let rows = key_legend();

    set_rgba(cr, 0.0, 0.0, 0.0, 0.45);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    let line_h = 26.0;
    let bw = 720.0_f64.min(w - 2.0 * PAD);
    let bh = 56.0 + rows.len() as f64 * line_h + 6.0;
    let bx = (w - bw) / 2.0;
    let by = (h - bh) / 2.0;

    set(cr, t.surface, 1.0);
    rounded_rect(cr, bx, by, bw, bh, 12.0);
    let _ = cr.fill();
    set(cr, t.accent, 1.0);
    cr.set_line_width(2.0);
    rounded_rect(cr, bx, by, bw, bh, 12.0);
    let _ = cr.stroke();

    // Title + close hint.
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    set(cr, t.text, 1.0);
    cr.set_font_size(16.0);
    text_at(cr, bx + 18.0, by + 30.0, "Keys");
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    set(cr, t.subtext, 0.9);
    cr.set_font_size(12.0);
    let close = "? or Esc to close";
    let cw = cr.text_extents(close).map(|e| e.x_advance()).unwrap_or(0.0);
    text_at(cr, bx + bw - 18.0 - cw, by + 30.0, close);

    // Align the action columns to the widest label.
    cr.select_font_face(
        "sans-serif",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    cr.set_font_size(14.0);
    let label_w = rows
        .iter()
        .map(|(label, _)| cr.text_extents(label).map(|e| e.x_advance()).unwrap_or(0.0))
        .fold(0.0_f64, f64::max);
    let actions_x = bx + 18.0 + label_w + 16.0;

    for (i, (label, actions)) in rows.iter().enumerate() {
        let y = by + 56.0 + 18.0 + i as f64 * line_h;

        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Bold,
        );
        set(cr, t.accent, 1.0);
        cr.set_font_size(14.0);
        text_at(cr, bx + 18.0, y, label);

        cr.select_font_face(
            "sans-serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
        set(cr, t.text, 0.92);
        text_at(
            cr,
            actions_x,
            y,
            &fit_text(cr, actions, bx + bw - 18.0 - actions_x),
        );
    }
}
