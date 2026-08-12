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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::Snapshot;

    const W: f64 = 1600.0;
    const H: f64 = 900.0;

    /// eDP-1 is 1920 wide at x=0; HDMI-A-1 is 2560 wide immediately to its right.
    fn two_outputs() -> Model {
        Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("code"))
            .window(10, 1, 1)
            .output("HDMI-A-1", 1920.0, 2560.0)
            .workspace(2, 1, Some("chat"))
            .window(20, 1, 1)
            .build()
    }

    #[test]
    fn outputs_keep_their_relative_widths() {
        let layout = compute_layout(&two_outputs(), W, H, None);
        let narrow = &layout.outputs[0];
        let wide = &layout.outputs[1];
        // 2560/1920 = 4/3, so the wider screen is drawn a third wider.
        assert!((wide.w / narrow.w - 4.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn outputs_are_placed_side_by_side_across_the_content_area() {
        let layout = compute_layout(&two_outputs(), W, H, None);
        assert_eq!(layout.outputs[0].x, PAD);
        // The right-hand screen starts where the left one ends.
        let left = &layout.outputs[0];
        assert!((layout.outputs[1].x - (left.x + left.w)).abs() < 1e-9);
    }

    #[test]
    fn a_soloed_output_takes_the_whole_width_on_its_own() {
        let layout = compute_layout(&two_outputs(), W, H, Some(1));
        assert_eq!(layout.outputs.len(), 1);
        assert_eq!(layout.outputs[0].o, 1);
        assert_eq!(layout.outputs[0].w, W - 2.0 * PAD);
    }

    #[test]
    fn soloing_an_output_that_is_not_there_shows_every_output() {
        let layout = compute_layout(&two_outputs(), W, H, Some(9));
        assert_eq!(layout.outputs.len(), 2);
    }

    #[test]
    fn windows_sharing_a_niri_column_share_one_slot() {
        let model = Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("code"))
            .window(10, 1, 1)
            .window(11, 1, 2)
            .window(12, 2, 1)
            .build();
        let layout = compute_layout(&model, W, H, None);
        let cols = &layout.workspaces[0].cols;
        assert_eq!(cols.len(), 2);
        assert_eq!(cols[0].win_lin, [0, 1]);
        assert_eq!(cols[1].win_lin, [2]);
    }

    #[test]
    fn columns_split_the_card_evenly() {
        let model = Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("code"))
            .window(10, 1, 1)
            .window(11, 2, 1)
            .build();
        let layout = compute_layout(&model, W, H, None);
        let cols = &layout.workspaces[0].cols;
        assert!((cols[0].w - cols[1].w).abs() < 1e-9);
        assert!((cols[1].x - (cols[0].x + cols[0].w)).abs() < 1e-9);
    }

    #[test]
    fn workspace_cards_stack_down_the_output() {
        let model = Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("code"))
            .window(10, 1, 1)
            .workspace(2, 2, Some("web"))
            .window(20, 1, 1)
            .build();
        let layout = compute_layout(&model, W, H, None);
        let (first, second) = (&layout.workspaces[0], &layout.workspaces[1]);
        assert_eq!(first.x, second.x);
        assert!((second.y - (first.y + first.h + WS_GAP)).abs() < 1e-9);
    }

    #[test]
    fn an_empty_workspace_gets_a_card_but_no_columns() {
        let model = Snapshot::new()
            .output("eDP-1", 0.0, 1920.0)
            .workspace(1, 1, Some("scratch"))
            .build();
        let layout = compute_layout(&model, W, H, None);
        assert_eq!(layout.workspaces.len(), 1);
        assert!(layout.workspaces[0].cols.is_empty());
    }

    #[test]
    fn a_map_with_no_outputs_lays_out_nothing() {
        let model = Snapshot::new().build();
        let layout = compute_layout(&model, W, H, None);
        assert!(layout.outputs.is_empty());
        assert!(layout.workspaces.is_empty());
    }
}
