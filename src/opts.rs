//! The command-line options, parsed before the GTK application is built.
//!
//! `GApplication` would try to parse our flags itself, so the app is run with
//! `run_with_args` passing only argv[0], and everything here is read by hand.

use gtk4 as gtk;

const APP_ID: &str = "io.iwd.niri-groom";
/// Default layer-shell namespace (niri matches the surface by this); `--app-id`
/// overrides it.
const APP_NAMESPACE: &str = "niri-groom";

/// Parsed command-line options.
pub struct Opts {
    /// Start in solo mode on the output with this name (if it exists).
    pub solo_monitor: Option<String>,
    /// Place the overlay surface on this output (niri can't position a layer
    /// surface from config, so the client must request it).
    pub output: Option<String>,
    /// The layer-shell namespace (what niri matches the surface by). `--app-id`
    /// sets it, so niri config can target a given instance for its rules.
    pub namespace: String,
    /// `--toggle`: a second launch of the same instance closes it instead of
    /// re-presenting, so one keybind opens and closes the overlay.
    pub toggle: bool,
    /// `--focus`: move niri's focus onto the overlay's output at launch, so the
    /// exclusive-keyboard surface grabs the keyboard even when opened elsewhere.
    pub focus: bool,
}

pub fn parse_args() -> Opts {
    parse_argv(&std::env::args().collect::<Vec<String>>())
}

/// Read the options out of a full argv, whose first entry is the program name.
/// An unknown flag is skipped, and a flag missing its value leaves the default.
fn parse_argv(argv: &[String]) -> Opts {
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
pub fn derive_app_id(namespace: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Opts {
        let argv: Vec<String> = std::iter::once("niri-groom")
            .chain(args.iter().copied())
            .map(str::to_string)
            .collect();
        parse_argv(&argv)
    }

    #[test]
    fn a_bare_launch_takes_the_defaults() {
        let o = parse(&[]);
        assert_eq!(o.namespace, "niri-groom");
        assert_eq!(o.solo_monitor, None);
        assert_eq!(o.output, None);
        assert!(!o.toggle);
        assert!(!o.focus);
    }

    #[test]
    fn a_value_can_be_given_as_a_separate_argument_or_after_an_equals() {
        assert_eq!(
            parse(&["--solo", "HDMI-A-1"]).solo_monitor.as_deref(),
            Some("HDMI-A-1")
        );
        assert_eq!(
            parse(&["--solo=HDMI-A-1"]).solo_monitor.as_deref(),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn the_flags_of_a_typical_toggle_bind_are_all_read() {
        let o = parse(&[
            "--toggle",
            "--solo",
            "HDMI-A-1",
            "--open-on-monitor",
            "eDP-1",
            "--app-id",
            "niri-groom-map",
        ]);
        assert!(o.toggle);
        assert_eq!(o.solo_monitor.as_deref(), Some("HDMI-A-1"));
        assert_eq!(o.output.as_deref(), Some("eDP-1"));
        assert_eq!(o.namespace, "niri-groom-map");
    }

    #[test]
    fn a_value_is_not_mistaken_for_a_flag() {
        // "--focus" here is the value of --solo, however unlikely a screen name.
        let o = parse(&["--solo", "--focus"]);
        assert_eq!(o.solo_monitor.as_deref(), Some("--focus"));
        assert!(!o.focus);
    }

    #[test]
    fn an_unknown_flag_is_ignored() {
        let o = parse(&["--nonsense", "--focus"]);
        assert!(o.focus);
    }

    #[test]
    fn a_flag_left_without_its_value_keeps_the_default() {
        assert_eq!(parse(&["--app-id"]).namespace, "niri-groom");
    }

    #[test]
    fn a_namespace_that_is_already_a_valid_id_is_used_as_is() {
        assert_eq!(derive_app_id("io.iwd.niri-groom"), "io.iwd.niri-groom");
    }

    #[test]
    fn a_bare_namespace_is_qualified_into_a_valid_id() {
        assert_eq!(derive_app_id("niri-groom-map"), "io.iwd.niri-groom-map");
    }

    #[test]
    fn distinct_namespaces_derive_distinct_ids() {
        // Single-instance is keyed on the id, so a shared id would make two
        // differently-namespaced overlays the same instance.
        assert_ne!(derive_app_id("niri-groom-map"), derive_app_id("niri-groom"));
    }

    #[test]
    fn a_namespace_that_cannot_be_qualified_falls_back_to_the_default_id() {
        assert_eq!(derive_app_id("!!!"), APP_ID);
    }
}
