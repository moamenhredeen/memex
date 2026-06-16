use gpui::*;
use gpui_component::{Icon, IconName, h_flex};

use super::Memex;

impl Memex {
    pub(super) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = {
            let t = self.current_document_title(cx);
            if t.is_empty() { "Memex".to_string() } else { t }
        };
        let dirty = self.current_document_dirty(cx);
        let title_text = if dirty {
            format!("{} ●", title)
        } else {
            title
        };

        h_flex()
            .id("title-bar")
            .w_full()
            .items_center()
            .justify_between()
            .child(
                div()
                    .w(px(72.))
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .justify_center()
                    .items_center()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(self.theme.text))
                            .child(title_text),
                    ),
            )
            .child(h_flex().gap(px(0.)).child(self.title_bar_close_button(cx)))
    }

    fn title_bar_close_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("close-btn")
            .w(px(24.))
            .h(px(24.))
            .m_2()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .bg(rgba(0x00000010))
            .cursor_pointer()
            .hover(|s| s.text_color(rgba(0x00000010)).bg(rgba(0xFF000040)))
            .window_control_area(WindowControlArea::Close)
            .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.quit();
            }))
            .child(Icon::new(IconName::WindowClose))
    }
}
