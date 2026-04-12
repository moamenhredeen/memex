use std::sync::Arc;

use gpui::*;

use super::PdfState;

pub struct PdfView {
    pub state: Entity<PdfState>,
    focus_handle: FocusHandle,
    _observe_state: Subscription,
}

impl PdfView {
    pub fn new(state: Entity<PdfState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        let _observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            focus_handle,
            _observe_state,
        }
    }
}

impl Focusable for PdfView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PdfView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page_count = self.state.read(cx).page_count;
        let scroll_offset = self.state.read(cx).scroll_offset;

        // Pre-render pages and collect image data
        let mut page_images: Vec<(usize, Arc<gpui::Image>, u32, u32)> = Vec::new();
        for i in 0..page_count {
            self.state.update(cx, |s, _| {
                if let Some(rendered) = s.render_page(i) {
                    page_images.push((i, rendered.image.clone(), rendered.width, rendered.height));
                }
            });
        }

        let mut pages_column = div()
            .id("pdf-pages")
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.));

        for (idx, image, w, h) in &page_images {
            pages_column = pages_column.child(
                div()
                    .id(ElementId::Name(format!("pdf-page-{}", idx).into()))
                    .w(px(*w as f32))
                    .h(px(*h as f32))
                    .bg(rgb(0xFFFFFF))
                    .shadow_md()
                    .child(
                        img(ImageSource::Image(image.clone()))
                            .w(px(*w as f32))
                            .h(px(*h as f32))
                            .object_fit(ObjectFit::Contain),
                    ),
            );
        }

        let neg_scroll = -scroll_offset;

        div()
            .id("pdf-view")
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("PdfView")
            .bg(rgb(0xE8E4DA))
            .overflow_hidden()
            .child(
                div()
                    .id("pdf-scroll-container")
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .py(px(16.))
                    .top(neg_scroll)
                    .child(pages_column),
            )
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _window, cx| {
                let key = e.keystroke.key.as_str();
                let ctrl = e.keystroke.modifiers.control;
                let scroll_amount = px(60.);

                this.state.update(cx, |state, cx| {
                    match key {
                        "j" | "down" => {
                            state.scroll_offset += scroll_amount;
                            cx.notify();
                        }
                        "k" | "up" => {
                            state.scroll_offset = (state.scroll_offset - scroll_amount).max(px(0.));
                            cx.notify();
                        }
                        "d" if ctrl => {
                            state.scroll_offset += px(400.);
                            cx.notify();
                        }
                        "u" if ctrl => {
                            state.scroll_offset = (state.scroll_offset - px(400.)).max(px(0.));
                            cx.notify();
                        }
                        "+" | "=" => {
                            state.zoom = (state.zoom + 0.1).min(3.0);
                            state.invalidate_cache();
                            cx.notify();
                        }
                        "-" => {
                            state.zoom = (state.zoom - 0.1).max(0.3);
                            state.invalidate_cache();
                            cx.notify();
                        }
                        "g" => {
                            state.scroll_offset = px(0.);
                            cx.notify();
                        }
                        _ => {}
                    }
                });
            }))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _window, cx| {
                this.state.update(cx, |state, cx| {
                    let delta = match e.delta {
                        ScrollDelta::Lines(lines) => lines.y * px(40.),
                        ScrollDelta::Pixels(pixels) => pixels.y,
                    };
                    state.scroll_offset = (state.scroll_offset - delta).max(px(0.));
                    cx.notify();
                });
            }))
    }
}
