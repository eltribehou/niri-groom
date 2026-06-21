//! The KDL config at `$XDG_CONFIG_HOME/niri-groom/niri-groom.kdl`. The app owns
//! this file: it creates a default on first run and rewrites it when the theme
//! is saved, preserving any comments / extra keys via the `kdl` crate.

use kdl::{KdlDocument, KdlNode};
use std::path::PathBuf;

const DEFAULT_CONFIG: &str = "\
// niri-groom configuration.
// Managed by the app (the theme picker writes here), but you can add comments.
theme \"catppuccin-mocha\"
";

fn config_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(dir.join("niri-groom").join("niri-groom.kdl"))
}

/// Read the configured theme name, creating the default config on first run.
pub fn load_theme() -> Option<String> {
    let path = config_path()?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, DEFAULT_CONFIG);
        return Some(crate::theme::DEFAULT.to_string());
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let doc: KdlDocument = text.parse().ok()?;
    doc.get("theme")
        .and_then(|n| n.entries().first())
        .and_then(|e| e.value().as_string())
        .map(str::to_string)
}

/// Persist the theme name, preserving the rest of the file.
pub fn save_theme(name: &str) {
    let Some(path) = config_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut doc: KdlDocument = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_default();

    if let Some(node) = doc.get_mut("theme") {
        node.entries_mut().clear();
        node.push(name);
    } else {
        let mut node = KdlNode::new("theme");
        node.push(name);
        doc.nodes_mut().push(node);
    }
    let _ = std::fs::write(&path, doc.to_string());
}
