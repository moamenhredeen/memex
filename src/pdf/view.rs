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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page_count = self.state.read(cx).page_count;
        let scroll_offset = self.state.read(cx).scroll_offset;
        let total_height = self.state.read(cx).total_height;
        let viewport_h: f32 = window.viewport_size().height.into();

        let (vis_first, vis_last) = self.state.read(cx).visible_range(viewport_h);

        // Render only visible pages and collect their image data
        let mut visible_pages: Vec<(usize, Arc<gpui::Image>, f32, f32)> = Vec::new();
        self.state.update(cx, |s, _| {
            for i in vis_first..vis_last {
                let (_, w, h) = s.page_layout(i);
                if let Some(rendered) = s.render_page(i) {
                    visible_pages.push((i, rendered.image.clone(), w, h));
                }
            }
            s.evict_distant_pages(vis_first, vis_last);
        });

        // Build the page column with spacers for off-screen pages
        let mut pages_column = div()
            .id("pdf-pages")
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.));

        // Top spacer covering all pages above visible range
        if vis_first > 0 {
            let top_spacer_height =
                self.state.read(cx).page_layout(vis_first).0 - super::PADDING_Y;
            pages_column =
                pages_column.child(div().id("pdf-spacer-top").h(px(top_spacer_height)));
        }

        // Visible pages with rendered images and click handlers for links
        let viewport_width: f32 = window.viewport_size().width.into();
        for (idx, image, w, h) in &visible_pages {
            let page_idx = *idx;
            let state = self.state.clone();
            let page_w = *w;
            let page_h = *h;
            let vw = viewport_width;

            pages_column = pages_column.child(
                div()
                    .id(ElementId::Name(format!("pdf-page-{}", idx).into()))
                    .w(px(page_w))
                    .h(px(page_h))
                    .bg(rgb(0xFFFFFF))
                    .shadow_md()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |e, _window, cx| {
                        state.update(cx, |s, cx| {
                            let (page_y_offset, page_width, _) = s.page_layout(page_idx);
                            let scroll: f32 = s.scroll_offset.into();
                            let page_x_start = (vw - page_width) / 2.0;

                            let click_x = f32::from(e.position.x) - page_x_start;
                            let click_y =
                                f32::from(e.position.y) + scroll - page_y_offset;

                            if click_x >= 0.0 && click_x <= page_width && click_y >= 0.0
                            {
                                if let Some(target) =
                                    s.hit_test_link(page_idx, click_x, click_y)
                                {
                                    s.goto_page(target);
                                    cx.notify();
                                }
                            }
                        });
                    })
                    .child(
                        img(ImageSource::Image(image.clone()))
                            .w(px(page_w))
                            .h(px(page_h))
                            .object_fit(ObjectFit::Contain),
                    ),
            );
        }

        // Bottom spacer covering all pages below visible range
        if vis_last < page_count && vis_last > 0 {
            let last_layout = self.state.read(cx).page_layout(vis_last - 1);
            let bottom_of_visible = last_layout.0 + last_layout.2 + 8.0;
            let bottom_spacer = total_height - bottom_of_visible;
            if bottom_spacer > 0.0 {
                pages_column = pages_column
                    .child(div().id("pdf-spacer-bottom").h(px(bottom_spacer)));
            }
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
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let ctrl = e.keystroke.modifiers.control;
                let scroll_amount = px(60.);
                let vh: f32 = window.viewport_size().height.into();

                this.state.update(cx, |state, cx| {
                    let max = state.max_scroll(vh);
                    match key {
                        "j" | "down" => {
                            state.scroll_offset =
                                (state.scroll_offset + scroll_amount).min(max);
                            cx.notify();
                        }
                        "k" | "up" => {
                            state.scroll_offset =
                                (state.scroll_offset - scroll_amount).max(px(0.));
                            cx.notify();
                        }
                        "d" if ctrl => {
                            state.scroll_offset =
                                (state.scroll_offset + px(400.)).min(max);
                            cx.notify();
                        }
                        "u" if ctrl => {
                            state.scroll_offset =
                                (state.scroll_offset - px(400.)).max(px(0.));
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
                        "G" => {
                            state.scroll_offset = max;
                            cx.notify();
                        }
                        _ => {}
                    }
                });
            }))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| {
                let vh: f32 = window.viewport_size().height.into();
                this.state.update(cx, |state, cx| {
                    let delta = match e.delta {
                        ScrollDelta::Lines(lines) => lines.y * px(40.),
                        ScrollDelta::Pixels(pixels) => pixels.y,
                    };
                    let max = state.max_scroll(vh);
                    state.scroll_offset =
                        (state.scroll_offset - delta).clamp(px(0.), max);
                    cx.notify();
                });
            }))
    }
}
