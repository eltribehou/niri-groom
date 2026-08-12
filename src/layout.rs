//! Where every box goes. Computing the positioned rectangles once, in one
//! place, is what lets rendering and pointer hit-testing agree about what is
//! where on screen.

use crate::model::Model;

pub const PAD: f64 = 20.0;
/// Reserved strip at the bottom for the small "? keys" hint.
const FOOTER_H: f64 = 22.0;
const OUTPUT_HEADER_H: f64 = 30.0;
pub const WS_GAP: f64 = 12.0;
pub const WS_HEADER_H: f64 = 28.0;

/// A column's on-screen slot within a workspace card.
pub struct ColLayout {
    /// niri column index (1-based) — used for `move-column-to-index`.
    #[allow(dead_code)]
    pub col: i64,
    /// Linear indices into the workspace's `windows`, top to bottom.
    pub win_lin: Vec<usize>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A workspace card's on-screen rect, with its columns.
pub struct WsLayout {
    pub o: usize,
    pub wi: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub cols: Vec<ColLayout>,
}

pub struct OutLayout {
    pub o: usize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The positioned boxes for the whole map. Computed once per frame and shared by
/// rendering and (later) pointer hit-testing, so geometry lives in one place.
pub struct Layout {
    pub outputs: Vec<OutLayout>,
    pub workspaces: Vec<WsLayout>,
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

pub fn compute_layout(model: &Model, w: f64, h: f64, solo: Option<usize>) -> Layout {
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
