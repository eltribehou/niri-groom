//! The picture the overlay draws: outputs, the workspaces on each, and the
//! windows in each workspace, built from a niri snapshot.

use crate::niri;
use std::collections::BTreeMap;

/// A workspace together with the windows it holds (sorted by column, then row).
pub struct WsView {
    pub ws: niri::Workspace,
    pub windows: Vec<niri::Window>,
}

/// One output (monitor) and its workspaces, sorted by index. `x`/`y` are niri's
/// logical position (used to order/place outputs) and `w` its logical width
/// (used for proportional horizontal sizing). Height isn't kept — outputs are
/// drawn full-height.
pub struct OutputView {
    pub name: String,
    pub workspaces: Vec<WsView>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
}

/// The full picture I draw, plus a flat navigation order over workspaces.
pub struct Model {
    pub outputs: Vec<OutputView>,
    /// `(output index, workspace index within output)` in display order.
    pub nav: Vec<(usize, usize)>,
}

/// Where `delta` steps of workspace navigation land, as an index into `nav`.
///
/// Stepping runs over the flat nav order, so it crosses from the last workspace
/// of one output to the first of the next. Solo mode confines it to the soloed
/// output. The ends clamp rather than wrap. `None` means there is nowhere to go.
pub fn step_nav(
    nav: &[(usize, usize)],
    sel: usize,
    delta: i32,
    solo: Option<usize>,
) -> Option<usize> {
    let candidates: Vec<usize> = (0..nav.len())
        .filter(|&i| solo.is_none_or(|o| nav[i].0 == o))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let pos = candidates.iter().position(|&i| i == sel).unwrap_or(0) as i32;
    let clamped = (pos + delta).clamp(0, candidates.len() as i32 - 1) as usize;
    Some(candidates[clamped])
}

/// Where `delta` steps of output navigation land, as `(nav index, output
/// index)` — the first workspace of the next output, wrapping around. `None`
/// when there are fewer than two outputs, or the target output holds no
/// workspaces.
pub fn step_output(
    nav: &[(usize, usize)],
    sel: usize,
    output_count: usize,
    delta: i32,
) -> Option<(usize, usize)> {
    if output_count < 2 {
        return None;
    }
    let current = nav.get(sel).map(|&(o, _)| o).unwrap_or(0);
    let next = ((current as i32 + delta).rem_euclid(output_count as i32)) as usize;
    let idx = nav.iter().position(|&(o, _)| o == next)?;
    Some((idx, next))
}

/// Where `delta` steps of window navigation land within a workspace holding
/// `count` windows. Both ends clamp, so it never leaves the workspace.
pub fn step_win(count: usize, sel: usize, delta: i32) -> usize {
    if count == 0 {
        return 0;
    }
    (sel as i32 + delta).clamp(0, count as i32 - 1) as usize
}

/// Build the model from a fresh niri snapshot.
pub fn build_model() -> Result<Model, String> {
    let workspaces = niri::fetch_workspaces()?;
    let windows = niri::fetch_windows()?;
    let outputs = niri::fetch_outputs().unwrap_or_default();
    Ok(model_from(workspaces, windows, outputs))
}

/// Arrange a niri snapshot into the model: windows onto their workspaces,
/// workspaces onto their outputs, outputs left to right.
fn model_from(
    workspaces: Vec<niri::Workspace>,
    windows: Vec<niri::Window>,
    outputs_geom: Vec<niri::Output>,
) -> Model {
    // Bucket windows by their workspace id.
    let mut by_ws: BTreeMap<u64, Vec<niri::Window>> = BTreeMap::new();
    for w in windows {
        if let Some(ws_id) = w.workspace_id {
            by_ws.entry(ws_id).or_default().push(w);
        }
    }

    // Logical placement per output, so I can draw screens where niri puts them.
    let geom: BTreeMap<String, (f64, f64, f64)> = outputs_geom
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

    Model { outputs, nav }
}

#[cfg(test)]
pub mod fixtures {
    use super::*;

    /// Builds a niri snapshot for tests: outputs left to right, workspaces on
    /// the output declared before them, windows in the workspace declared
    /// before them. Ids are given explicitly so a test can assert on them.
    #[derive(Default)]
    pub struct Snapshot {
        workspaces: Vec<niri::Workspace>,
        windows: Vec<niri::Window>,
        outputs: Vec<niri::Output>,
    }

    impl Snapshot {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn output(mut self, name: &str, x: f64, width: f64) -> Self {
            self.outputs.push(niri::Output {
                name: name.to_string(),
                logical: Some(niri::Logical {
                    x,
                    y: 0.0,
                    width,
                    height: 1080.0,
                }),
            });
            self
        }

        pub fn workspace(mut self, id: u64, idx: i64, name: Option<&str>) -> Self {
            let output = self.outputs.last().expect("declare an output first");
            self.workspaces.push(niri::Workspace {
                id,
                idx,
                name: name.map(str::to_string),
                output: Some(output.name.clone()),
                ..Default::default()
            });
            self
        }

