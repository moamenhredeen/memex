use gpui::*;

use super::commands::EditorCommand;
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
                    state.dispatch(EditorCommand::InsertTab, window, cx);
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
                let ctrl = e.keystroke.modifiers.control;

                let cmd = match (key, ctrl, shift) {
                    ("z", true, false) => Some(EditorCommand::Undo),
                    ("z", true, true) => Some(EditorCommand::Redo),
                    ("backspace", false, _) => Some(EditorCommand::DeleteBackward),
                    ("delete", false, _) => Some(EditorCommand::DeleteForward),
                    ("left", false, true) => Some(EditorCommand::SelectLeft),
                    ("right", false, true) => Some(EditorCommand::SelectRight),
                    ("left", false, false) => Some(EditorCommand::MoveLeft),
                    ("right", false, false) => Some(EditorCommand::MoveRight),
                    ("up", false, _) => Some(EditorCommand::MoveUp),
                    ("down", false, _) => Some(EditorCommand::MoveDown),
                    ("home", false, _) => Some(EditorCommand::MoveLineStart),
                    ("end", false, _) => Some(EditorCommand::MoveLineEnd),
                    ("enter", false, _) => Some(EditorCommand::InsertNewline),
                    _ => None,
                };

                if let Some(cmd) = cmd {
                    this.state.update(cx, |state, cx| {
                        state.dispatch(cmd, window, cx);
                    });
                }
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
