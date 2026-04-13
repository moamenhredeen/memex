use gpui::*;

use super::commands::EditorCommand;
use super::element::EditorElement;
use super::{EditorEvent, EditorState, TabAction, ShiftTabAction};

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
                    state.dispatch(EditorCommand::OutlineCycleFold, window, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &ShiftTabAction, window, cx| {
                this.state.update(cx, |state, cx| {
                    state.dispatch(EditorCommand::OutlineGlobalCycle, window, cx);
                });
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.is_selecting = true;
                    let pos = this.state.read(cx).index_for_mouse_position(e.position);

                    // Check for wikilink click (Ctrl+click or plain click)
                    if let Some(title) = this.state.read(cx).wikilink_at_offset(pos) {
                        this.is_selecting = false;
                        this.state.update(cx, |state, cx| {
                            cx.emit(EditorEvent::WikilinkClicked(title));
                            let _ = state;
                        });
                        return;
                    }

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
