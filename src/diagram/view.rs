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
fn arrowhead(window: &mut Window, from: (f32, f32), tip: (f32, f32), size: f32, color: Rgba) {
    let dx = tip.0 - from.0;
    let dy = tip.1 - from.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    // Rotate the reversed direction by +/- ~28 degrees.
    let ang = 0.5_f32;
    let (sa, ca) = ang.sin_cos();
    let w = (size * 0.6).max(1.0);
    for s in [sa, -sa] {
        let rx = -ux * ca + uy * s;
        let ry = -ux * s - uy * ca;
        let barb = (tip.0 + rx * size, tip.1 + ry * size);
        stroke_segment(window, tip, barb, w, color);
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
                fill_polygon(window, &corners, c);
            }
            stroke_polyline(window, &corners, stroke_w, stroke, true);
        }
        "diamond" => {
            let pts = [
                (sx + w * 0.5, sy),
                (sx + w, sy + h * 0.5),
                (sx + w * 0.5, sy + h),
                (sx, sy + h * 0.5),
            ];
            if let Some(c) = fill {
                fill_polygon(window, &pts, c);
            }
            stroke_polyline(window, &pts, stroke_w, stroke, true);
        }
        "ellipse" => {
            let pts = ellipse_points(sx + w * 0.5, sy + h * 0.5, w * 0.5, h * 0.5);
            if let Some(c) = fill {
                fill_polygon(window, &pts, c);
            }
            stroke_polyline(window, &pts, stroke_w, stroke, true);
        }
        "line" | "arrow" => {
            let Some(points) = &el.points else {
                return;
            };
            let screen: Vec<(f32, f32)> = points
                .iter()
                .map(|p| to_screen(el.x + p[0], el.y + p[1]))
                .collect();
            stroke_polyline(window, &screen, stroke_w, stroke, false);
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
        let elements = ds.file.elements.clone();
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
                    let (tool, world, hit, handle_hit, pan_x, pan_y) = {
                        let ds = this.state.read(cx);
                        let (wx, wy) = ds.screen_to_world(mx, my);
                        let handle_hit = ds.selected_single_box().and_then(|i| {
                            let b = ds.element_bounds(i)?;
                            handle_at(ds, b, mx, my).map(|h| (i, h, b))
                        });
                        (
                            ds.tool,
                            (wx, wy),
                            ds.hit_test(wx, wy),
                            handle_hit,
                            ds.pan_x,
                            ds.pan_y,
                        )
                    };
                    match tool {
                        Tool::Select => {
                            if let Some((idx, handle, orig)) = handle_hit {
                                this.state.update(cx, |s, _| s.push_undo());
                                this.drag = Some(Drag::Resize {
                                    index: idx,
                                    handle,
                                    orig,
                                });
                            } else if let Some(idx) = hit {
                                this.state.update(cx, |s, _| {
                                    s.select_only(idx);
                                    s.push_undo();
                                });
                                let origins = this.state.read(cx).selected_origins();
                                this.drag = Some(Drag::Move {
                                    start: world,
                                    origins,
                                    moved: false,
                                });
                            } else {
                                this.state.update(cx, |s, _| s.clear_selection());
                                this.drag = Some(Drag::Pan(mx, my, pan_x, pan_y));
                            }
                            cx.notify();
                        }
                        Tool::Text => {
                            this.state.update(cx, |s, _| s.set_pending_text(world.0, world.1));
                            cx.emit(DiagramViewEvent::Command("diagram-text-input"));
                            cx.stop_propagation();
                        }
                        creation => {
                            let idx = this.state.update(cx, |s, _| {
                                s.push_undo();
                                s.create_element(creation, world.0, world.1)
                            });
                            if let Some(idx) = idx {
                                this.drag = Some(Drag::Create {
                                    index: idx,
                                    tool: creation,
                                    start: world,
                                });
                                cx.notify();
                            } else {
                                this.state.update(cx, |s, _| s.discard_last_undo());
                            }
                        }
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _window, cx| {
                if e.pressed_button != Some(MouseButton::Left) {
                    this.drag = None;
                    return;
                }
                let mx: f32 = e.position.x.into();
                let my: f32 = e.position.y.into();
                enum Act {
                    Pan(f32, f32, f32, f32),
                    Move((f64, f64), Vec<(usize, f64, f64)>),
                    Create(usize, Tool, (f64, f64)),
                    Resize(usize, Handle, (f64, f64, f64, f64)),
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
                        let (dx, dy) = (wx - start.0, wy - start.1);
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
                        let cur = this.state.read(cx).screen_to_world(mx, my);
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
                        let cur = this.state.read(cx).screen_to_world(mx, my);
                        let (x, y, w, h) = resize_box(handle, orig, cur);
                        this.state.update(cx, |s, _| s.set_element_box(index, x, y, w, h));
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
                        }
                    },
                )
                .size_full(),
            )
            .child(self.render_palette(tool, cx))
    }
}

impl DiagramView {
    /// Build the floating tool palette.
    fn render_palette(&self, active: Tool, cx: &mut Context<Self>) -> impl IntoElement {
        const TOOLS: [(&str, &str, Tool); 8] = [
            ("Sel", "diagram-tool-select", Tool::Select),
            ("Rect", "diagram-tool-rectangle", Tool::Rectangle),
            ("Ellip", "diagram-tool-ellipse", Tool::Ellipse),
            ("Diam", "diagram-tool-diamond", Tool::Diamond),
            ("Arrow", "diagram-tool-arrow", Tool::Arrow),
            ("Line", "diagram-tool-line", Tool::Line),
            ("Draw", "diagram-tool-draw", Tool::Draw),
            ("Text", "diagram-tool-text", Tool::Text),
        ];
        let theme = self.theme;
        let mut row = div()
            .absolute()
            .top_2()
            .left_2()
            .flex()
            .flex_row()
            .gap_1();
        for (label, cmd, tool) in TOOLS {
            let is_active = active == tool;
            row = row.child(
                div()
                    .id(cmd)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(theme.text_muted))
                    .bg(rgb(if is_active {
                        theme.accent
                    } else {
                        theme.background
                    }))
                    .text_color(rgb(if is_active {
                        theme.background
                    } else {
                        theme.text
                    }))
                    .child(SharedString::from(label))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_this, _e: &MouseDownEvent, _window, cx| {
                            cx.emit(DiagramViewEvent::Command(cmd));
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        row
    }
}
