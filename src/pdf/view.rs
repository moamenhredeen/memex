use std::sync::Arc;

use gpui::*;

use super::PdfState;

pub struct PdfView {
    pub state: Entity<PdfState>,
    focus_handle: FocusHandle,
    _observe_state: Subscription,
    /// Whether the user is dragging the scrollbar thumb
    dragging_scrollbar: bool,
    /// Mouse Y at drag start (window coords)
    drag_start_mouse_y: f32,
    /// Scroll offset at drag start
    drag_start_scroll: f32,
}

impl PdfView {
    pub fn new(state: Entity<PdfState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        let _observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            focus_handle,
            _observe_state,
            dragging_scrollbar: false,
            drag_start_mouse_y: 0.0,
            drag_start_scroll: 0.0,
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

        // Collect cached page images and track which pages need rendering
        let mut visible_pages: Vec<(usize, Option<Arc<gpui::Image>>, f32, f32)> = Vec::new();
        let mut needs_render: Vec<usize> = Vec::new();

        {
            let state = self.state.read(cx);
            for i in vis_first..vis_last {
                let (_, w, h) = state.page_layout(i);
                if let Some(rendered) = state.get_cached_page(i) {
                    visible_pages.push((i, Some(rendered.image.clone()), w, h));
                } else {
                    visible_pages.push((i, None, w, h));
                    if !state.is_pending(i) {
                        needs_render.push(i);
                    }
                }
            }
        }

        // Kick off background renders for uncached pages
        if !needs_render.is_empty() {
            self.state.update(cx, |s, cx| {
                s.request_render_pages(&needs_render, cx);
            });
        }

        // Evict distant pages
        self.state.update(cx, |s, _| {
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

        // Visible pages: rendered images or loading placeholders
        let viewport_width: f32 = window.viewport_size().width.into();
        for (idx, maybe_image, w, h) in &visible_pages {
            let page_idx = *idx;
            let state = self.state.clone();
            let page_w = *w;
            let page_h = *h;
            let vw = viewport_width;

            let mut page_div = div()
                .id(ElementId::Name(format!("pdf-page-{}", idx).into()))
                .w(px(page_w))
                .h(px(page_h))
                .bg(rgb(0xFFFFFF))
                .shadow_md();

            if let Some(image) = maybe_image {
                // Rendered page with click handler for links
                page_div = page_div
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
                    );
            } else {
                // Loading placeholder
                page_div = page_div.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(0x999999))
                        .child(format!("Loading page {}...", page_idx + 1)),
                );
            }

            pages_column = pages_column.child(page_div);
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
        let scroll_f: f32 = scroll_offset.into();
        let scrollbar_width = 10.0;
        let min_thumb_height = 30.0;
        let track_height = viewport_h;
        let thumb_ratio = if total_height > 0.0 {
            (viewport_h / total_height).min(1.0)
        } else {
            1.0
        };
        let scroll_ratio = if total_height > viewport_h {
            scroll_f / (total_height - viewport_h)
        } else {
            0.0
        };
        let thumb_height = (track_height * thumb_ratio).max(min_thumb_height);
        let thumb_top = scroll_ratio * (track_height - thumb_height);
        let show_scrollbar = total_height > viewport_h;

        let total_h = total_height;
        let vh_copy = viewport_h;
        let thumb_h = thumb_height;
        let thumb_t = thumb_top;

        let mut root = div()
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
            );

        // Scrollbar track on the right edge
        if show_scrollbar {
            root = root.child(
                div()
                    .id("pdf-scrollbar-track")
                    .absolute()
                    .right(px(0.))
                    .top(px(0.))
                    .w(px(scrollbar_width))
                    .h(px(track_height))
                    .bg(rgba(0x00000015))
                    .on_mouse_down(MouseButton::Left, cx.listener(
                        move |this, e: &MouseDownEvent, _window, cx| {
                            // Skip if thumb already handled this click
                            if this.dragging_scrollbar {
                                return;
                            }
                            // Click on track (not thumb): center thumb at click position
                            let click_y: f32 = e.position.y.into();
                            let usable = (vh_copy - thumb_h).max(1.0);
                            let new_thumb_top = (click_y - thumb_h / 2.0).clamp(0.0, usable);
                            let ratio = new_thumb_top / usable;
                            let max_scroll = (total_h - vh_copy).max(0.0);
                            let new_scroll = ratio * max_scroll;
                            this.state.update(cx, |s, cx| {
                                s.scroll_offset = px(new_scroll);
                                cx.notify();
                            });
                            // Start dragging from this position
                            this.dragging_scrollbar = true;
                            this.drag_start_mouse_y = click_y;
                            this.drag_start_scroll = new_scroll;
                        },
                    ))
                    .child(
                        div()
                            .id("pdf-scrollbar-thumb")
                            .absolute()
                            .top(px(thumb_top))
                            .left(px(1.0))
                            .w(px(scrollbar_width - 2.0))
                            .h(px(thumb_height))
                            .bg(rgba(0x00000055))
                            .rounded(px(3.0))
                            .hover(|s| s.bg(rgba(0x00000088)))
                            .on_mouse_down(MouseButton::Left, cx.listener(
                                move |this, e: &MouseDownEvent, _window, cx| {
                                    this.dragging_scrollbar = true;
                                    this.drag_start_mouse_y = f32::from(e.position.y);
                                    this.drag_start_scroll =
                                        f32::from(this.state.read(cx).scroll_offset);
                                },
                            )),
                    ),
            );
        }

        // Mouse move handler on root for scrollbar dragging.
        // Uses delta from drag start to compute new scroll, so thumb tracks mouse exactly.
        root = root.on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _window, cx| {
            if this.dragging_scrollbar && e.pressed_button == Some(MouseButton::Left) {
                let mouse_y: f32 = e.position.y.into();
                let delta_y = mouse_y - this.drag_start_mouse_y;
                // Convert pixel delta to scroll delta:
                // 1 pixel of thumb movement = (max_scroll / usable_track) scroll pixels
                let usable = (vh_copy - thumb_h).max(1.0);
                let max_scroll = (total_h - vh_copy).max(0.0);
                let scroll_per_px = max_scroll / usable;
                let new_scroll = (this.drag_start_scroll + delta_y * scroll_per_px)
                    .clamp(0.0, max_scroll);
                this.state.update(cx, |s, cx| {
                    s.scroll_offset = px(new_scroll);
                    cx.notify();
                });
            }
        }));

        // Mouse up handler to stop dragging
        root = root.on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _e: &MouseUpEvent, _window, _cx| {
                this.dragging_scrollbar = false;
            }),
        );

        root
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
