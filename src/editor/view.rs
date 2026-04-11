use gpui::*;

use super::element::EditorElement;
use super::{EditorState, TabAction, ShiftTabAction};

pub struct EditorView {
    pub state: Entity<EditorState>,
    focus_handle: FocusHandle,
    is_selecting: bool,
    _observe_state: Subscription,
}

impl EditorView {
    pub fn new(state: Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        let _observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            focus_handle,
            is_selecting: false,
            _observe_state,
        }
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("editor-view")
            .size_full()
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .key_context("Editor")
            .on_action(cx.listener(|this, _: &TabAction, window, cx| {
                this.state.update(cx, |state, cx| {
                    if !state.handle_table_tab(true, cx) {
                        state.replace_text_in_range(None, "    ", window, cx);
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &ShiftTabAction, _window, cx| {
                this.state.update(cx, |state, cx| {
                    state.handle_table_tab(false, cx);
                });
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let shift = e.keystroke.modifiers.shift;

                this.state.update(cx, |state, cx| {
                    let content = state.content();
                    match key {
                    "backspace" => {
                        if state.selected_range.is_empty() {
                            state.select_to(state.prev_grapheme(state.cursor_offset()), cx);
                        }
                        state.replace_text_in_range(None, "", window, cx);
                    }
                    "delete" => {
                        if state.selected_range.is_empty() {
                            state.select_to(state.next_grapheme(state.cursor_offset()), cx);
                        }
                        state.replace_text_in_range(None, "", window, cx);
                    }
                    "left" => {
                        if shift {
                            state.select_to(state.prev_grapheme(state.cursor_offset()), cx);
                        } else if state.selected_range.is_empty() {
                            state.move_to(state.prev_grapheme(state.cursor_offset()), cx);
                        } else {
                            state.move_to(state.selected_range.start, cx);
                        }
                    }
                    "right" => {
                        if shift {
                            state.select_to(state.next_grapheme(state.cursor_offset()), cx);
                        } else if state.selected_range.is_empty() {
                            state.move_to(state.next_grapheme(state.cursor_offset()), cx);
                        } else {
                            state.move_to(state.selected_range.end, cx);
                        }
                    }
                    "up" => {
                        let pos = state.cursor;
                        let before = &content[..pos.min(content.len())];
                        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                        let col = pos - line_start;
                        if line_start == 0 {
                            state.move_to(0, cx);
                        } else {
                            let prev_end = line_start - 1;
                            let prev_start = content[..prev_end]
                                .rfind('\n')
                                .map(|i| i + 1)
                                .unwrap_or(0);
                            let prev_len = prev_end - prev_start;
                            state.move_to(prev_start + col.min(prev_len), cx);
                        }
                    }
                    "down" => {
                        let pos = state.cursor;
                        let before = &content[..pos.min(content.len())];
                        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                        let col = pos - line_start;
                        let after = &content[pos..];
                        if let Some(nl) = after.find('\n') {
                            let next_start = pos + nl + 1;
                            let rest = &content[next_start..];
                            let next_len = rest.find('\n').unwrap_or(rest.len());
                            state.move_to(next_start + col.min(next_len), cx);
                        } else {
                            state.move_to(content.len(), cx);
                        }
                    }
                    "home" => {
                        let pos = state.cursor.min(content.len());
                        let line_start =
                            content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
                        state.move_to(line_start, cx);
                    }
                    "end" => {
                        let pos = state.cursor.min(content.len());
                        let line_end = content[pos..]
                            .find('\n')
                            .map(|p| pos + p)
                            .unwrap_or(content.len());
                        state.move_to(line_end, cx);
                    }
                    "enter" => {
                        state.replace_text_in_range(None, "\n", window, cx);
                    }
                    _ => {}
                }});
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.is_selecting = true;
                    let pos = this.state.read(cx).index_for_mouse_position(e.position);
                    this.state.update(cx, |state, cx| {
                        if e.modifiers.shift {
                            state.select_to(pos, cx);
                        } else {
                            state.move_to(pos, cx);
                        }
                        state.focus_handle.focus(window);
                        state.blink_cursor.update(cx, |bc, cx| bc.start(cx));
                    });
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, _cx| {
                    this.is_selecting = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, _cx| {
                    this.is_selecting = false;
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _window, cx| {
                if this.is_selecting {
                    let pos = this.state.read(cx).index_for_mouse_position(e.position);
                    this.state.update(cx, |state, cx| {
                        state.select_to(pos, cx);
                    });
                }
            }))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _window, cx| {
                this.state.update(cx, |state, cx| {
                    let delta = match e.delta {
                        ScrollDelta::Lines(lines) => lines.y * px(20.),
                        ScrollDelta::Pixels(pixels) => pixels.y,
                    };
                    state.scroll_offset = (state.scroll_offset - delta).max(px(0.));
                    cx.notify();
                });
            }))
            .child(EditorElement::new(&self.state))
    }
}
