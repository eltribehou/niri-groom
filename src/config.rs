//! The KDL config at `$XDG_CONFIG_HOME/niri-groom/niri-groom.kdl`. The app owns
//! this file: it creates a default on first run and rewrites it when the theme
//! is saved, preserving any comments / extra keys via the `kdl` crate.

use crate::badges;
use kdl::{KdlDocument, KdlNode};
use std::path::PathBuf;

const DEFAULT_CONFIG: &str = "\
// niri-groom configuration.
// Managed by the app (the theme picker writes here), but you can add comments.
theme \"catppuccin-mocha\"

// Optionally flag workspaces with a colored badge from an external command.
// The command prints one tab-separated line per workspace to mark:
//   <workspace-name>\\t<label>\\t[#rrggbb]
// The label (e.g. a bookmark key) shows in a pill; the color is optional and
// falls back to the theme's marker color. I use this for my niri bookmarks:
// workspace-badges command=\"~/.config/niri/scripts/niri-groom-badges.sh\"

// Optionally declare kinds of workspace mark, each on its own key. Pressing the
// key runs the command with the selected workspace name in
// $NIRI_GROOM_WORKSPACE and the kind name in $NIRI_GROOM_MARK_KIND; the command
// owns the store (a file, etc.). Have the badges command above read it back so
// a mark shows as a pill. Bind the same script to a niri key to toggle from
// outside. Several kinds may share one command.
// mark-kind \"work\"     key=\"m\" command=\"~/.config/niri/scripts/niri-groom-mark-toggle.sh\"
// mark-kind \"personal\" key=\"p\" command=\"~/.config/niri/scripts/niri-groom-mark-toggle.sh\"
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

/// Read the configured `workspace-badges command="..."`, if present. This is
/// the generic hook for marking workspaces (my niri bookmarks are one use): the
/// app runs the command and badges the workspaces it names. Returns `None` when
/// unset, so the feature is simply off.
pub fn load_badge_command() -> Option<String> {
    let path = config_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let doc: KdlDocument = text.parse().ok()?;
    let node = doc.get("workspace-badges")?;
    // Prefer the `command=` property; accept a bare positional argument too.
    node.get("command")
        .or_else(|| node.get(0))
        .and_then(|v| v.as_string())
        .map(str::to_string)
}

/// Read the declared mark kinds, in config order.
///
/// Each `mark-kind "<name>" key="<c>" command="<cmd>"` node names one kind of
/// workspace mark: the key that toggles it and the command that owns its store.
/// `workspace-mark-toggle command="..."` is shorthand for a single kind named
/// `mark` on `m`, used only when no `mark-kind` node is present.
///
/// A node missing its key or command, or whose key is not a single character, is
/// dropped — a malformed line silently costs one kind rather than breaking the
/// rest of the config.
pub fn load_mark_kinds() -> Vec<badges::MarkKind> {
    let Some(path) = config_path() else {
        return Vec::new();
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| t.parse::<KdlDocument>().ok())
        .map(|doc| mark_kinds_from(&doc))
        .unwrap_or_default()
}

/// The mark kinds `doc` declares, in document order.
fn mark_kinds_from(doc: &KdlDocument) -> Vec<badges::MarkKind> {
    let kinds: Vec<badges::MarkKind> = doc
        .nodes()
        .iter()
        .filter(|n| n.name().value() == "mark-kind")
        .filter_map(|n| {
            let name = n.get(0).and_then(|v| v.as_string()).unwrap_or("mark");
            let mut key = n.get("key").and_then(|v| v.as_string())?.chars();
            let key = key.next().filter(|_| key.next().is_none())?;
            let command = n.get("command").and_then(|v| v.as_string())?;
            Some(badges::MarkKind {
                name: name.to_string(),
                key,
                command: command.to_string(),
            })
        })
        .collect();
    if !kinds.is_empty() {
        return kinds;
    }

    // Fall back to the single-kind shorthand.
    let Some(node) = doc.get("workspace-mark-toggle") else {
        return Vec::new();
    };
    // Prefer the `command=` property; accept a bare positional argument too.
    node.get("command")
        .or_else(|| node.get(0))
        .and_then(|v| v.as_string())
        .map(|command| {
            vec![badges::MarkKind {
                name: "mark".to_string(),
                key: 'm',
                command: command.to_string(),
            }]
        })
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(String, char, String)> {
        let doc: KdlDocument = src.parse().expect("valid kdl");
        mark_kinds_from(&doc)
            .into_iter()
            .map(|k| (k.name, k.key, k.command))
            .collect()
    }

    #[test]
    fn every_declared_kind_is_read_in_order() {
        assert_eq!(
            kinds(
                r#"
                mark-kind "work" key="m" command="work.sh"
                mark-kind "personal" key="p" command="personal.sh"
                "#
            ),
            vec![
                ("work".to_string(), 'm', "work.sh".to_string()),
                ("personal".to_string(), 'p', "personal.sh".to_string()),
            ]
        );
    }

    #[test]
    fn two_kinds_may_share_one_command() {
        let read = kinds(
            r#"
            mark-kind "work" key="m" command="marks.sh"
            mark-kind "personal" key="p" command="marks.sh"
            "#,
        );
        assert_eq!(read[0].2, read[1].2);
    }

    #[test]
    fn a_kind_without_a_key_or_a_command_is_dropped() {
        assert_eq!(
            kinds(
                r#"
                mark-kind "nokey" command="a.sh"
                mark-kind "nocmd" key="p"
                mark-kind "fine" key="p" command="a.sh"
                "#
            )
            .len(),
            1
        );
    }

    #[test]
    fn a_key_must_be_a_single_character() {
        assert!(kinds(r#"mark-kind "work" key="mm" command="a.sh""#).is_empty());
        assert!(kinds(r#"mark-kind "work" key="" command="a.sh""#).is_empty());
    }

    #[test]
    fn a_kind_left_unnamed_is_called_mark() {
        assert_eq!(kinds(r#"mark-kind key="p" command="a.sh""#)[0].0, "mark");
    }

    #[test]
    fn the_shorthand_declares_one_kind_on_m() {
        assert_eq!(
            kinds(r#"workspace-mark-toggle command="marks.sh""#),
            vec![("mark".to_string(), 'm', "marks.sh".to_string())]
        );
    }

    #[test]
    fn a_declared_kind_wins_over_the_shorthand() {
        assert_eq!(
            kinds(
                r#"
                workspace-mark-toggle command="old.sh"
                mark-kind "work" key="m" command="new.sh"
                "#
            ),
            vec![("work".to_string(), 'm', "new.sh".to_string())]
        );
    }

    #[test]
    fn a_config_declaring_no_marks_yields_none() {
        assert!(kinds(r#"theme "nord""#).is_empty());
    }
}
