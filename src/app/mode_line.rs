use gpui::*;
use gpui_component::h_flex;

use super::Memex;

impl Memex {
    pub(super) fn render_mode_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editor_view = self.active_editor_view();
        let ev = editor_view.read(cx);
        let vim_enabled = ev.keymap.vim_enabled;
        let vim_state = ev.keymap.active_vim_state().map(|s| s.to_string());

        let vault_name = self.state.vault_name();
        let note_title = self.current_document_title(cx);
        let dirty = self.current_document_dirty(cx);
        let dirty_indicator = if dirty { " ●" } else { "" };

        let focused_item = self.focused_item();
        let position_text = focused_item.position_text(600.0, cx);

        let show_non_editor =
            focused_item.is_pdf() || focused_item.is_graph() || focused_item.is_backlinks();
        let mode_badge = if show_non_editor {
            let (label, color) = focused_item.mode_badge();
            div().px(px(6.)).py(px(1.)).bg(rgb(color)).child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(self.theme.background))
                    .child(label),
            )
        } else if vim_enabled {
            let (label, bg) = match vim_state.as_deref() {
                Some("NORMAL") => ("NORMAL", rgb(self.theme.accent)),
                Some("INSERT") => ("INSERT", rgb(self.theme.success)),
                Some("VISUAL") => ("VISUAL", rgb(self.theme.violet)),
                Some("V-LINE") => ("V-LINE", rgb(self.theme.violet)),
                _ => ("NOR", rgb(self.theme.accent)),
            };
            div().px(px(6.)).py(px(1.)).bg(bg).child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(self.theme.background))
                    .child(label),
            )
        } else {
            div()
                .px(px(6.))
                .py(px(1.))
                .bg(rgb(self.theme.success))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(self.theme.background))
                        .child("EDT"),
                )
        };

        h_flex()
            .w_full()
            .h(px(24.))
            .bg(rgb(self.theme.surface))
            .items_center()
            .gap(px(0.))
            .child(mode_badge)
            .child(
                div().px(px(8.)).child(
                    div()
                        .text_size(px(14.))
                        .text_color(rgb(self.theme.text_strong))
                        .child(format!(
                            " {} › {}{}",
                            vault_name, note_title, dirty_indicator
                        )),
                ),
            )
            .child(div().flex_1())
            .child(
                div().px(px(8.)).child(
                    div()
                        .text_size(px(14.))
                        .text_color(rgb(self.theme.text_muted))
                        .child(position_text),
                ),
            )
    }
}
