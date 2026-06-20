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

/// Close a single window by id (no confirmation).
pub fn close_window(id: u64) -> Result<(), String> {
    let out = Command::new("niri")
        .args(["msg", "action", "close-window", "--id", &id.to_string()])
        .output()
        .map_err(|e| format!("failed to run close-window: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "close-window {id} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}
