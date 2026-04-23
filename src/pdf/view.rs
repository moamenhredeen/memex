use std::sync::Arc;

use gpui::*;

use super::PdfState;
use crate::ui::Scrollbar;
use crate::keymap::{Action, KeyCombo, KeyTrie, Layer, build_pdf_layer};

/// Emitted by [`PdfView`] when a keybinding resolves to a command.
/// The app shell subscribes and runs the command through
/// `ActiveItem::execute_command`.
#[derive(Clone, Debug)]
pub enum PdfViewEvent {
    Command(&'static str),
}

impl EventEmitter<PdfViewEvent> for PdfView {}

pub struct PdfView {
    pub state: Entity<PdfState>,
    focus_handle: FocusHandle,
    /// PDF-local keymap layer. Only resolves when this view has focus.
    keymap: Layer,
    _observe_state: Subscription,
}

impl PdfView {
    pub fn new(state: Entity<PdfState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        let _observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            focus_handle,
            keymap: build_pdf_layer(),
            _observe_state,
        }
    }

    /// Resolve a keystroke against the PDF layer. Pure function — useful for
    /// tests. Returns the bound command id, if any.
    pub fn resolve_command(&self, key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<&'static str> {
        let combo = KeyCombo::from_keystroke(key, ctrl, shift, alt);
        match self.keymap.lookup(&combo)? {
            KeyTrie::Leaf(Action::Command(id)) => Some(*id),
            _ => None,
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
                .shadow_md()
                .relative();

            if let Some(image) = maybe_image {
                // Collect search highlights for this page
                let search_highlights: Vec<(bool, f32, f32, f32, f32)> = {
                    let st = self.state.read(cx);
                    let scale = st.page_scale(page_idx);
                    let current = st.search_current;
                    st.search_hits_for_page(page_idx)
                        .into_iter()
                        .map(|(global_idx, hit)| {
                            let q = &hit.quad;
                            // Quad corners: ul (upper-left), ur (upper-right),
                            // ll (lower-left), lr (lower-right) in PDF coords.
                            let x = q.ul.x.min(q.ll.x) * scale;
                            let y = q.ul.y.min(q.ur.y) * scale;
                            let x2 = q.ur.x.max(q.lr.x) * scale;
                            let y2 = q.ll.y.max(q.lr.y) * scale;
                            (global_idx == current, x, y, x2 - x, y2 - y)
                        })
                        .collect()
                };

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

                // Add search highlight overlays
                for (i, (is_current, x, y, w, h)) in search_highlights.iter().enumerate() {
                    let color = if *is_current {
                        rgba(0xFF8C0080) // orange for current match
                    } else {
                        rgba(0xFFFF0050) // yellow for other matches
                    };
                    page_div = page_div.child(
                        div()
                            .id(ElementId::Name(
                                format!("search-hl-{}-{}", page_idx, i).into(),
                            ))
                            .absolute()
                            .left(px(*x))
                            .top(px(*y))
                            .w(px(*w))
                            .h(px(*h))
                            .bg(color),
                    );
                }
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

        div()
            .id("pdf-view")
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .key_context("PdfView")
            .bg(rgb(0xE8E4DA))
            .overflow_hidden()
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _window, cx| {
                let k = &e.keystroke;
                if let Some(cmd) = this.resolve_command(
                    k.key.as_str(),
                    k.modifiers.control,
                    k.modifiers.shift,
                    k.modifiers.alt,
                ) {
                    cx.emit(PdfViewEvent::Command(cmd));
                    cx.stop_propagation();
                }
            }))
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
            // Custom scrollbar element — uses actual rendered bounds
            .child(Scrollbar::new(self.state.clone()).with_id("pdf-scrollbar"))
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
