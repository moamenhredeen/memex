use gpui::*;

use super::{GraphEvent, GraphState};
use crate::keymap::{Action, KeyCombo, KeyTrie, Layer, build_graph_layer};

/// Emitted by [`GraphView`] when a keybinding resolves to a command.
#[derive(Clone, Debug)]
pub enum GraphViewEvent {
    Command(&'static str),
}

impl EventEmitter<GraphViewEvent> for GraphView {}

pub struct GraphView {
    state: Entity<GraphState>,
    /// Track drag state for panning.
    drag_start: Option<(f32, f32, f32, f32)>, // (mouse_x, mouse_y, pan_x, pan_y)
    /// Graph-local keymap layer. Only resolves when this view has focus.
    keymap: Layer,
}

impl GraphView {
    /// Resolve a keystroke against the graph layer. Pure function — useful for tests.
    pub fn resolve_command(&self, key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<&'static str> {
        let combo = KeyCombo::from_keystroke(key, ctrl, shift, alt);
        match self.keymap.lookup(&combo)? {
            KeyTrie::Leaf(Action::Command(id)) => Some(*id),
            _ => None,
        }
    }

    pub fn new(state: Entity<GraphState>, cx: &mut Context<Self>) -> Self {
        // Tick the physics simulation periodically
        let state_clone = state.clone();
        cx.spawn(async move |_this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let sc = state_clone.clone();
                let should_notify = cx.update(|cx| sc.update(cx, |s, _| s.tick()));
                match should_notify {
                    Ok(true) => {
                        // Physics moved — the app's cx.observe will trigger re-render
                    }
                    Ok(false) => {
                        // Sim settled, slow down
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(200))
                            .await;
                    }
                    Err(_) => break,
                }
            }
        })
        .detach();

        Self {
            state,
            drag_start: None,
            keymap: build_graph_layer(),
        }
    }
}

