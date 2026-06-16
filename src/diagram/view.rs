use gpui::*;

use super::DiagramState;
use crate::keymap::{Action, KeyContext, KeymapSystem, ResolvedKey};
use crate::theme::Theme;

/// Emitted when a keybinding resolves to a command.
#[derive(Clone, Debug)]
pub enum DiagramViewEvent {
    Command(&'static str),
}

impl EventEmitter<DiagramViewEvent> for DiagramView {}

pub struct DiagramView {
    state: Entity<DiagramState>,
    keymap: KeymapSystem,
    /// Pan drag anchor: (mouse_x, mouse_y, pan_x_start, pan_y_start).
    drag_start: Option<(f32, f32, f32, f32)>,
    theme: Theme,
}

impl DiagramView {
    pub fn new(state: Entity<DiagramState>, theme: Theme, _cx: &mut Context<Self>) -> Self {
        Self {
            state,
            keymap: KeymapSystem::new(false),
            drag_start: None,
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

/// Parse a `#rrggbb` color into a packed `0xRRGGBB`, ignoring alpha.
fn parse_hex_rgb(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    let rgb = match hex.len() {
        6 | 8 => &hex[..6],
        3 => {
            // Shorthand #rgb -> expand each nibble.
            let mut expanded = String::with_capacity(6);
            for c in hex.chars() {
                expanded.push(c);
                expanded.push(c);
            }
            return u32::from_str_radix(&expanded, 16).ok();
        }
        _ => return None,
    };
    u32::from_str_radix(rgb, 16).ok()
}

impl Render for DiagramView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ds = self.state.read(cx);
        let focus = ds.focus_handle.clone();
        let theme = self.theme;
        let count = ds.element_count();
        let bg = ds
            .file
            .background_color()
            .and_then(parse_hex_rgb)
            .unwrap_or(theme.background);

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
                    let ds = this.state.read(cx);
                    this.drag_start = Some((mx, my, ds.pan_x, ds.pan_y));
                }),
            )
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _window, cx| {
                if let Some((sx, sy, pan_x0, pan_y0)) = this.drag_start {
                    if e.pressed_button == Some(MouseButton::Left) {
                        let mx: f32 = e.position.x.into();
                        let my: f32 = e.position.y.into();
                        this.state.update(cx, |s, _| {
                            s.pan_x = pan_x0 + (mx - sx);
                            s.pan_y = pan_y0 + (my - sy);
                        });
                        cx.notify();
                    } else {
                        this.drag_start = None;
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
            // Phase 0 placeholder: element rendering arrives in Phase 1.
            .child(
                div()
                    .absolute()
                    .top_4()
                    .left_4()
                    .text_color(rgb(theme.text_muted))
                    .child(SharedString::from(format!(
                        "Diagram - {} element(s) - rendering in Phase 1",
                        count
                    ))),
            )
    }
}
