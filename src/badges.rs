//! Generic per-workspace "badges" pulled from an external command. This keeps
//! niri-groom agnostic about *what* a badge means: the configured command
//! prints one `name<TAB>label[<TAB>#rrggbb]` line per workspace I want flagged,
//! and I render a small pill (and a colored outline) on the matching workspace.
//!
//! I use it to surface my niri workspace bookmarks (a thin script greps my
//! `bookmarks.kdl`), but the app itself knows nothing about bookmarks — anyone
//! can point the command at whatever notion of "marked workspace" they have.
//!
//! [`toggle`] is the write half of the same idea: a command flips a workspace's
//! marked state in its own store, which the read command above then reports back
//! as a pill. The app still owns no state — it only invokes the command on the
//! selected workspace.
//!
//! Marks come in [`MarkKind`]s, declared in the config. A kind is a name, the key
//! that toggles it and the command that owns its store, and that is all the app
//! knows about it: what a kind *means*, and whether two kinds exclude each other,
//! lives in the command.

use crate::theme::{self, Rgb};
use std::collections::HashMap;
use std::process::Command;

#[derive(Clone)]
pub struct Badge {
    /// The short text drawn in the pill (e.g. a bookmark key like `F11`). May
    /// be empty, in which case I only mark the workspace (colored outline) with
    /// no pill.
    pub label: String,
    /// Optional per-entry color (`#rrggbb`); when absent I use the theme's
    /// `marker` color so the mark stays coherent with the active palette.
    pub color: Option<Rgb>,
}

/// Run the badge command (through `sh -c`, so `~`, pipes and shell syntax work)
/// and parse its stdout into a map keyed by **lowercased** workspace name. niri
/// treats workspace names case-insensitively, so I match the same way. A
/// missing command, a failure, or unparseable output all yield an empty map —
/// badges are purely decorative, so I never surface an error for them.
pub fn load(command: &str) -> HashMap<String, Badge> {
    let mut map = HashMap::new();
    let output = match Command::new("sh").arg("-c").arg(command).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => return map,
    };
    for line in String::from_utf8_lossy(&output).lines() {
        let mut fields = line.split('\t');
        let name = fields.next().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let label = fields.next().unwrap_or("").trim().to_string();
        let color = fields.next().and_then(|c| theme::parse_hex(c.trim()));
        map.insert(name.to_lowercase(), Badge { label, color });
    }
    map
}

/// One declared kind of workspace mark.
#[derive(Clone)]
pub struct MarkKind {
    /// Label for the key legend, and what the command receives in
    /// `NIRI_GROOM_MARK_KIND`.
    pub name: String,
    /// The key that toggles this kind on the selected workspace.
    pub key: char,
    /// The command that owns this kind's store.
    pub command: String,
}

/// Run `kind`'s toggle command (through `sh -c`) to flip `workspace`'s mark of
/// that kind. The workspace name is passed in `NIRI_GROOM_WORKSPACE` and the
/// kind's name in `NIRI_GROOM_MARK_KIND`; the command decides what "marked"
/// means and where it's stored. The call blocks until the command exits so a
/// following refresh sees the new state. Any failure is ignored — marks are
/// decorative.
pub fn toggle(kind: &MarkKind, workspace: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(&kind.command)
        .env("NIRI_GROOM_WORKSPACE", workspace)
        .env("NIRI_GROOM_MARK_KIND", &kind.name)
        .status();
}