impl Render for GraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let gs = self.state.read(cx);
        let nodes = gs.nodes.clone();
        let edges = gs.edges.clone();
        let zoom = gs.zoom;
        let pan_x = gs.pan_x;
        let pan_y = gs.pan_y;
        let selected = gs.selected;
        let hovered = gs.hovered;
        let local_visible = gs.local_visible_set();
        let focus = gs.focus_handle.clone();

        div()
            .id("graph-view")
            .size_full()
            .bg(rgb(0xFDF6E3)) // solarized base3
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _window, cx| {
                let k = &e.keystroke;
                if let Some(cmd) = this.resolve_command(
                    k.key.as_str(),
                    k.modifiers.control,
                    k.modifiers.shift,
                    k.modifiers.alt,
                ) {
                    cx.emit(GraphViewEvent::Command(cmd));
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _window, cx| {
                    let mx: f32 = e.position.x.into();
                    let my: f32 = e.position.y.into();
                    let gs = this.state.read(cx);
                    let bounds = (800.0f32, 600.0f32);
                    if let Some(idx) = gs.node_at(mx, my, bounds.0, bounds.1) {
                        let path = gs.nodes[idx].path.clone();
                        drop(gs);
                        this.state.update(cx, |s, _| {
                            s.selected = Some(idx);
                        });
                        this.state.update(cx, |_s, cx| {
                            cx.emit(GraphEvent::OpenNote(path));
                        });
                    } else {
                        let gs_pan_x = gs.pan_x;
                        let gs_pan_y = gs.pan_y;
                        drop(gs);
                        this.drag_start = Some((mx, my, gs_pan_x, gs_pan_y));
                        this.state.update(cx, |s, _| s.selected = None);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _window, cx| {
                let mx: f32 = e.position.x.into();
                let my: f32 = e.position.y.into();
                if let Some((sx, sy, pan_x_start, pan_y_start)) = this.drag_start {
                    if e.pressed_button == Some(MouseButton::Left) {
                        let dx = mx - sx;
                        let dy = my - sy;
                        this.state.update(cx, |s, _| {
                            s.pan_x = pan_x_start + dx;
                            s.pan_y = pan_y_start + dy;
                        });
                        cx.notify();
                    } else {
                        this.drag_start = None;
                    }
                } else {
                    let gs = this.state.read(cx);
                    let hit = gs.node_at(mx, my, 800.0, 600.0);
                    if hit != gs.hovered {
                        drop(gs);
                        this.state.update(cx, |s, _| s.hovered = hit);
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, _cx| {
                    this.drag_start = None;
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
                    move |_bounds, _window, _cx| {},
                    {
                        let nodes = nodes.clone();
                        let edges = edges.clone();
                        let local_visible = local_visible.clone();
                        move |bounds, _prepaint: (), window, cx| {
                            let center_x: f32 = bounds.center().x.into();
                            let center_y: f32 = bounds.center().y.into();

                            // Draw edges
                            for edge in &edges {
                                if let Some(ref vis) = local_visible {
                                    if !vis.contains(&edge.source) || !vis.contains(&edge.target) {
                                        continue;
                                    }
                                }

                                let src = &nodes[edge.source];
                                let tgt = &nodes[edge.target];

                                let x1 = src.x * zoom + pan_x + center_x;
                                let y1 = src.y * zoom + pan_y + center_y;
                                let x2 = tgt.x * zoom + pan_x + center_x;
                                let y2 = tgt.y * zoom + pan_y + center_y;

                                let is_selected = selected == Some(edge.source)
                                    || selected == Some(edge.target);

                                let color = if is_selected {
                                    rgba(0x268BD2CC) // solarized blue
                                } else {
                                    rgba(0x93A1A140) // solarized base1, faint
                                };

                                let path = {
                                    let mut p = gpui::Path::new(point(px(x1), px(y1)));
                                    p.line_to(point(px(x2), px(y2)));
                                    p
                                };
                                window.paint_path(path, color);
                            }

                            // Draw nodes
                            let node_radius = 6.0 * zoom;
                            for (i, node) in nodes.iter().enumerate() {
                                if let Some(ref vis) = local_visible {
                                    if !vis.contains(&i) {
                                        continue;
                                    }
                                }

                                let nx = node.x * zoom + pan_x + center_x;
                                let ny = node.y * zoom + pan_y + center_y;

                                let is_sel = selected == Some(i);
                                let is_hov = hovered == Some(i);
                                let color = if is_sel {
                                    rgb(0xDC322F) // solarized red
                                } else if is_hov {
                                    rgb(0x268BD2) // solarized blue
                                } else {
                                    rgb(0x657B83) // solarized base00
                                };

                                let r = if is_sel || is_hov {
                                    node_radius * 1.3
                                } else {
                                    node_radius
                                };

                                // Draw filled circle (rounded rect with full corner radius)
                                let node_bounds = Bounds::new(
                                    point(px(nx - r), px(ny - r)),
                                    size(px(r * 2.0), px(r * 2.0)),
                                );
                                window.paint_quad(PaintQuad {
                                    bounds: node_bounds,
                                    corner_radii: Corners::all(px(r)),
                                    background: color.into(),
                                    border_widths: Edges::all(px(0.0)),
                                    border_color: transparent_black(),
                                    border_style: BorderStyle::default(),
                                });

                                // Draw label
                                if zoom > 0.3 {
                                    let font_size = (11.0 * zoom).max(8.0).min(14.0);
                                    let label_color = if is_sel {
                                        rgb(0xDC322F)
                                    } else {
                                        rgb(0x586E75) // solarized base01
                                    };

                                    let run = TextRun {
                                        len: node.title.len(),
                                        font: Font {
                                            family: "FiraCode Nerd Font".into(),
                                            features: FontFeatures::default(),
                                            fallbacks: None,
                                            weight: if is_sel {
                                                FontWeight::BOLD
                                            } else {
                                                FontWeight::NORMAL
                                            },
                                            style: FontStyle::Normal,
                                        },
                                        color: label_color.into(),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    };

                                    let shaped = window
                                        .text_system()
                                        .shape_line(
                                            node.title.clone().into(),
                                            px(font_size),
                                            &[run],
                                            None,
                                        );

                                    {
                                        let text_width: f32 = shaped.width.into();
                                        let text_x = nx - text_width / 2.0;
                                        let text_y = ny + r + 4.0;
                                        let _ = shaped.paint(
                                            point(px(text_x), px(text_y)),
                                            px(font_size),
                                            window,
                                            cx,
                                        );
                                    }
                                }
                            }
                        }
                    },
                )
                .size_full(),
            )
    }
}
