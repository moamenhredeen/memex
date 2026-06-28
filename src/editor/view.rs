use gpui::*;
use std::collections::HashMap;
use std::path::PathBuf;

use super::code_edit;
use super::element::EditorElement;
use super::outline;
use super::{DIAGRAM_EMBED_HEIGHT_PX, EditorEvent, EditorState, ShiftTabAction, TabAction};
use crate::diagram::{self, ChromeConfig, DiagramView, Mode};
use crate::keymap::{Action, KeyContext, KeymapSystem, ResolvedKey};
use crate::pane::{ItemAction, VimSnapshot};
use crate::theme::Theme;

const MIN_HORIZONTAL_PADDING: f32 = 24.0;
const VERTICAL_PADDING: f32 = 24.0;

fn content_inset(viewport_width: Pixels, editor_width: Pixels) -> Pixels {
    let viewport_width: f32 = viewport_width.into();
    let editor_width: f32 = editor_width.into();
    px(((viewport_width - editor_width) / 2.0).max(MIN_HORIZONTAL_PADDING))
}

fn content_width(viewport_width: Pixels, editor_width: Pixels) -> Pixels {
    viewport_width - content_inset(viewport_width, editor_width) * 2.0
}

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
    /// Open a diagram link target in the host's secondary/right pane.
    OpenDiagram(String),
}

impl EventEmitter<EditorViewEvent> for EditorView {}

pub struct EditorView {
    pub state: Entity<EditorState>,
    /// Editor-owned keymap. Only resolves when this view has focus.
    pub keymap: KeymapSystem,
    focus_handle: FocusHandle,
    is_selecting: bool,
    theme: Theme,
    editor_width: Pixels,
    diagram_embeds: HashMap<PathBuf, Entity<DiagramView>>,
    _observe_state: Subscription,
}

impl EditorView {
    pub fn new(
        state: Entity<EditorState>,
        theme: Theme,
        editor_width: u32,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        let _observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        let keymap = KeymapSystem::new(true);
        let this = Self {
            state,
            keymap,
            focus_handle,
            is_selecting: false,
            theme,
            editor_width: px(editor_width as f32),
            diagram_embeds: HashMap::new(),
            _observe_state,
        };
        this.sync_state_vim_flags(cx);
        this
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        for view in self.diagram_embeds.values() {
            view.update(cx, |view, cx| {
                view.set_theme(diagram::theme_from_memex(theme), cx)
            });
        }
        cx.notify();
    }

    pub fn reload_diagram_embed(&mut self, path: &PathBuf, cx: &mut Context<Self>) {
        self.diagram_embeds.remove(path);
        cx.notify();
    }

    fn diagram_embed_view(
        &mut self,
        path: &PathBuf,
        cx: &mut Context<Self>,
    ) -> Option<Entity<DiagramView>> {
        if let Some(view) = self.diagram_embeds.get(path) {
            return Some(view.clone());
        }
        let graph = diagram::load_graph(path).ok()?;
        let base_dir = path.parent().map(|parent| parent.to_path_buf());
        let theme = self.theme;
        let view = cx.new(|cx| {
            let mut view = if let Some(base_dir) = base_dir.as_ref() {
                DiagramView::with_base_dir(graph, base_dir, cx)
            } else {
                DiagramView::new(graph, cx)
            };
            view.set_theme(diagram::theme_from_memex(theme), cx);
            view.set_mode(Mode::View, cx);
            view.set_chrome(ChromeConfig::bare(), cx);
            view.fit_to_content(cx);
            view
        });
        self.diagram_embeds.insert(path.clone(), view.clone());
        Some(view)
    }

