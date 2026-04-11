use gpui::*;
use gpui_component::{h_flex, v_flex};

use super::commands::EditorCommand;
use super::element::EditorElement;
use super::keymap::{EditorMode, KeyCombo};
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
        let state = self.state.read(cx);
        let vim_enabled = state.vim.enabled;
        let mode = state.mode;
        let command_line = state.command_line.clone();
        let status_msg = state.status_message.clone();
        drop(state);

        let mut root = v_flex()
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
                let ctrl = e.keystroke.modifiers.control;
                let shift = e.keystroke.modifiers.shift;
                let alt = e.keystroke.modifiers.alt;

                this.state.update(cx, |state, cx| {
                    let mode = state.mode;

                    // In vim Normal/Visual modes, route to vim handler
                    if state.vim.enabled {
                        match mode {
                            EditorMode::Command => {
                                state.handle_command_key(key, ctrl, window, cx);
                                return;
                            }
                            EditorMode::Normal
                            | EditorMode::Visual
                            | EditorMode::VisualLine => {
                                state.handle_vim_key(key, window, cx);
                                return;
                            }
                            EditorMode::Insert => {
                                if key == "escape" {
                                    state.mode = EditorMode::Normal;
                                    state.history.break_coalescing();
                                    cx.notify();
                                    return;
                                }
                                // Fall through to normal keymap handling
                            }
                        }
                    }

                    // Standard keymap-based dispatch
                    let combo = KeyCombo::from_keystroke(key, ctrl, shift, alt);
                    if let Some(cmd) = state.keymap.resolve(mode, &combo) {
                        state.dispatch(cmd, window, cx);
                    }
                });
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
            // Editor canvas takes all available space
            .child(
                div().flex_1().w_full().child(EditorElement::new(&self.state)),
            );

        // Vim mode indicator + command bar at the bottom
        if vim_enabled {
            if mode == EditorMode::Command {
                // Command bar: shows `:` prefix + typed input
                root = root.child(
                    h_flex()
                        .w_full()
                        .h(px(26.))
                        .bg(rgb(0x1E1E2E))
                        .items_center()
                        .px(px(8.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_family("FiraCode Nerd Font")
                                .text_color(rgb(0xCDD6F4))
                                .child(format!(":{}█", command_line)),
                        ),
                );
            } else {
                // Mode indicator bar with optional status message
                let (mode_label, mode_bg) = match mode {
                    EditorMode::Normal => ("NORMAL", rgb(0x89B4FA)),
                    EditorMode::Insert => ("INSERT", rgb(0xA6E3A1)),
                    EditorMode::Visual => ("VISUAL", rgb(0xCBA6F7)),
                    EditorMode::VisualLine => ("V-LINE", rgb(0xCBA6F7)),
                    EditorMode::Command => unreachable!(),
                };

                let right_text = status_msg.unwrap_or_default();

                root = root.child(
                    h_flex()
                        .w_full()
                        .h(px(26.))
                        .bg(rgb(0x1E1E2E))
                        .items_center()
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(2.))
                                .bg(mode_bg)
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(0x1E1E2E))
                                        .child(mode_label),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .px(px(8.))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(0x6C7086))
                                        .child(right_text),
                                ),
                        ),
                );
            }
        }

        root
    }
}
