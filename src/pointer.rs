//! Pointer drags: what is being dragged, what lies under the cursor, and where
//! a drop would land.
//!
//! Drop slots are expressed as indices among the *visible* neighbours; mapping
//! those back to niri's own workspace and column indices happens where the move
//! is applied, so hidden trailing empty workspaces can't offset a drop.

use crate::layout::{ColLayout, Layout, WsLayout, WS_GAP, WS_HEADER_H};
use crate::model::Model;
use std::collections::HashMap;

/// What the pointer is dragging.
#[derive(Clone)]
pub enum DragKind {
    /// A whole workspace (grabbed by its header), identified by id.
    Workspace { id: u64 },
    /// A column within a workspace: the source workspace + the niri column index,
    /// plus a representative window id to focus the column on drop.
    Column { ws_id: u64, col: i64, win_id: u64 },
}

/// Where a drag would land if dropped now.
#[derive(Clone, Copy, PartialEq)]
pub enum DropTarget {
    /// Insert the workspace into output `o` at slot `idx` (0-based among the
    /// other workspaces there).
    Workspace { o: usize, idx: usize },
    /// Insert the column into workspace (o, wi) at column slot `idx`.
    Column { o: usize, wi: usize, idx: usize },
}

pub struct Drag {
    pub kind: DragKind,
    /// Pointer offset within the grabbed item's rect, so it doesn't jump.
    pub grab: (f64, f64),
    /// Press point and current pointer, in widget coordinates.
    pub start: (f64, f64),
    pub cursor: (f64, f64),
    /// True once the pointer has moved past the start threshold.
    pub active: bool,
    pub target: Option<DropTarget>,
    /// True if the grabbed item was already the selection at press time. A
    /// release without a drag then focuses it (click the highlighted item again
    /// to activate it, like `Enter`).
    pub was_selected: bool,
}

pub const DRAG_THRESHOLD: f64 = 6.0;

/// Output index whose column the cursor x is in (nearest if between/outside).
pub fn output_under_x(layout: &Layout, x: f64) -> Option<usize> {
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

/// If (x,y) is on a workspace card, return the selection it implies as
/// `(nav index, window index)` — the same `(sel_nav, sel_win)` hjkl would set.
/// A click on a specific window selects that window; a click anywhere else on
/// the card selects the workspace (window index 0).
pub fn hit_select(layout: &Layout, model: &Model, x: f64, y: f64) -> Option<(usize, usize)> {
    for wl in &layout.workspaces {
        if x < wl.x || x > wl.x + wl.w || y < wl.y || y > wl.y + wl.h {
            continue;
        }
        let nav = model
            .nav
            .iter()
            .position(|&(o, w)| o == wl.o && w == wl.wi)?;
        for col in &wl.cols {
            if x >= col.x && x <= col.x + col.w && y >= col.y && y <= col.y + col.h {
                let rows = col.win_lin.len().max(1);
                let rh = col.h / rows as f64;
                let r = (((y - col.y) / rh) as usize).min(col.win_lin.len().saturating_sub(1));
                return Some((nav, col.win_lin[r]));
            }
        }
        return Some((nav, 0));
    }
    None
}

/// If (x,y) is on a workspace header, return the workspace drag and its rect.
pub fn hit_workspace_header(
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
pub fn workspace_drop_target(
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
pub fn workspace_reflow(layout: &Layout, model: &Model, drag: &Drag) -> HashMap<u64, (f64, f64)> {
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
pub fn hit_column(
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
pub fn column_drop_target(
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
pub fn column_reflow(
    layout: &Layout,
    model: &Model,
    drag: &Drag,
) -> HashMap<(u64, i64), (f64, f64)> {
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
