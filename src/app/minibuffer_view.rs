use gpui::*;
use gpui_component::{h_flex, v_flex};

use super::{Memex, ui_helpers};
use crate::minibuffer::{DelegateKind, MinibufferVimMode};

impl Memex {
    /// Render the minibuffer area — unified, single rendering path.
    /// Always visible like emacs: shows echo area messages when idle,
    /// prompt + input + vertico candidates when active.
    pub(super) fn render_minibuffer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let base = v_flex().w_full().bg(rgb(self.theme.background));

        if !self.minibuffer.active {
            let msg = self
                .minibuffer
                .message
                .clone()
                .or_else(|| self.active_editor_state().read(cx).status_message.clone())
                .unwrap_or_default();
            return base.child(
                h_flex().w_full().h(px(22.)).px(px(8.)).py(px(3.)).child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(self.theme.text_muted))
                        .child(msg),
                ),
            );
        }

        let candidates = self.get_candidates(cx);
        let selected = self.minibuffer.selected;
        let (before_cursor, after_cursor) = self.minibuffer.input_parts();
        let cursor_char = match self.minibuffer.vim_mode {
            MinibufferVimMode::Normal => "█",
            MinibufferVimMode::Insert => "│",
        };

        let max_visible = 10usize;
        let candidate_area_h = px((max_visible as f32) * 20.0);
        let scroll_top = if candidates.len() <= max_visible {
            0
        } else if selected < max_visible / 2 {
            0
        } else if selected + max_visible / 2 >= candidates.len() {
            candidates.len().saturating_sub(max_visible)
        } else {
            selected - max_visible / 2
        };
        let visible_end = (scroll_top + max_visible).min(candidates.len());

        let mut items = v_flex().w_full().h(candidate_area_h);
        for i in scroll_top..visible_end {
            let candidate = &candidates[i];
            let is_selected = i == selected;
            let bg_color = if is_selected {
                rgb(self.theme.selection)
            } else {
                rgb(self.theme.background)
            };
            let text_color = if candidate.is_action {
                rgb(self.theme.success)
            } else if is_selected {
                rgb(self.theme.text_strong)
            } else {
                rgb(self.theme.text)
            };

            let label_element = if matches!(self.minibuffer.delegate_kind, DelegateKind::Item(ref id) if self.focused_item().highlight_input(id))
                && !self.minibuffer.input.is_empty()
            {
                ui_helpers::render_highlighted_label(
                    &candidate.label,
                    &self.minibuffer.input,
                    text_color,
                    self.theme.warning,
                )
            } else {
                div()
                    .text_size(px(13.))
                    .text_color(text_color)
                    .child(candidate.label.clone())
            };

            let mut row = h_flex().gap(px(8.)).child(label_element);
            if let Some(detail) = &candidate.detail {
                row = row.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(self.theme.text_muted))
                        .child(detail.clone()),
                );
            }

            items = items.child(
                div()
                    .id(ElementId::Name(format!("mb-item-{}", i).into()))
                    .w_full()
                    .px(px(8.))
                    .py(px(2.))
                    .bg(bg_color)
                    .child(row),
            );
        }

        base.border_t_1()
            .border_color(rgb(self.theme.border))
            .track_focus(&self.minibuffer_focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let ctrl = e.keystroke.modifiers.control;
                let shift = e.keystroke.modifiers.shift;
                this.handle_minibuffer_key(key, ctrl, shift, window, cx);
            }))
            .child(
                h_flex()
                    .w_full()
                    .px(px(8.))
                    .py(px(3.))
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(self.theme.accent))
                            .child(self.minibuffer.prompt.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(self.theme.text_strong))
                            .child(format!("{}{}{}", before_cursor, cursor_char, after_cursor)),
                    ),
            )
            .child(items)
    }
}