        /// A window at `(column, row)` of the scrolling layout, in the
        /// workspace declared last.
        pub fn window(mut self, id: u64, column: i64, row: i64) -> Self {
            let ws = self.workspaces.last().expect("declare a workspace first");
            self.windows.push(niri::Window {
                id,
                title: Some(format!("window {id}")),
                workspace_id: Some(ws.id),
                layout: Some(niri::Layout {
                    pos_in_scrolling_layout: Some([column, row]),
                }),
                ..Default::default()
            });
            self
        }

        pub fn build(self) -> Model {
            model_from(self.workspaces, self.windows, self.outputs)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::Snapshot;
    use super::*;

    /// Two outputs: eDP-1 holds two workspaces, HDMI-A-1 holds one.
    fn two_outputs() -> Model {
        Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("code"))
            .window(10, 1, 1)
            .workspace(2, 2, Some("web"))
            .window(20, 1, 1)
            .output("HDMI-A-1", 1920.0, 2560.0)
            .workspace(3, 1, Some("chat"))
            .window(30, 1, 1)
            .build()
    }

    #[test]
    fn an_unnamed_empty_workspace_is_left_out_of_the_map() {
        let model = Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("code"))
            .window(10, 1, 1)
            .workspace(2, 2, None)
            .build();
        assert_eq!(model.outputs[0].workspaces.len(), 1);
        assert_eq!(model.outputs[0].workspaces[0].ws.id, 1);
    }

    #[test]
    fn a_named_empty_workspace_stays_on_the_map() {
        let model = Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("scratch"))
            .build();
        assert_eq!(model.outputs[0].workspaces.len(), 1);
    }

    #[test]
    fn outputs_are_ordered_by_their_logical_position() {
        let model = Snapshot::new()
            .output("HDMI-A-1", 1920.0, 2560.0)
            .workspace(1, 1, Some("right"))
            .output("eDP-1", 0.0, 1920.0)
            .workspace(2, 1, Some("left"))
            .build();
        let names: Vec<&str> = model.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["eDP-1", "HDMI-A-1"]);
    }

    #[test]
    fn windows_are_ordered_by_column_then_row() {
        let model = Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("code"))
            .window(30, 2, 1)
            .window(20, 1, 2)
            .window(10, 1, 1)
            .build();
        let ids: Vec<u64> = model.outputs[0].workspaces[0]
            .windows
            .iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(ids, [10, 20, 30]);
    }

    #[test]
    fn stepping_past_the_last_workspace_crosses_to_the_next_output() {
        let model = two_outputs();
        // nav index 1 is the last workspace of eDP-1; 2 is the first of HDMI-A-1.
        assert_eq!(step_nav(&model.nav, 1, 1, None), Some(2));
        assert_eq!(step_nav(&model.nav, 2, -1, None), Some(1));
    }

    #[test]
    fn stepping_stops_at_the_ends_rather_than_wrapping() {
        let model = two_outputs();
        assert_eq!(step_nav(&model.nav, 0, -1, None), Some(0));
        assert_eq!(step_nav(&model.nav, 2, 1, None), Some(2));
    }

    #[test]
    fn solo_mode_confines_stepping_to_the_soloed_output() {
        let model = two_outputs();
        // From the last workspace of eDP-1, forward would cross to HDMI-A-1 —
        // but eDP-1 is soloed, so the selection stays put.
        assert_eq!(step_nav(&model.nav, 1, 1, Some(0)), Some(1));
        // And from within HDMI-A-1, back stays inside HDMI-A-1.
        assert_eq!(step_nav(&model.nav, 2, -1, Some(1)), Some(2));
    }

    #[test]
    fn stepping_a_selection_outside_the_soloed_output_pulls_it_back_in() {
        let model = two_outputs();
        // Selection sits on eDP-1 while HDMI-A-1 is soloed: the step lands on
        // the soloed output rather than leaving the selection off-screen.
        assert_eq!(step_nav(&model.nav, 0, 1, Some(1)), Some(2));
    }

    #[test]
    fn there_is_nowhere_to_step_on_an_empty_map() {
        assert_eq!(step_nav(&[], 0, 1, None), None);
        assert_eq!(step_nav(&[(0, 0)], 0, 1, Some(9)), None);
    }

    #[test]
    fn output_steps_land_on_the_first_workspace_of_the_next_output() {
        let model = two_outputs();
        assert_eq!(step_output(&model.nav, 0, 2, 1), Some((2, 1)));
    }

    #[test]
    fn output_steps_wrap_around() {
        let model = two_outputs();
        // Forward from HDMI-A-1 (the last output) returns to eDP-1.
        assert_eq!(step_output(&model.nav, 2, 2, 1), Some((0, 0)));
        assert_eq!(step_output(&model.nav, 0, 2, -1), Some((2, 1)));
    }

    #[test]
    fn a_single_output_has_nowhere_to_step_to() {
        let model = two_outputs();
        assert_eq!(step_output(&model.nav, 0, 1, 1), None);
    }

    #[test]
    fn window_steps_clamp_inside_the_workspace() {
        assert_eq!(step_win(3, 0, 1), 1);
        assert_eq!(step_win(3, 2, 1), 2);
        assert_eq!(step_win(3, 0, -1), 0);
        assert_eq!(step_win(0, 0, 1), 0);
    }
}
