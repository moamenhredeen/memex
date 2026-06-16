use std::sync::Arc;

use gpui::*;

use super::PdfState;
use crate::keymap::{Action, KeyContext, KeymapSystem, ResolvedKey};
use crate::theme::Theme;
use crate::ui::Scrollbar;

fn top_spacer_height(page_y_offset: f32) -> f32 {
    (page_y_offset - super::PADDING_Y - super::PAGE_GAP).max(0.0)
}

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
    keymap: KeymapSystem,
    theme: Theme,
    _observe_state: Subscription,
}

impl PdfView {
    pub fn new(state: Entity<PdfState>, theme: Theme, cx: &mut Context<Self>) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        let _observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            focus_handle,
            keymap: KeymapSystem::new(false),
            theme,
            _observe_state,
        }
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    /// Resolve a keystroke against the PDF key context.
    pub fn resolve_command(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> Option<&'static str> {
        let mut context = KeyContext::new();
        context.add("Pdf");
        match self.keymap.resolve_key(key, ctrl, shift, alt, &context) {
            ResolvedKey::Action(Action::Command(id), _) => Some(id),
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
        let mut pages_column = div().w_full().flex().flex_col().items_center().gap(px(8.));
        let mut child_pages = Vec::new();

        // Top spacer covering all pages above visible range
        if vis_first > 0 {
            // The flex column inserts PAGE_GAP after this spacer. Exclude that
            // gap here so the rendered page top still matches PageLayout::y_offset.
            let top_spacer_height = top_spacer_height(self.state.read(cx).page_layout(vis_first).0);
            pages_column = pages_column.child(div().id("pdf-spacer-top").h(px(top_spacer_height)));
            child_pages.push(None);
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
                // Collect transient overlays for this page.
                let (search_highlights, selection_highlights, annotation_highlights) = {
                    let st = self.state.read(cx);
                    let scale = st.page_scale(page_idx);
                    let current = st.search_current;
                    let search: Vec<(bool, f32, f32, f32, f32)> = st
                        .search_hits_for_page(page_idx)
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
                        .collect();
                    let selection = st.selection_quads_for_page(page_idx);
                    let annotation = st.selected_annotation_quads_for_page(page_idx);
                    (search, selection, annotation)
                };

                // Start a text selection. A click without a drag selects an
                // existing annotation or follows an internal link on mouse-up.
                page_div = page_div
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, move |e, _window, cx| {
                        state.update(cx, |s, cx| {
                            if let Some(point) = s.screen_to_page_point(page_idx, vw, e.position) {
                                s.begin_text_selection(page_idx, point);
                                cx.notify();
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
                for (kind, quads, color) in [
                    ("selection", selection_highlights, rgba(0x4A90E260)),
                    ("annotation", annotation_highlights, rgba(0xFF8C0080)),
                ] {
                    let scale = self.state.read(cx).page_scale(page_idx);
                    for (i, quad) in quads.iter().enumerate() {
                        let rect = mupdf::Rect::from(quad.clone());
                        page_div = page_div.child(
                            div()
                                .id(ElementId::Name(
                                    format!("{}-{}-{}", kind, page_idx, i).into(),
                                ))
                                .absolute()
                                .left(px(rect.x0 * scale))
                                .top(px(rect.y0 * scale))
                                .w(px(rect.width() * scale))
                                .h(px(rect.height() * scale))
                                .bg(color),
                        );
                    }
                }
            } else {
                // Loading placeholder
                page_div = page_div.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(self.theme.text_muted))
                        .child(format!("Loading page {}...", page_idx + 1)),
                );
            }

            pages_column = pages_column.child(page_div);
            child_pages.push(Some(page_idx));
        }

        // Bottom spacer covering all pages below visible range
        if vis_last < page_count && vis_last > 0 {
            let last_layout = self.state.read(cx).page_layout(vis_last - 1);
            let bottom_of_visible = last_layout.0 + last_layout.2 + 8.0;
            let bottom_spacer = total_height - bottom_of_visible;
            if bottom_spacer > 0.0 {
                pages_column =
                    pages_column.child(div().id("pdf-spacer-bottom").h(px(bottom_spacer)));
                child_pages.push(None);
            }
        }

        let bounds_state = self.state.clone();
        pages_column = pages_column.on_children_prepainted(move |bounds, _window, cx| {
            let measured = child_pages
                .iter()
                .copied()
                .zip(bounds)
                .filter_map(|(page, bounds)| page.map(|page| (page, bounds)));
            bounds_state.update(cx, |state, _| state.set_rendered_page_bounds(measured));
        });

        let neg_scroll = -scroll_offset;

        div()
            .id("pdf-view")
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .key_context("PdfView")
            .bg(rgb(self.theme.pdf_background))
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
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                if e.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let vw: f32 = window.viewport_size().width.into();
                this.state.update(cx, |state, cx| {
                    if let Some((page, point)) = state.screen_to_document_point(vw, e.position) {
                        if let Err(error) = state.update_text_selection(page, point) {
                            eprintln!("PDF selection preview failed: {}", error);
                        }
                        cx.notify();
                    }
                });
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, e: &MouseUpEvent, window, cx| {
                    let vw: f32 = window.viewport_size().width.into();
                    this.state.update(cx, |state, cx| {
                        let point = state.screen_to_document_point(vw, e.position);
                        match state.finish_text_selection() {
                            Ok(true) => cx.notify(),
                            Ok(false) => {
                                if let Some((page, point)) = point {
                                    if let Some(target) = state.hit_test_link_point(page, point) {
                                        state.goto_page(target);
                                    }
                                }
                                cx.notify();
                            }
                            Err(error) => eprintln!("PDF selection failed: {}", error),
                        }
                    });
                }),
            )
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
                    state.scroll_offset = (state.scroll_offset - delta).clamp(px(0.), max);
                    cx.notify();
                });
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::top_spacer_height;

    #[test]
    fn virtual_spacer_does_not_double_count_flex_gap() {
        assert_eq!(top_spacer_height(524.0), 500.0);
    }
}
