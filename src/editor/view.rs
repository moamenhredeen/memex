use gpui::*;

use super::commands::EditorCommand;
use super::element::EditorElement;
use super::{EditorEvent, EditorState, TabAction, ShiftTabAction};
use crate::keymap::{Action, KeymapSystem, ResolvedKey};
use crate::pane::{ItemAction, VimSnapshot};

/// Events from [`EditorView`] that the app shell processes.
///
/// `Command` and `ItemActions` cross the view → app boundary because they
/// reach into the minibuffer / clipboard / pane layout. Vim / insert-mode
/// mutations stay inside the view and are not emitted.
#[derive(Clone, Debug)]
pub enum EditorViewEvent {
    /// A keymap-resolved command — runs through `Memex::execute_command`.
    Command(&'static str, usize),
    /// `ItemAction`s produced by grammar execution (minibuffer delegate,
    /// clipboard write, vim toggle, etc.).
    ItemActions(Vec<ItemAction>),
    /// The editor's vim/insert/visual state changed. The app doesn't need
    /// to do anything except re-render the mode-line.
    VimStateChanged,
}

impl EventEmitter<EditorViewEvent> for EditorView {}

pub struct EditorView {
    pub state: Entity<EditorState>,
    /// Editor-owned keymap. Only resolves when this view has focus.
    pub keymap: KeymapSystem,
    focus_handle: FocusHandle,
    is_selecting: bool,
    _observe_state: Subscription,
}

impl EditorView {
    pub fn new(state: Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        let _observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        let keymap = KeymapSystem::new(true);
        let mut this = Self {
            state,
            keymap,
            focus_handle,
            is_selecting: false,
            _observe_state,
        };
        this.sync_state_vim_flags(cx);
        this
    }

    /// Snapshot of vim state for passing into `EditorState::process_vim_action`.
    pub fn vim_snapshot(&self) -> VimSnapshot {
        VimSnapshot {
            vim_enabled: self.keymap.vim_enabled,
            visual_active: self.keymap.is_visual_active(),
            insert_active: self.keymap.is_insert_active(),
        }
    }

    /// Mirror keymap vim/insert flags onto `EditorState` so the cursor
    /// renderer and other consumers can read them without holding a view
    /// reference.
    pub fn sync_state_vim_flags(&self, cx: &mut Context<Self>) {
        let vim = self.keymap.vim_enabled;
        let insert = self.keymap.is_insert_active();
        self.state.update(cx, |s, _cx| s.sync_vim_flags(vim, insert));
    }

    /// Toggle vim mode on or off.
    pub fn set_vim_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.keymap.set_vim_enabled(enabled);
        self.sync_state_vim_flags(cx);
        cx.emit(EditorViewEvent::VimStateChanged);
    }

    /// Dispatch a keystroke through the editor's keymap. Returns `true` if
    /// the key was consumed — the caller should `stop_propagation`. Returns
    /// `false` when the key is unbound, so gpui's input handler path can
    /// insert the character (insert-mode typing).
    fn dispatch_key(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let resolved = self.keymap.resolve_key(key, ctrl, shift, alt);
        match resolved {
            ResolvedKey::Action(action, count) => match &action {
                Action::Command(cmd_id) => {
                    cx.emit(EditorViewEvent::Command(*cmd_id, count));
                    true
                }
                Action::ActivateLayer(layer_id) => {
                    self.keymap.stack.activate_layer(layer_id);
                    self.state.update(cx, |s, cx| s.on_layer_activated(layer_id, cx));
                    self.sync_state_vim_flags(cx);
                    cx.emit(EditorViewEvent::VimStateChanged);
                    cx.notify();
                    true
                }
                _ => {
                    // Motion / Operator / SelfInsert — handed to the grammar.
                    let vim = self.vim_snapshot();
                    let stack = &mut self.keymap.stack;
                    let actions = self.state.update(cx, |state, ecx| {
                        state.process_vim_action(action, key, count, vim, stack, window, ecx)
                    });
                    if !actions.is_empty() {
                        cx.emit(EditorViewEvent::ItemActions(actions));
                    }
                    true
                }
            },
            ResolvedKey::TransientCapture { transient_id, ch, count } => {
                self.state.update(cx, |s, ecx| {
                    s.handle_transient_capture(&transient_id, ch, count, window, ecx);
                });
                true
            }
            ResolvedKey::Pending => {
                self.state.update(cx, |s, _| s.suppress_next_input = true);
                true
            }
            // Unhandled: leave propagation alone so gpui's input handler can
            // insert the character through `EntityInputHandler`.
            ResolvedKey::Unhandled => false,
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
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                let handled = this.dispatch_key(
                    k.key.as_str(),
                    k.modifiers.control,
                    k.modifiers.shift,
                    k.modifiers.alt,
                    window,
                    cx,
                );
                if handled {
                    cx.stop_propagation();
                }
                // If not handled, let propagation continue so gpui's input
                // handler can insert the character (insert-mode typing).
            }))
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

                    // Check for checkbox click
                    if let Some(range) = this.state.read(cx).checkbox_at_offset(pos) {
                        this.is_selecting = false;
                        this.state.update(cx, |state, cx| {
                            state.toggle_checkbox(range, cx);
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