    fn render_diagram_embeds(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let bounds = self.state.read(cx).last_bounds;
        let Some(bounds) = bounds else {
            return Vec::new();
        };
        let wrap_width = content_width(bounds.size.width, self.editor_width);

        self.state.update(cx, |state, cx| {
            state.prepare_display_layout(wrap_width, bounds.size.height, cx);
        });

        let (content, scroll, cursor, line_count) = {
            let state = self.state.read(cx);
            (
                state.content(),
                state.scroll_offset,
                state.cursor,
                state.display_map.line_count(),
            )
        };
        if line_count == 0 {
            return Vec::new();
        }

        let horizontal_inset = content_inset(bounds.size.width, self.editor_width);
        let (first, last) = {
            let state = self.state.read(cx);
            state
                .display_map
                .visible_range(scroll, bounds.size.height, 4)
        };
        let mut embeds = Vec::new();
        for line_idx in first..last {
            let (line_start, line_end, line_y, hidden) = {
                let state = self.state.read(cx);
                let Some(range) = state.line_text_range(line_idx, &content) else {
                    continue;
                };
                (
                    range.start,
                    range.end,
                    state.display_map.line_y(line_idx),
                    state.display_map.is_line_hidden(line_idx),
                )
            };
            if hidden || (cursor >= line_start && cursor <= line_end) {
                continue;
            }
            let Some(line_text) = content.get(line_start..line_end) else {
                continue;
            };
            let Some(embed) = self.state.read(cx).diagram_embed_for_line(line_text) else {
                continue;
            };
            let top = px(VERTICAL_PADDING) - scroll + line_y;
            let left = horizontal_inset;
            let height = px(DIAGRAM_EMBED_HEIGHT_PX - 16.0);
            let view = self.diagram_embed_view(&embed.path, cx);
            let target = embed.target.clone();
            let body = if let Some(view) = view {
                div()
                    .size_full()
                    .overflow_hidden()
                    .child(view)
                    .into_any_element()
            } else {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(self.theme.text_muted))
                    .child(format!("Diagram not found: {}", embed.target))
                    .into_any_element()
            };
            embeds.push(
                div()
                    .id(ElementId::Name(
                        format!("diagram-embed-{}", line_start).into(),
                    ))
                    .absolute()
                    .left(left)
                    .top(top)
                    .w(wrap_width)
                    .h(height)
                    .bg(rgb(self.theme.background))
                    .border_1()
                    .border_color(rgb(self.theme.border))
                    .rounded_md()
                    .overflow_hidden()
                    .child(body)
                    .child(
                        div()
                            .absolute()
                            .top(px(8.0))
                            .right(px(8.0))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(self.theme.surface))
                            .border_1()
                            .border_color(rgb(self.theme.border))
                            .text_color(rgb(self.theme.text))
                            .text_size(px(12.0))
                            .cursor_pointer()
                            .child("Open")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |_this, _e: &MouseDownEvent, _window, cx| {
                                    cx.emit(EditorViewEvent::OpenDiagram(target.clone()));
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                    .into_any_element(),
            );
        }
        embeds
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
        self.state
            .update(cx, |s, _cx| s.sync_vim_flags(vim, insert));
    }

    fn key_context(&self, cx: &mut Context<Self>) -> KeyContext {
        let mut context = KeyContext::new();
        context.add("Editor");
        context.set("vim_mode", self.keymap.active_vim_mode());

        let state = self.state.read(cx);
        let content = state.content();
        let cursor = state.cursor_offset();
        if code_edit::is_inside_code_block(&content, cursor) {
            context.add("code_block");
        }
        if state.cursor_is_in_table() {
            context.add("table");
        }
        let line = state.display_map.line_for_offset(cursor);
        let headings = outline::extract_headings(&state.display_map.line_kinds());
        if outline::heading_for_line(line, &headings).is_some() {
            context.add("heading");
        }

        context
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
        let context = self.key_context(cx);
        let resolved = self.keymap.resolve_key(key, ctrl, shift, alt, &context);
        match resolved {
            ResolvedKey::Action(action, count) => match &action {
                Action::Command(cmd_id) => {
                    cx.emit(EditorViewEvent::Command(*cmd_id, count));
                    true
                }
                Action::SetVimMode(mode) => {
                    self.keymap.set_vim_mode(*mode);
                    self.state
                        .update(cx, |s, cx| s.on_vim_mode_changed(*mode, cx));
                    self.sync_state_vim_flags(cx);
                    cx.emit(EditorViewEvent::VimStateChanged);
                    cx.notify();
                    true
                }
                _ => {
                    // Motion / Operator / SelfInsert — handed to the grammar.
                    let vim = self.vim_snapshot();
                    let registry = &self.keymap.registry;
                    let actions = self.state.update(cx, |state, ecx| {
                        state.process_vim_action(action, key, count, vim, registry, window, ecx)
                    });
                    for action in &actions {
                        match action {
                            ItemAction::SetVimMode(mode) => self.keymap.set_vim_mode(*mode),
                            ItemAction::PushTransient(transient) => {
                                self.keymap.push_transient(*transient)
                            }
                            _ => {}
                        }
                    }
                    if !actions.is_empty() {
                        cx.emit(EditorViewEvent::ItemActions(actions));
                    }
                    true
                }
            },
            ResolvedKey::TransientCapture {
                transient,
                ch,
                count,
            } => {
                self.state.update(cx, |s, ecx| {
                    s.handle_transient_capture(transient, ch, count, window, ecx);
                });
                true
            }
            ResolvedKey::Pending => true,
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
            .relative()
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
                if this.dispatch_key("tab", false, false, false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &ShiftTabAction, window, cx| {
                if this.dispatch_key("tab", false, true, false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.is_selecting = true;
                    let pos = this.state.read(cx).index_for_mouse_position(e.position);

                    // Standalone diagram links render as embedded previews. Opening them is
                    // reserved for the overlay button so canvas interaction doesn't open a pane.
                    if this.state.read(cx).offset_is_diagram_embed_line(pos) {
                        this.is_selecting = false;
                        this.state.read(cx).focus_handle.focus(window);
                        return;
                    }

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
                    let total = <EditorState as crate::ui::Scrollable>::total_height(state);
                    let viewport: f32 = state.viewport_height.into();
                    let max = px((total - viewport).max(0.0));
                    state.scroll_offset = (state.scroll_offset - delta).clamp(px(0.), max);
                    cx.notify();
                });
            }))
            .child(EditorElement::new(
                &self.state,
                self.theme,
                self.editor_width,
            ))
            .children(self.render_diagram_embeds(cx))
            .child(crate::ui::Scrollbar::new(self.state.clone()).with_id("editor-scrollbar"))
    }
}
