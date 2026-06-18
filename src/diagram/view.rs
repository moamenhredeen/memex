use gpui::*;

use super::{DiagramState, Element, Tool};
use crate::keymap::{Action, KeyContext, KeymapSystem, ResolvedKey};
use crate::theme::Theme;

/// Emitted when a keybinding resolves to a command.
#[derive(Clone, Debug)]
pub enum DiagramViewEvent {
    Command(&'static str),
}

impl EventEmitter<DiagramViewEvent> for DiagramView {}

/// In-progress pointer interaction.
enum Drag {
    /// Panning the camera: (mouse_x, mouse_y, pan_x_start, pan_y_start).
    Pan(f32, f32, f32, f32),
    /// Moving the selection: mouse-down world point + each element's origin.
    Move {
        start: (f64, f64),
        origins: Vec<(usize, f64, f64)>,
        moved: bool,
    },
    /// Creating a new element by dragging.
    Create {
        index: usize,
        tool: Tool,
        start: (f64, f64),
    },
    /// Resizing a box element by dragging a handle.
    Resize {
        index: usize,
        handle: Handle,
        orig: (f64, f64, f64, f64),
    },
    /// Rubber-band selection: world start + current corner. `additive` unions
    /// onto `base` (the selection when the drag began).
    Marquee {
        start: (f64, f64),
        cur: (f64, f64),
        additive: bool,
        base: Vec<usize>,
    },
}

/// A resize handle on the selection bounding box.
#[derive(Clone, Copy)]
enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

/// World positions of the eight resize handles for a `(x, y, w, h)` box.
fn handle_world_positions(b: (f64, f64, f64, f64)) -> [(Handle, f64, f64); 8] {
    let (x, y, w, h) = b;
    [
        (Handle::NW, x, y),
        (Handle::N, x + w / 2.0, y),
        (Handle::NE, x + w, y),
        (Handle::E, x + w, y + h / 2.0),
        (Handle::SE, x + w, y + h),
        (Handle::S, x + w / 2.0, y + h),
        (Handle::SW, x, y + h),
        (Handle::W, x, y + h / 2.0),
    ]
}

/// Find a handle near the screen point `(mx, my)`, if any.
fn handle_at(ds: &DiagramState, b: (f64, f64, f64, f64), mx: f32, my: f32) -> Option<Handle> {
    const TOL: f32 = 7.0;
    for (handle, wx, wy) in handle_world_positions(b) {
        let (sx, sy) = ds.world_to_screen(wx, wy);
        if (sx - mx).abs() <= TOL && (sy - my).abs() <= TOL {
            return Some(handle);
        }
    }
    None
}

/// New `(x, y, w, h)` after dragging `handle` to world point `cur`.
fn resize_box(handle: Handle, orig: (f64, f64, f64, f64), cur: (f64, f64)) -> (f64, f64, f64, f64) {
    let (x, y, w, h) = orig;
    let (mut left, mut top, mut right, mut bottom) = (x, y, x + w, y + h);
    match handle {
        Handle::NW => {
            left = cur.0;
            top = cur.1;
        }
        Handle::N => top = cur.1,
        Handle::NE => {
            right = cur.0;
            top = cur.1;
        }
        Handle::E => right = cur.0,
        Handle::SE => {
            right = cur.0;
            bottom = cur.1;
        }
        Handle::S => bottom = cur.1,
        Handle::SW => {
            left = cur.0;
            bottom = cur.1;
        }
        Handle::W => left = cur.0,
    }
    let nw = (right - left).max(1.0);
    let nh = (bottom - top).max(1.0);
    (left.min(right), top.min(bottom), nw, nh)
}

pub struct DiagramView {
    state: Entity<DiagramState>,
    keymap: KeymapSystem,
    drag: Option<Drag>,
    theme: Theme,
}

impl DiagramView {
    pub fn new(state: Entity<DiagramState>, theme: Theme, _cx: &mut Context<Self>) -> Self {
        Self {
            state,
            keymap: KeymapSystem::new(false),
            drag: None,
            theme,
        }
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    /// Resolve a keystroke against the diagram key context.
    pub fn resolve_command(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> Option<&'static str> {
        let mut context = KeyContext::new();
        context.add("Diagram");
        match self.keymap.resolve_key(key, ctrl, shift, alt, &context) {
            ResolvedKey::Action(Action::Command(id), _) => Some(id),
            _ => None,
        }
    }
}

// ─── Color helpers ───────────────────────────────────────────────────────

/// Parse a `#rrggbb` (or `#rgb`, or `#rrggbbaa`) color into packed `0xRRGGBB`,
/// dropping any alpha (element opacity is applied separately).
fn parse_hex_rgb(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    match hex.len() {
        6 | 8 => u32::from_str_radix(&hex[..6], 16).ok(),
        3 => {
            let mut expanded = String::with_capacity(6);
            for c in hex.chars() {
                expanded.push(c);
                expanded.push(c);
            }
            u32::from_str_radix(&expanded, 16).ok()
        }
        _ => None,
    }
}

/// Resolve an excalidraw color string + element opacity (0..100) into an
/// `Rgba`. Returns `None` for `transparent` or unparseable colors.
fn elem_rgba(color: &str, opacity: f64) -> Option<Rgba> {
    if color.eq_ignore_ascii_case("transparent") {
        return None;
    }
    let rgb = parse_hex_rgb(color)?;
    let alpha = ((opacity / 100.0).clamp(0.0, 1.0) * 255.0) as u32;
    Some(rgba((rgb << 8) | alpha))
}

// ─── Paint primitives ──────────────────────────────────────────────────────

/// Fill a closed polygon.
fn fill_polygon(window: &mut Window, pts: &[(f32, f32)], color: Rgba) {
    if pts.len() < 3 {
        return;
    }
    let mut path = Path::new(point(px(pts[0].0), px(pts[0].1)));
    for q in &pts[1..] {
        path.line_to(point(px(q.0), px(q.1)));
    }
    path.line_to(point(px(pts[0].0), px(pts[0].1)));
    window.paint_path(path, color);
}

/// Stroke a single segment as a filled quad of the given width (so stroke
/// width scales with zoom, unlike a hairline path).
fn stroke_segment(window: &mut Window, a: (f32, f32), b: (f32, f32), width: f32, color: Rgba) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 {
        return;
    }
    let half = (width * 0.5).max(0.5);
    let nx = -dy / len * half;
    let ny = dx / len * half;
    let quad = [
        (a.0 + nx, a.1 + ny),
        (b.0 + nx, b.1 + ny),
        (b.0 - nx, b.1 - ny),
        (a.0 - nx, a.1 - ny),
    ];
    fill_polygon(window, &quad, color);
}

/// Stroke a polyline (or polygon when `closed`).
fn stroke_polyline(
    window: &mut Window,
    pts: &[(f32, f32)],
    width: f32,
    color: Rgba,
    closed: bool,
) {
    if pts.len() < 2 {
        return;
    }
    for i in 0..pts.len() - 1 {
        stroke_segment(window, pts[i], pts[i + 1], width, color);
    }
    if closed {
        stroke_segment(window, pts[pts.len() - 1], pts[0], width, color);
    }
}

/// Stroke a polyline with a dash pattern `(on, off)` in screen pixels. The
/// pattern carries continuously across vertices so corners stay even.
fn stroke_polyline_dashed(
    window: &mut Window,
    pts: &[(f32, f32)],
    width: f32,
    color: Rgba,
    closed: bool,
    on: f32,
    off: f32,
) {
    if pts.len() < 2 || on <= 0.0 || off <= 0.0 {
        return;
    }
    let mut seq = pts.to_vec();
    if closed {
        seq.push(pts[0]);
    }
    let mut cell_left = on; // remaining length in the current cell
    let mut drawing = true; // on-cell first
    for i in 0..seq.len() - 1 {
        let (mut ax, mut ay) = seq[i];
        let (bx, by) = seq[i + 1];
        let dx = bx - ax;
        let dy = by - ay;
        let seg = (dx * dx + dy * dy).sqrt();
        if seg < 1e-3 {
            continue;
        }
        let (ux, uy) = (dx / seg, dy / seg);
        let mut pos = 0.0;
        while pos < seg {
            let step = cell_left.min(seg - pos);
            let (nx, ny) = (ax + ux * step, ay + uy * step);
            if drawing {
                stroke_segment(window, (ax, ay), (nx, ny), width, color);
            }
            ax = nx;
            ay = ny;
            pos += step;
            cell_left -= step;
            if cell_left <= 1e-4 {
                drawing = !drawing;
                cell_left = if drawing { on } else { off };
            }
        }
    }
}

/// Dash pattern (screen px) for an excalidraw `strokeStyle`, or `None` for
/// solid. Scales with the stroke width so it reads at any zoom.
fn dash_pattern(style: &str, stroke_w: f32) -> Option<(f32, f32)> {
    match style {
        "dashed" => Some(((stroke_w * 4.0).max(6.0), (stroke_w * 3.0).max(5.0))),
        "dotted" => Some((stroke_w.max(1.5), (stroke_w * 2.0).max(3.0))),
        _ => None,
    }
}

/// Stroke a polyline either solid or dashed, per `dash`.
fn stroke_styled(
    window: &mut Window,
    pts: &[(f32, f32)],
    width: f32,
    color: Rgba,
    closed: bool,
    dash: Option<(f32, f32)>,
) {
    match dash {
        Some((on, off)) => stroke_polyline_dashed(window, pts, width, color, closed, on, off),
        None => stroke_polyline(window, pts, width, color, closed),
    }
}

/// Draw one family of parallel hatch lines clipped to a convex polygon `poly`
/// (screen coords). `slope_pos` picks the diagonal direction.
fn hatch_dir(
    window: &mut Window,
    poly: &[(f32, f32)],
    color: Rgba,
    line_w: f32,
    spacing: f32,
    slope_pos: bool,
) {
    if poly.len() < 3 {
        return;
    }
    // Line family: x + s*y = c. s=+1 gives slope -1, s=-1 gives slope +1.
    let s = if slope_pos { 1.0 } else { -1.0 };
    let mut cmin = f32::MAX;
    let mut cmax = f32::MIN;
    for &(x, y) in poly {
        let c = x + s * y;
        cmin = cmin.min(c);
        cmax = cmax.max(c);
    }
    let step = spacing * std::f32::consts::SQRT_2;
    if step < 1.0 {
        return;
    }
    let mut c = (cmin / step).ceil() * step;
    while c <= cmax {
        // Intersections of the line with each polygon edge.
        let mut hits: Vec<(f32, f32)> = Vec::new();
        for i in 0..poly.len() {
            let (x0, y0) = poly[i];
            let (x1, y1) = poly[(i + 1) % poly.len()];
            let f0 = x0 + s * y0 - c;
            let f1 = x1 + s * y1 - c;
            if (f0 <= 0.0 && f1 > 0.0) || (f0 > 0.0 && f1 <= 0.0) {
                let t = f0 / (f0 - f1);
                hits.push((x0 + t * (x1 - x0), y0 + t * (y1 - y0)));
            }
        }
        // Order along the line (key = projection onto its direction) and draw
        // between consecutive intersection pairs.
        hits.sort_by(|a, b| {
            let ka = s * a.0 - a.1;
            let kb = s * b.0 - b.1;
            ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut k = 0;
        while k + 1 < hits.len() {
            stroke_segment(window, hits[k], hits[k + 1], line_w, color);
            k += 2;
        }
        c += step;
    }
}

/// Fill a shape polygon: solid, hachure, or cross-hatch per `fill_style`.
fn fill_shape(window: &mut Window, poly: &[(f32, f32)], color: Rgba, fill_style: &str, line_w: f32) {
    let w = line_w.max(1.0);
    match fill_style {
        "hachure" => hatch_dir(window, poly, color, w, 8.0, true),
        "cross-hatch" => {
            hatch_dir(window, poly, color, w, 8.0, true);
            hatch_dir(window, poly, color, w, 8.0, false);
        }
        _ => fill_polygon(window, poly, color),
    }
}

/// Paint the background grid. `(bx, by)` is the canvas screen origin, `(w, h)`
/// its size; `(ox, oy)` is the world-to-screen offset (canvas origin + pan).
#[allow(clippy::too_many_arguments)]
fn paint_grid(
    window: &mut Window,
    bx: f32,
    by: f32,
    w: f32,
    h: f32,
    ox: f32,
    oy: f32,
    zoom: f32,
    grid: f32,
    color: Rgba,
) {
    let step = grid * zoom;
    if step < 4.0 {
        // Too dense to be useful; skip rather than flood the canvas.
        return;
    }
    // Visible world range (inverse of `sx = wx * zoom + ox`).
    let wl = (bx - ox) / zoom;
    let wr = (bx + w - ox) / zoom;
    let wt = (by - oy) / zoom;
    let wb = (by + h - oy) / zoom;
    // Vertical lines.
    let mut gx = (wl / grid).ceil() * grid;
    while gx <= wr {
        let sx = gx * zoom + ox;
        stroke_segment(window, (sx, by), (sx, by + h), 1.0, color);
        gx += grid;
    }
    // Horizontal lines.
    let mut gy = (wt / grid).ceil() * grid;
    while gy <= wb {
        let sy = gy * zoom + oy;
        stroke_segment(window, (bx, sy), (bx + w, sy), 1.0, color);
        gy += grid;
    }
}

/// Approximate an axis-aligned ellipse as a polygon.
fn ellipse_points(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<(f32, f32)> {
    const SEGMENTS: usize = 48;
    (0..SEGMENTS)
        .map(|i| {
            let t = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

/// Draw an arrowhead at `tip`, pointing along the direction `from -> tip`.
/// Painted as a filled triangle (not two strokes) so the tip is solid, like
/// drawio/excalidraw.
fn arrowhead(window: &mut Window, from: (f32, f32), tip: (f32, f32), size: f32, color: Rgba) {
    let dx = tip.0 - from.0;
    let dy = tip.1 - from.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    // Two barbs: rotate the reversed direction by +/- ~28 degrees.
    let ang = 0.5_f32;
    let (sa, ca) = ang.sin_cos();
    let mut barbs = [(0.0f32, 0.0f32); 2];
    for (k, s) in [sa, -sa].into_iter().enumerate() {
        let rx = -ux * ca + uy * s;
        let ry = -ux * s - uy * ca;
        barbs[k] = (tip.0 + rx * size, tip.1 + ry * size);
    }
    fill_polygon(window, &[tip, barbs[0], barbs[1]], color);
}

/// Paint a tool's glyph inside a button's bounds (used by the palette icons).
fn paint_tool_glyph(window: &mut Window, tool: Tool, bounds: Bounds<Pixels>, color: Rgba) {
    let ox: f32 = bounds.origin.x.into();
    let oy: f32 = bounds.origin.y.into();
    let s: f32 = bounds.size.width.into();
    let p = 3.0;
    let (l, t, r, b) = (ox + p, oy + p, ox + s - p, oy + s - p);
    let (w, h) = (r - l, b - t);
    let (cx, cy) = ((l + r) * 0.5, (t + b) * 0.5);
    let sw = 1.6;
    match tool {
        Tool::Select => {
            // Mouse-cursor silhouette.
            let pts = [
                (l, t),
                (l, b),
                (l + w * 0.30, b - h * 0.26),
                (l + w * 0.50, b),
                (l + w * 0.62, b - h * 0.05),
                (l + w * 0.42, b - h * 0.32),
                (l + w * 0.72, b - h * 0.32),
            ];
            fill_polygon(window, &pts, color);
        }
        Tool::Rectangle => {
            let c = [(l, t), (r, t), (r, b), (l, b)];
            stroke_polyline(window, &c, sw, color, true);
        }
        Tool::Ellipse => {
            let e = ellipse_points(cx, cy, w * 0.5, h * 0.5);
            stroke_polyline(window, &e, sw, color, true);
        }
        Tool::Diamond => {
            let d = [(cx, t), (r, cy), (cx, b), (l, cy)];
            stroke_polyline(window, &d, sw, color, true);
        }
        Tool::Arrow => {
            stroke_polyline(window, &[(l, b), (r, t)], sw, color, false);
            arrowhead(window, (l, b), (r, t), 6.0, color);
        }
        Tool::Line => stroke_polyline(window, &[(l, b), (r, t)], sw, color, false),
        Tool::Draw => {
            let z = [
                (l, b),
                (l + w * 0.3, t),
                (cx, b),
                (l + w * 0.7, t),
                (r, b),
            ];
            stroke_polyline(window, &z, sw, color, false);
        }
        Tool::Text => {
            stroke_polyline(window, &[(l, t), (r, t)], sw, color, false);
            stroke_polyline(window, &[(cx, t), (cx, b)], sw, color, false);
        }
    }
}

/// Paint one element with the given camera (`zoom`, screen origin `ox`/`oy`).
fn paint_element(
    window: &mut Window,
    cx: &mut App,
    el: &Element,
    zoom: f32,
    ox: f32,
    oy: f32,
    fallback_stroke: u32,
) {
    let to_screen = |wx: f64, wy: f64| {
        (
            wx as f32 * zoom + ox,
            wy as f32 * zoom + oy,
        )
    };
    let stroke = elem_rgba(&el.stroke_color, el.opacity)
        .unwrap_or_else(|| rgba((fallback_stroke << 8) | 0xFF));
    let fill = elem_rgba(&el.background_color, el.opacity);
    let stroke_w = (el.stroke_width as f32 * zoom).max(1.0);
    let dash = dash_pattern(el.stroke_style.as_str(), stroke_w);
    let fill_style = el.fill_style.as_str();
    let hatch_w = (stroke_w * 0.6).max(1.0);

    let (sx, sy) = to_screen(el.x, el.y);
    let w = el.width as f32 * zoom;
    let h = el.height as f32 * zoom;

    match el.element_type.as_str() {
        "rectangle" | "frame" => {
            let corners = [
                (sx, sy),
                (sx + w, sy),
                (sx + w, sy + h),
                (sx, sy + h),
            ];
            if let Some(c) = fill {
                fill_shape(window, &corners, c, fill_style, hatch_w);
            }
            stroke_styled(window, &corners, stroke_w, stroke, true, dash);
        }
        "diamond" => {
            let pts = [
                (sx + w * 0.5, sy),
                (sx + w, sy + h * 0.5),
                (sx + w * 0.5, sy + h),
                (sx, sy + h * 0.5),
            ];
            if let Some(c) = fill {
                fill_shape(window, &pts, c, fill_style, hatch_w);
            }
            stroke_styled(window, &pts, stroke_w, stroke, true, dash);
        }
        "ellipse" => {
            let pts = ellipse_points(sx + w * 0.5, sy + h * 0.5, w * 0.5, h * 0.5);
            if let Some(c) = fill {
                fill_shape(window, &pts, c, fill_style, hatch_w);
            }
            stroke_styled(window, &pts, stroke_w, stroke, true, dash);
        }
        "line" | "arrow" => {
            let Some(points) = &el.points else {
                return;
            };
            let screen: Vec<(f32, f32)> = points
                .iter()
                .map(|p| to_screen(el.x + p[0], el.y + p[1]))
                .collect();
            stroke_styled(window, &screen, stroke_w, stroke, false, dash);
            if el.element_type == "arrow" && screen.len() >= 2 {
                let n = screen.len();
                let head = (12.0 * zoom).max(8.0);
                arrowhead(window, screen[n - 2], screen[n - 1], head, stroke);
            }
        }
        "freedraw" => {
            if let Some(points) = &el.points {
                let screen: Vec<(f32, f32)> = points
                    .iter()
                    .map(|p| to_screen(el.x + p[0], el.y + p[1]))
                    .collect();
                stroke_polyline(window, &screen, stroke_w, stroke, false);
            }
        }
        "text" => {
            let Some(text) = &el.text else {
                return;
            };
            let font_size = (el.font_size.unwrap_or(20.0) as f32 * zoom).max(1.0);
            if font_size < 4.0 {
                return;
            }
            let line_h = font_size * 1.25;
            for (i, line) in text.split('\n').enumerate() {
                if line.is_empty() {
                    continue;
                }
                let run = TextRun {
                    len: line.len(),
                    font: Font {
                        family: "FiraCode Nerd Font".into(),
                        features: FontFeatures::default(),
                        fallbacks: None,
                        weight: FontWeight::NORMAL,
                        style: FontStyle::Normal,
                    },
                    color: stroke.into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window.text_system().shape_line(
                    line.to_string().into(),
                    px(font_size),
                    &[run],
                    None,
                );
                let _ = shaped.paint(
                    point(px(sx), px(sy + i as f32 * line_h)),
                    px(line_h),
                    window,
                    cx,
                );
            }
        }
        "image" => {
            // Phase 1: placeholder box; embedded image data lands later.
            let corners = [
                (sx, sy),
                (sx + w, sy),
                (sx + w, sy + h),
                (sx, sy + h),
            ];
            fill_polygon(window, &corners, rgba(0xCCCCCC40));
            stroke_polyline(window, &corners, stroke_w, stroke, true);
        }
        _ => {}
    }
}

impl Render for DiagramView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ds = self.state.read(cx);
        let focus = ds.focus_handle.clone();
        let theme = self.theme;
        let zoom = ds.zoom;
        let pan_x = ds.pan_x;
        let pan_y = ds.pan_y;
        let tool = ds.tool;
        let selected = ds.selected.clone();
        let handle_box = ds.selected_single_box().and_then(|i| ds.element_bounds(i));
        let editing = ds.editing_text();
        let show_grid = ds.show_grid;
        let grid = ds.grid_size() as f32;
        let grid_color = rgba((theme.border << 8) | 0x66);
        let elements = ds.file.elements.clone();
        let marquee = if let Some(Drag::Marquee { start, cur, .. }) = &self.drag {
            Some((*start, *cur))
        } else {
            None
        };
        let marquee_fill = rgba((theme.accent << 8) | 0x22);
        let bg = ds
            .file
            .background_color()
            .and_then(parse_hex_rgb)
            .unwrap_or(theme.background);
        let fallback_stroke = theme.text;

        div()
            .id("diagram-view")
            .size_full()
            .bg(rgb(bg))
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _window, cx| {
                let k = &e.keystroke;
                // While editing text, keys feed the text element, not commands.
                if this.state.read(cx).editing_text().is_some() {
                    let handled = match k.key.as_str() {
                        "escape" => {
                            this.state.update(cx, |s, _| s.finish_text_editing());
                            true
                        }
                        "backspace" => {
                            this.state.update(cx, |s, _| s.text_backspace());
                            true
                        }
                        "enter" => {
                            this.state.update(cx, |s, _| s.text_input("\n"));
                            true
                        }
                        _ if !k.modifiers.control && !k.modifiers.alt => {
                            if let Some(ch) = &k.key_char {
                                this.state.update(cx, |s, _| s.text_input(ch));
                                true
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    if handled {
                        cx.notify();
                        cx.stop_propagation();
                    }
                    return;
                }
                if let Some(cmd) = this.resolve_command(
                    k.key.as_str(),
                    k.modifiers.control,
                    k.modifiers.shift,
                    k.modifiers.alt,
                ) {
                    cx.emit(DiagramViewEvent::Command(cmd));
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _window, cx| {
                    let mx: f32 = e.position.x.into();
                    let my: f32 = e.position.y.into();
                    // Commit any in-progress text edit before handling the click.
                    if this.state.read(cx).editing_text().is_some() {
                        this.state.update(cx, |s, _| s.finish_text_editing());
                        cx.notify();
                    }
                    let (tool, world, hit, handle_hit) = {
                        let ds = this.state.read(cx);
                        let (wx, wy) = ds.screen_to_world(mx, my);
                        let handle_hit = ds.selected_single_box().and_then(|i| {
                            let b = ds.element_bounds(i)?;
                            handle_at(ds, b, mx, my).map(|h| (i, h, b))
                        });
                        (ds.tool, (wx, wy), ds.hit_test(wx, wy), handle_hit)
                    };
                    match tool {
                        Tool::Select => {
                            let ctrl = e.modifiers.control;
                            if let Some((idx, handle, orig)) = handle_hit {
                                this.state.update(cx, |s, _| s.push_undo());
                                this.drag = Some(Drag::Resize {
                                    index: idx,
                                    handle,
                                    orig,
                                });
                            } else if let Some(idx) = hit {
                                this.state.update(cx, |s, _| {
                                    if ctrl {
                                        s.toggle_select(idx);
                                    } else if !s.is_selected(idx) {
                                        // Click on an already-selected element of
                                        // a group keeps the group (moves all).
                                        s.select_only(idx);
                                    }
                                    s.push_undo();
                                });
                                let origins = this.state.read(cx).selected_origins();
                                this.drag = Some(Drag::Move {
                                    start: world,
                                    origins,
                                    moved: false,
                                });
                            } else {
                                // Empty canvas: rubber-band select. Ctrl adds to
                                // the existing selection.
                                let base = this.state.read(cx).selected.clone();
                                if !ctrl {
                                    this.state.update(cx, |s, _| s.clear_selection());
                                }
                                this.drag = Some(Drag::Marquee {
                                    start: world,
                                    cur: world,
                                    additive: ctrl,
                                    base,
                                });
                            }
                            cx.notify();
                        }
                        Tool::Text => {
                            let existing =
                                hit.filter(|&i| this.state.read(cx).is_text_element(i));
                            this.state.update(cx, |s, _| match existing {
                                Some(i) => s.edit_existing_text(i),
                                None => s.start_text(world.0, world.1),
                            });
                            cx.notify();
                            cx.stop_propagation();
                        }
                        creation => {
                            // Snap the origin corner so drag-created shapes sit
                            // on the grid. Alt inverts snap.
                            let snap = this.state.read(cx).snap_enabled ^ e.modifiers.alt;
                            let start = if snap {
                                let s = this.state.read(cx);
                                (s.snap_coord(world.0), s.snap_coord(world.1))
                            } else {
                                world
                            };
                            let idx = this.state.update(cx, |s, _| {
                                s.push_undo();
                                s.create_element(creation, start.0, start.1)
                            });
                            if let Some(idx) = idx {
                                this.drag = Some(Drag::Create {
                                    index: idx,
                                    tool: creation,
                                    start,
                                });
                                cx.notify();
                            } else {
                                this.state.update(cx, |s, _| s.discard_last_undo());
                            }
                        }
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, e: &MouseDownEvent, _window, cx| {
                    let mx: f32 = e.position.x.into();
                    let my: f32 = e.position.y.into();
                    let (px0, py0) = {
                        let ds = this.state.read(cx);
                        (ds.pan_x, ds.pan_y)
                    };
                    this.drag = Some(Drag::Pan(mx, my, px0, py0));
                    cx.stop_propagation();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _e: &MouseUpEvent, _window, _cx| {
                    this.drag = None;
                }),
            )
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _window, cx| {
                if e.pressed_button != Some(MouseButton::Left)
                    && e.pressed_button != Some(MouseButton::Middle)
                {
                    this.drag = None;
                    return;
                }
                let mx: f32 = e.position.x.into();
                let my: f32 = e.position.y.into();
                let alt = e.modifiers.alt;
                enum Act {
                    Pan(f32, f32, f32, f32),
                    Move((f64, f64), Vec<(usize, f64, f64)>),
                    Create(usize, Tool, (f64, f64)),
                    Resize(usize, Handle, (f64, f64, f64, f64)),
                    Marquee((f64, f64), bool, Vec<usize>),
                }
                let act = match &this.drag {
                    Some(Drag::Pan(a, b, c, d)) => Some(Act::Pan(*a, *b, *c, *d)),
                    Some(Drag::Move { start, origins, .. }) => {
                        Some(Act::Move(*start, origins.clone()))
                    }
                    Some(Drag::Create { index, tool, start }) => {
                        Some(Act::Create(*index, *tool, *start))
                    }
                    Some(Drag::Resize {
                        index,
                        handle,
                        orig,
                    }) => Some(Act::Resize(*index, *handle, *orig)),
                    Some(Drag::Marquee {
                        start,
                        additive,
                        base,
                        ..
                    }) => Some(Act::Marquee(*start, *additive, base.clone())),
                    None => None,
                };
                match act {
                    Some(Act::Pan(sx, sy, px0, py0)) => {
                        this.state.update(cx, |s, _| {
                            s.pan_x = px0 + (mx - sx);
                            s.pan_y = py0 + (my - sy);
                        });
                        cx.notify();
                    }
                    Some(Act::Move(start, origins)) => {
                        let (wx, wy) = this.state.read(cx).screen_to_world(mx, my);
                        let (mut dx, mut dy) = (wx - start.0, wy - start.1);
                        // Snap by aligning the anchor element's origin to the
                        // grid, then move the rest by the same delta (keeps the
                        // group's relative layout). Alt inverts snap.
                        let snap = this.state.read(cx).snap_enabled ^ alt;
                        if snap && let Some((_, ax, ay)) = origins.first() {
                            let s = this.state.read(cx);
                            dx = s.snap_coord(ax + dx) - ax;
                            dy = s.snap_coord(ay + dy) - ay;
                        }
                        this.state.update(cx, |s, _| {
                            for (i, ox, oy) in &origins {
                                s.set_element_position(*i, ox + dx, oy + dy);
                            }
                        });
                        if let Some(Drag::Move { moved, .. }) = &mut this.drag {
                            *moved = true;
                        }
                        cx.notify();
                    }
                    Some(Act::Create(index, tool, start)) => {
                        let mut cur = this.state.read(cx).screen_to_world(mx, my);
                        let snap = this.state.read(cx).snap_enabled ^ alt;
                        if snap {
                            let s = this.state.read(cx);
                            cur = (s.snap_coord(cur.0), s.snap_coord(cur.1));
                        }
                        this.state.update(cx, |s, _| {
                            if tool == Tool::Draw {
                                s.append_freedraw_point(index, start, cur);
                            } else {
                                s.update_creation(index, tool, start, cur);
                            }
                        });
                        cx.notify();
                    }
                    Some(Act::Resize(index, handle, orig)) => {
                        let mut cur = this.state.read(cx).screen_to_world(mx, my);
                        let snap = this.state.read(cx).snap_enabled ^ alt;
                        if snap {
                            let s = this.state.read(cx);
                            cur = (s.snap_coord(cur.0), s.snap_coord(cur.1));
                        }
                        let (x, y, w, h) = resize_box(handle, orig, cur);
                        this.state.update(cx, |s, _| s.set_element_box(index, x, y, w, h));
                        cx.notify();
                    }
                    Some(Act::Marquee(start, additive, base)) => {
                        let cur = this.state.read(cx).screen_to_world(mx, my);
                        if let Some(Drag::Marquee { cur: c, .. }) = &mut this.drag {
                            *c = cur;
                        }
                        let x0 = start.0.min(cur.0);
                        let y0 = start.1.min(cur.1);
                        let x1 = start.0.max(cur.0);
                        let y1 = start.1.max(cur.1);
                        this.state
                            .update(cx, |s, _| s.select_in_rect(x0, y0, x1, y1, additive, &base));
                        cx.notify();
                    }
                    None => {}
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, cx| {
                    match &this.drag {
                        Some(Drag::Move { moved: true, .. }) => {
                            this.state.update(cx, |s, _| s.dirty = true);
                            cx.notify();
                        }
                        // A click that selected but did not move: cancel the
                        // snapshot pushed on mouse-down.
                        Some(Drag::Move { moved: false, .. }) => {
                            this.state.update(cx, |s, _| s.discard_last_undo());
                        }
                        Some(Drag::Create { index, .. }) => {
                            let idx = *index;
                            this.state.update(cx, |s, _| s.finish_creation(idx));
                            cx.notify();
                        }
                        Some(Drag::Resize { .. }) => {
                            this.state.update(cx, |s, _| s.dirty = true);
                            cx.notify();
                        }
                        _ => {}
                    }
                    this.drag = None;
                }),
            )
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _window, cx| {
                let delta: f32 = match e.delta {
                    ScrollDelta::Lines(d) => d.y.into(),
                    ScrollDelta::Pixels(d) => {
                        let dy: f32 = d.y.into();
                        dy / 50.0
                    }
                };
                this.state.update(cx, |s, _| {
                    if delta > 0.0 {
                        s.zoom = (s.zoom * 1.1).min(5.0);
                    } else {
                        s.zoom = (s.zoom / 1.1).max(0.1);
                    }
                });
                cx.notify();
            }))
            .child(
                canvas(
                    {
                        let state = self.state.clone();
                        move |bounds, _window, cx| {
                            let ox: f32 = bounds.origin.x.into();
                            let oy: f32 = bounds.origin.y.into();
                            state.update(cx, |s, _| s.set_viewport_origin(ox, oy));
                        }
                    },
                    {
                        let accent = rgba((theme.accent << 8) | 0xFF);
                        move |bounds, _prepaint: (), window, cx| {
                            let ox: f32 = bounds.origin.x.into();
                            let oy: f32 = bounds.origin.y.into();
                            if show_grid {
                                let bw: f32 = bounds.size.width.into();
                                let bh: f32 = bounds.size.height.into();
                                paint_grid(
                                    window,
                                    ox,
                                    oy,
                                    bw,
                                    bh,
                                    ox + pan_x,
                                    oy + pan_y,
                                    zoom,
                                    grid,
                                    grid_color,
                                );
                            }
                            for el in &elements {
                                if el.is_deleted {
                                    continue;
                                }
                                paint_element(
                                    window,
                                    cx,
                                    el,
                                    zoom,
                                    ox + pan_x,
                                    oy + pan_y,
                                    fallback_stroke,
                                );
                            }
                            // Selection outlines.
                            for &idx in &selected {
                                let Some(el) = elements.get(idx) else {
                                    continue;
                                };
                                if el.is_deleted {
                                    continue;
                                }
                                let sx = el.x as f32 * zoom + ox + pan_x;
                                let sy = el.y as f32 * zoom + oy + pan_y;
                                let w = el.width as f32 * zoom;
                                let h = el.height as f32 * zoom;
                                let pad = 3.0;
                                let corners = [
                                    (sx - pad, sy - pad),
                                    (sx + w + pad, sy - pad),
                                    (sx + w + pad, sy + h + pad),
                                    (sx - pad, sy + h + pad),
                                ];
                                stroke_polyline(window, &corners, 1.5, accent, true);
                            }
                            // Resize handles for a single box selection.
                            if let Some(b) = handle_box {
                                let hs = 4.0; // half-size in screen px
                                for (_, wx, wy) in handle_world_positions(b) {
                                    let sx = wx as f32 * zoom + ox + pan_x;
                                    let sy = wy as f32 * zoom + oy + pan_y;
                                    let sq = [
                                        (sx - hs, sy - hs),
                                        (sx + hs, sy - hs),
                                        (sx + hs, sy + hs),
                                        (sx - hs, sy + hs),
                                    ];
                                    fill_polygon(window, &sq, accent);
                                }
                            }
                            // Rubber-band marquee rectangle.
                            if let Some((s, c)) = marquee {
                                let sx0 = s.0.min(c.0) as f32 * zoom + ox + pan_x;
                                let sy0 = s.1.min(c.1) as f32 * zoom + oy + pan_y;
                                let sx1 = s.0.max(c.0) as f32 * zoom + ox + pan_x;
                                let sy1 = s.1.max(c.1) as f32 * zoom + oy + pan_y;
                                let corners = [
                                    (sx0, sy0),
                                    (sx1, sy0),
                                    (sx1, sy1),
                                    (sx0, sy1),
                                ];
                                fill_polygon(window, &corners, marquee_fill);
                                stroke_polyline(window, &corners, 1.0, accent, true);
                            }
                            // Text caret while editing.
                            if let Some(eidx) = editing
                                && let Some(el) = elements.get(eidx)
                            {
                                let fs = el.font_size.unwrap_or(20.0) as f32 * zoom;
                                let txt = el.text.as_deref().unwrap_or("");
                                let last = txt.split('\n').next_back().unwrap_or("");
                                let lines = txt.split('\n').count().max(1);
                                let caret_x = el.x as f32 * zoom
                                    + ox
                                    + pan_x
                                    + last.chars().count() as f32 * fs * 0.6;
                                let caret_y =
                                    el.y as f32 * zoom + oy + pan_y + (lines - 1) as f32 * fs * 1.25;
                                stroke_segment(
                                    window,
                                    (caret_x, caret_y),
                                    (caret_x, caret_y + fs),
                                    1.5,
                                    accent,
                                );
                            }
                        }
                    },
                )
                .size_full(),
            )
            .child(self.render_palette(tool, cx))
            .children(self.render_panel(cx))
    }
}

impl DiagramView {
    /// Build the floating, centered tool palette with drawn glyph icons.
    fn render_palette(&self, active: Tool, cx: &mut Context<Self>) -> impl IntoElement {
        // Freehand (Tool::Draw) is intentionally absent: this is a diagramming
        // tool, not a sketch pad. The tool + model + import support remain.
        const TOOLS: [(&str, Tool); 7] = [
            ("diagram-tool-select", Tool::Select),
            ("diagram-tool-rectangle", Tool::Rectangle),
            ("diagram-tool-ellipse", Tool::Ellipse),
            ("diagram-tool-diamond", Tool::Diamond),
            ("diagram-tool-arrow", Tool::Arrow),
            ("diagram-tool-line", Tool::Line),
            ("diagram-tool-text", Tool::Text),
        ];
        let theme = self.theme;
        let mut bar = div()
            .flex()
            .flex_row()
            .gap_1()
            .p_1()
            .rounded_lg()
            .bg(rgb(theme.surface))
            .border_1()
            .border_color(rgb(theme.border))
            .shadow_md();
        for (cmd, tool) in TOOLS {
            let is_active = active == tool;
            let icon_color = rgba(
                ((if is_active { theme.background } else { theme.text }) << 8) | 0xFF,
            );
            bar = bar.child(
                div()
                    .id(cmd)
                    .size(px(30.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(rgb(if is_active { theme.accent } else { theme.surface }))
                    .hover(move |s| {
                        s.bg(rgb(if is_active {
                            theme.accent
                        } else {
                            theme.selection
                        }))
                    })
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _: (), window, _cx| {
                                paint_tool_glyph(window, tool, bounds, icon_color);
                            },
                        )
                        .size(px(20.0)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _e: &MouseDownEvent, _window, cx| {
                            cx.emit(DiagramViewEvent::Command(cmd));
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        div()
            .absolute()
            .top_3()
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(bar)
    }

    /// Properties panel for the current selection: stroke/background colors,
    /// width, stroke style, fill style. `None` when nothing is selected.
    fn render_panel(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let theme = self.theme;
        let (cur_stroke, cur_bg, cur_width, cur_sstyle, cur_fill) = {
            let s = self.state.read(cx);
            let el = s.primary_selected()?;
            (
                el.stroke_color.clone(),
                el.background_color.clone(),
                el.stroke_width,
                el.stroke_style.clone(),
                el.fill_style.clone(),
            )
        };

        const STROKES: [(&str, &str); 5] = [
            ("dstk-0", "#1e1e1e"),
            ("dstk-1", "#e03131"),
            ("dstk-2", "#2f9e44"),
            ("dstk-3", "#1971c2"),
            ("dstk-4", "#f08c00"),
        ];
        const BGS: [(&str, &str); 5] = [
            ("dbg-0", "transparent"),
            ("dbg-1", "#ffc9c9"),
            ("dbg-2", "#b2f2bb"),
            ("dbg-3", "#a5d8ff"),
            ("dbg-4", "#ffec99"),
        ];

        let label = |text: &'static str| div().text_color(rgb(theme.text)).child(text);

        // Stroke color swatches.
        let mut stroke_row = div().flex().flex_row().gap_1();
        for (id, hex) in STROKES {
            let active = cur_stroke.eq_ignore_ascii_case(hex);
            let col = parse_hex_rgb(hex).unwrap_or(0);
            stroke_row = stroke_row.child(
                div()
                    .id(id)
                    .size(px(20.0))
                    .rounded_md()
                    .bg(rgb(col))
                    .border_2()
                    .border_color(rgb(if active { theme.accent } else { theme.border }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                            this.state.update(cx, |s, _| s.set_selected_stroke_color(hex));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }

        // Background swatches.
        let mut bg_row = div().flex().flex_row().gap_1();
        for (id, hex) in BGS {
            let active = cur_bg.eq_ignore_ascii_case(hex);
            let transparent = hex == "transparent";
            let col = parse_hex_rgb(hex).unwrap_or(theme.background);
            bg_row = bg_row.child(
                div()
                    .id(id)
                    .size(px(20.0))
                    .rounded_md()
                    .bg(rgb(if transparent { theme.background } else { col }))
                    .border_2()
                    .border_color(rgb(if active { theme.accent } else { theme.border }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                            this.state.update(cx, |s, _| s.set_selected_background(hex));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }

        // Stroke width.
        let mut width_row = div().flex().flex_row().gap_1();
        for (id, wv, bar_h) in [
            ("dw-1", 1.0_f64, 2.0_f32),
            ("dw-2", 2.0, 4.0),
            ("dw-3", 4.0, 6.0),
        ] {
            let active = (cur_width - wv).abs() < 0.01;
            width_row = width_row.child(
                div()
                    .id(id)
                    .w(px(30.0))
                    .h(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(rgb(if active { theme.accent } else { theme.surface }))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .child(div().w(px(16.0)).h(px(bar_h)).rounded_sm().bg(rgb(theme.text)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                            this.state.update(cx, |s, _| s.set_selected_stroke_width(wv));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }

        // Stroke style + fill style (text buttons).
        let mut sstyle_row = div().flex().flex_row().gap_1();
        for (id, val, lbl) in [
            ("dss-solid", "solid", "solid"),
            ("dss-dashed", "dashed", "dash"),
            ("dss-dotted", "dotted", "dot"),
        ] {
            let active = cur_sstyle == val;
            sstyle_row = sstyle_row.child(
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_color(rgb(theme.text))
                    .bg(rgb(if active { theme.accent } else { theme.surface }))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .child(lbl)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                            this.state.update(cx, |s, _| s.set_selected_stroke_style(val));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }

        let mut fill_row = div().flex().flex_row().gap_1();
        for (id, val, lbl) in [
            ("dfs-solid", "solid", "solid"),
            ("dfs-hachure", "hachure", "hach"),
            ("dfs-cross", "cross-hatch", "cross"),
        ] {
            let active = cur_fill == val;
            fill_row = fill_row.child(
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_color(rgb(theme.text))
                    .bg(rgb(if active { theme.accent } else { theme.surface }))
                    .border_1()
                    .border_color(rgb(theme.border))
                    .child(lbl)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                            this.state.update(cx, |s, _| s.set_selected_fill_style(val));
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }

        Some(
            div()
                .id("diagram-panel")
                .absolute()
                .top_3()
                .right_3()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded_lg()
                .bg(rgb(theme.surface))
                .border_1()
                .border_color(rgb(theme.border))
                .shadow_md()
                .text_color(rgb(theme.text))
                // Swallow clicks on the panel so they don't reach the canvas.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_this, _e: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                    }),
                )
                .child(label("Stroke"))
                .child(stroke_row)
                .child(label("Background"))
                .child(bg_row)
                .child(label("Width"))
                .child(width_row)
                .child(label("Stroke style"))
                .child(sstyle_row)
                .child(label("Fill style"))
                .child(fill_row),
        )
    }
}
