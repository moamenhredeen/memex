use std::path::PathBuf;

use gpui::*;
use gpui_component::{h_flex, v_flex};

use crate::command::Command;
use crate::minibuffer::Candidate;
use crate::pane::{CommandOutcome, ItemAction};
use crate::theme::Theme;

pub struct BacklinksState {
    pub current_title: String,
    pub backlinks: Vec<(String, PathBuf)>,
    pub focus_handle: FocusHandle,
}

impl BacklinksState {
    pub fn new(
        current_title: String,
        backlinks: Vec<(String, PathBuf)>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            current_title,
            backlinks,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    pub fn commands() -> Vec<Command> {
        Vec::new()
    }

    pub fn execute_command(
        &mut self,
        _cmd_id: &str,
        _viewport: (f32, f32),
        _vim_enabled: bool,
        _cx: &mut Context<Self>,
    ) -> CommandOutcome {
        CommandOutcome::Unhandled
    }

    pub fn get_candidates(&self, _delegate_id: &str, _input: &str) -> Vec<Candidate> {
        Vec::new()
    }

    pub fn handle_confirm(
        &mut self,
        _delegate_id: &str,
        _input: &str,
        _candidate: Option<&Candidate>,
        _cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        Vec::new()
    }

    pub fn on_input_changed(&mut self, _delegate_id: &str, _input: &str, _cx: &mut Context<Self>) {}
}

#[derive(Clone, Debug)]
pub enum BacklinksViewEvent {
    OpenPath(PathBuf),
}

impl EventEmitter<BacklinksViewEvent> for BacklinksView {}

pub struct BacklinksView {
    state: Entity<BacklinksState>,
    theme: Theme,
}

impl BacklinksView {
    pub fn new(state: Entity<BacklinksState>, theme: Theme, _cx: &mut Context<Self>) -> Self {
        Self { state, theme }
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }
}

impl Render for BacklinksView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let current_title = state.current_title.clone();
        let backlinks = state.backlinks.clone();
        let focus = state.focus_handle.clone();
        let theme = self.theme;

        let header = h_flex()
            .w_full()
            .px(px(8.))
            .py(px(3.))
            .bg(rgb(theme.surface))
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme.text_strong))
                    .child("Backlinks"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(theme.text_muted))
                    .child(format!("{}", backlinks.len())),
            );

        let mut list = v_flex()
            .w_full()
            .flex_1()
            .overflow_hidden()
            .bg(rgb(theme.background));

        if backlinks.is_empty() {
            list = list.child(
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(theme.text_muted))
                    .child(if current_title.is_empty() {
                        "No note open."
                    } else {
                        "No backlinks yet. Link to this note from another note with [[…]]."
                    }),
            );
        } else {
            for (title, path) in backlinks {
                list = list.child(
                    div()
                        .id(ElementId::Name(
                            format!("bl-{}", path.to_string_lossy()).into(),
                        ))
                        .w_full()
                        .px(px(10.))
                        .py(px(3.))
                        .text_size(px(12.))
                        .text_color(rgb(theme.accent))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(0x00000010)))
                        .child(title)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |_this, _e: &MouseDownEvent, _window, cx| {
                                cx.emit(BacklinksViewEvent::OpenPath(path.clone()));
                            }),
                        ),
                );
            }
        }

        v_flex()
            .w_full()
            .h_full()
            .track_focus(&focus)
            .child(header)
            .child(list)
    }
}
