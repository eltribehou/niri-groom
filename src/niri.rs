//! Thin wrapper around the `niri msg` IPC: I read the current workspaces and
//! windows as JSON, and issue close actions.

use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub id: u64,
    pub idx: i64,
    pub name: Option<String>,
    pub output: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub is_active: bool,
    #[serde(default)]
    pub is_focused: bool,
    #[serde(default)]
    pub is_urgent: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Layout {
    /// 1-based `[column, row]` of the window inside the scrolling layout.
    /// Absent for floating windows.
    pub pos_in_scrolling_layout: Option<[i64; 2]>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Window {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub workspace_id: Option<u64>,
    #[serde(default)]
    pub is_focused: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub is_floating: bool,
    #[serde(default)]
    pub is_urgent: bool,
    #[serde(default)]
    pub layout: Option<Layout>,
}

impl Window {
    /// A human label: the title, falling back to the app id.
    pub fn label(&self) -> String {
        match self.title.as_deref() {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => self
                .app_id
                .clone()
                .unwrap_or_else(|| format!("window {}", self.id)),
        }
    }

    /// 1-based column in the scrolling layout (1 if unknown / floating).
    pub fn column(&self) -> i64 {
        self.layout
            .as_ref()
            .and_then(|l| l.pos_in_scrolling_layout)
            .map(|p| p[0])
            .unwrap_or(1)
    }

    /// 1-based row within the column (1 if unknown / floating).
    pub fn row(&self) -> i64 {
        self.layout
            .as_ref()
            .and_then(|l| l.pos_in_scrolling_layout)
            .map(|p| p[1])
            .unwrap_or(1)
    }
}

impl Workspace {
    /// A human label: the name, falling back to the index.
    pub fn label(&self) -> String {
        match self.name.as_deref() {
            Some(n) if !n.is_empty() => format!("{} {}", self.idx, n),
            _ => format!("workspace {}", self.idx),
        }
    }
}

/// An output's position and size in niri's logical coordinate space.
#[derive(Debug, Clone, Deserialize)]
pub struct Logical {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    /// Parsed for completeness; outputs are drawn full-height so it's unused.
    #[allow(dead_code)]
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Output {
    pub name: String,
    /// Absent when the output is disabled/off.
    pub logical: Option<Logical>,
}

fn niri_json<T: serde::de::DeserializeOwned>(subcommand: &str) -> Result<T, String> {
    let out = Command::new("niri")
        .args(["msg", "--json", subcommand])
        .output()
        .map_err(|e| format!("failed to run `niri msg`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`niri msg {subcommand}` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("parsing {subcommand}: {e}"))
}

pub fn fetch_workspaces() -> Result<Vec<Workspace>, String> {
    niri_json("workspaces")
}

pub fn fetch_windows() -> Result<Vec<Window>, String> {
    niri_json("windows")
}

/// Spawn `niri msg --json event-stream` with its stdout piped. niri emits one
/// JSON event per line whenever workspaces/windows/etc. change, so reading this
/// lets me refresh on change instead of polling.
pub fn spawn_event_stream() -> Option<std::process::Child> {
    let mut cmd = Command::new("niri");
    cmd.args(["msg", "--json", "event-stream"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    // Have the child die with us, so a `niri msg event-stream` can't linger as
    // an orphan if we're killed before it next writes (and hits SIGPIPE).
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong);
            Ok(())
        });
    }
    cmd.spawn().ok()
}

/// `niri msg --json outputs` is an object keyed by connector name; I just want
/// the values (each carries its own `name`).
pub fn fetch_outputs() -> Result<Vec<Output>, String> {
    let map: std::collections::BTreeMap<String, Output> = niri_json("outputs")?;
    Ok(map.into_values().collect())
}

/// Run `niri msg action <args...>`.
fn action(args: &[&str]) -> Result<(), String> {
    let out = Command::new("niri")
        .args(["msg", "action"])
        .args(args)
        .output()
        .map_err(|e| format!("failed to run niri action {args:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "niri action {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// Close a single window by id (no confirmation).
pub fn close_window(id: u64) -> Result<(), String> {
    action(&["close-window", "--id", &id.to_string()])
}

/// Drop a workspace's name (by index or name reference). niri keeps named
/// workspaces around forever, so once I've emptied a named workspace I unset
/// its name and niri reclaims the now-empty, unnamed workspace. Unnamed
/// workspaces are reclaimed automatically, so this is a no-op for them.
pub fn unset_workspace_name(reference: &str) -> Result<(), String> {
    action(&["unset-workspace-name", reference])
}

/// Focus a workspace by its stable id. `focus-workspace` only takes an index or
/// name, and indices can shift after a reorder, so I re-read the current state
/// to find the workspace's monitor and current index.
pub fn focus_workspace_by_id(id: u64) -> Result<(), String> {
    let wss = fetch_workspaces()?;
    let ws = wss
        .iter()
        .find(|w| w.id == id)
        .ok_or_else(|| format!("workspace {id} no longer exists"))?;
    if let Some(output) = &ws.output {
        action(&["focus-monitor", output])?;
    }
    action(&["focus-workspace", &ws.idx.to_string()])
}

/// Focus a window by its (stable) id.
pub fn focus_window(id: u64) -> Result<(), String> {
    action(&["focus-window", "--id", &id.to_string()])
}

/// Focus a monitor by output name. Used to move niri's focus onto the overlay's
/// output so its exclusive-keyboard layer surface can grab the keyboard.
pub fn focus_monitor(output: &str) -> Result<(), String> {
    action(&["focus-monitor", output])
}

/// Rename the workspace with stable `id` to `name` (an empty `name` unsets it),
/// **without moving focus**. I target it through `set-workspace-name`'s
/// `--workspace` reference instead of focusing it:
///   - a named workspace is referenced by its current name;
///   - an unnamed one is referenced by its index, which niri resolves on the
///     focused output — so I focus its monitor first (restoring the previous
///     one) only when it sits on a different output.
///
/// niri drops a `set-workspace-name` whose new name equals the current one
/// case-insensitively, so for a named workspace I force a case-only edit
/// (`Foo` -> `foo`) through a throwaway intermediate name (a zero-width-space
/// prefix), referencing the workspace by name at each step.
pub fn rename_workspace_by_id(id: u64, name: &str) -> Result<(), String> {
    let wss = fetch_workspaces()?;
    let ws = wss
        .iter()
        .find(|w| w.id == id)
        .ok_or_else(|| format!("workspace {id} no longer exists"))?;
    let current = ws.name.as_deref().filter(|n| !n.is_empty());

    if name.is_empty() {
        // Only a named workspace has a name to drop; an unnamed one already has none.
        return match current {
            Some(c) => action(&["unset-workspace-name", c]),
            None => Ok(()),
        };
    }

    if let Some(current) = current {
        let scratch = format!("\u{200b}{name}");
        let _ = action(&["set-workspace-name", "--workspace", current, &scratch]);
        return action(&["set-workspace-name", "--workspace", &scratch, name]);
    }

    // Unnamed: `--workspace <index>` resolves on the focused output, so focus
    // the target's monitor first when it differs, then put focus back.
    let idx = ws.idx.to_string();
    let focused_output = wss.iter().find(|w| w.is_focused).and_then(|w| w.output.clone());
    let cross = match (&ws.output, &focused_output) {
        (Some(t), Some(f)) => t != f,
        _ => false,
    };
    if cross {
        if let Some(t) = &ws.output {
            action(&["focus-monitor", t])?;
        }
    }
    let res = action(&["set-workspace-name", "--workspace", &idx, name]);
    if cross {
        if let Some(f) = &focused_output {
            let _ = action(&["focus-monitor", f]);
        }
    }
    res
}

/// Move the column containing `window_id` left or right within its workspace.
/// `move-column-left`/`-right` act on the focused column, so I focus a window
/// in the column first. niri clamps the move at the ends of the workspace.
pub fn move_column(window_id: u64, right: bool) -> Result<(), String> {
    focus_window(window_id)?;
    action(&[if right {
        "move-column-right"
    } else {
        "move-column-left"
    }])
}

/// Move the *focused* workspace to the monitor on the left/right. Caller focuses
/// the target workspace first. niri no-ops when there's no monitor that way.
pub fn move_workspace_to_monitor(left: bool) -> Result<(), String> {
    action(&[if left {
        "move-workspace-to-monitor-left"
    } else {
        "move-workspace-to-monitor-right"
    }])
}

// --- drag-and-drop drop primitives (all act on the focused workspace/column) ---

/// Move the focused workspace to a 1-based index on its monitor.
pub fn move_workspace_to_index(idx: i64) -> Result<(), String> {
    action(&["move-workspace-to-index", &idx.to_string()])
}

/// Move the focused workspace to a monitor by output name.
pub fn move_workspace_to_monitor_named(output: &str) -> Result<(), String> {
    action(&["move-workspace-to-monitor", output])
}

/// Move the focused column to a 1-based index on its workspace.
pub fn move_column_to_index(idx: i64) -> Result<(), String> {
    action(&["move-column-to-index", &idx.to_string()])
}

/// Move the focused column to a workspace by reference (index or name), without
/// moving keyboard focus along with it.
pub fn move_column_to_workspace(reference: &str) -> Result<(), String> {
    action(&["move-column-to-workspace", "--focus", "false", reference])
}

/// Move the focused column to a monitor by output name.
pub fn move_column_to_monitor(output: &str) -> Result<(), String> {
    action(&["move-column-to-monitor", output])
}

/// Move a workspace up or down within its monitor's stack.
///
/// `move-workspace-up`/`-down` only act on the *focused* workspace, and
/// `focus-workspace` only resolves within the focused output, so I first focus
/// the workspace's monitor, then the workspace by its (per-monitor) index, then
/// move it. niri clamps the move at the top/bottom of the stack.
pub fn move_workspace(output: &str, idx: i64, down: bool) -> Result<(), String> {
    action(&["focus-monitor", output])?;
    action(&["focus-workspace", &idx.to_string()])?;
    action(&[if down {
        "move-workspace-down"
    } else {
        "move-workspace-up"
    }])
}
