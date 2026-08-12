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
