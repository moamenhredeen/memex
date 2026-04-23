//! Generic vertical scrollbar element.
//!
//! Originally extracted from the PDF view. Parameterised over any state
//! entity that implements [`Scrollable`]. The editor and the PDF view both
//! use it; future scrollable views can implement the trait and drop it in.

use gpui::*;

const SCROLLBAR_WIDTH: Pixels = px(10.);
const THUMB_MIN_HEIGHT: Pixels = px(30.);
const THUMB_RADIUS: Pixels = px(3.);

/// Drag state stored on the scrollable entity so it survives across frames.
#[derive(Clone, Copy, Debug)]
pub struct DragState {
    pub start_mouse_y: f32,
    pub start_scroll: f32,
}

/// Anything that has a vertical scroll position and a known content height.
///
/// The scrollbar reads these to compute thumb geometry and writes back
/// `scroll_offset` / `drag_state` as the user interacts with it.
pub trait Scrollable: 'static {
    /// Total document height in pixels (content only, no padding chrome).
    fn total_height(&self) -> f32;
    fn scroll_offset(&self) -> Pixels;
    fn set_scroll_offset(&mut self, offset: Pixels);
    fn drag_state(&self) -> Option<DragState>;
    fn set_drag_state(&mut self, drag: Option<DragState>);
}

/// Prepaint state computed from actual element bounds.
#[doc(hidden)]
pub struct ScrollbarPrepaintState {
    track_bounds: Bounds<Pixels>,
    thumb_bounds: Bounds<Pixels>,
    track_hitbox: Hitbox,
    thumb_hitbox: Hitbox,
}

/// A custom gpui Element that renders a vertical scrollbar, generic over any
/// [`Scrollable`] entity.
pub struct Scrollbar<S: Scrollable> {
    state: Entity<S>,
    element_id: ElementId,
}

impl<S: Scrollable> Scrollbar<S> {
    pub fn new(state: Entity<S>) -> Self {
        Self {
            state,
            element_id: "scrollbar".into(),
        }
    }

    /// Override the element id. Useful when multiple scrollbars share a
    /// parent dispatch tree.
    pub fn with_id(mut self, id: impl Into<ElementId>) -> Self {
        self.element_id = id.into();
        self
    }
}

impl<S: Scrollable> IntoElement for Scrollbar<S> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<S: Scrollable> Element for Scrollbar<S> {
    type RequestLayoutState = ();
    type PrepaintState = Option<ScrollbarPrepaintState>;

    fn id(&self) -> Option<ElementId> {
        Some(self.element_id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Absolute position, fills parent (100% x 100%), pinned to top-left.
        let mut style = Style {
            position: Position::Absolute,
            size: size(relative(1.), relative(1.)).map(Into::into),
            ..Default::default()
        };
        style.inset.top = Length::Definite(px(0.).into());
        style.inset.left = Length::Definite(px(0.).into());
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let s = self.state.read(cx);
        let total_height = s.total_height();
        let scroll_f: f32 = s.scroll_offset().into();
        let view_height: f32 = bounds.size.height.into();

        if total_height <= view_height {
            return None;
        }

        // Track on the right edge of the element.
        let track_bounds = Bounds::new(
            point(
                bounds.origin.x + bounds.size.width - SCROLLBAR_WIDTH,
                bounds.origin.y,
            ),
            size(SCROLLBAR_WIDTH, bounds.size.height),
        );

        // Thumb proportional to viewport/total ratio.
        let thumb_ratio = (view_height / total_height).min(1.0);
        let thumb_height = (bounds.size.height * thumb_ratio).max(THUMB_MIN_HEIGHT);
        let usable = bounds.size.height - thumb_height;
        let scroll_ratio = scroll_f / (total_height - view_height);
        let thumb_top = usable * scroll_ratio;

        let thumb_bounds = Bounds::new(
            point(
                track_bounds.origin.x + px(1.),
                bounds.origin.y + thumb_top,
            ),
            size(SCROLLBAR_WIDTH - px(2.), thumb_height),
        );

        let track_hitbox = window.insert_hitbox(track_bounds, HitboxBehavior::BlockMouse);
        let thumb_hitbox = window.insert_hitbox(thumb_bounds, HitboxBehavior::BlockMouse);

        Some(ScrollbarPrepaintState {
            track_bounds,
            thumb_bounds,
            track_hitbox,
            thumb_hitbox,
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(prepaint) = prepaint.take() else {
            return;
        };

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(prepaint.track_bounds, hsla(0., 0., 0., 0.06)));

            let dragging = self.state.read(cx).drag_state().is_some();
            let thumb_color = if prepaint.thumb_hitbox.is_hovered(window) || dragging {
                hsla(0., 0., 0., 0.45)
            } else {
                hsla(0., 0., 0., 0.25)
            };
            window.paint_quad(quad(
                prepaint.thumb_bounds,
                Corners::all(THUMB_RADIUS),
                thumb_color,
                Edges::default(),
                transparent_black(),
                BorderStyle::default(),
            ));

            window.set_cursor_style(CursorStyle::Arrow, &prepaint.thumb_hitbox);

            // Mouse down on thumb — start drag.
            let state = self.state.clone();
            window.on_mouse_event({
                let thumb_hitbox = prepaint.thumb_hitbox.clone();
                let state = state.clone();

                move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !thumb_hitbox.is_hovered(window)
                    {
                        return;
                    }
                    let scroll_offset: f32 = state.read(cx).scroll_offset().into();
                    state.update(cx, |s, _| {
                        s.set_drag_state(Some(DragState {
                            start_mouse_y: f32::from(event.position.y),
                            start_scroll: scroll_offset,
                        }));
                    });
                    cx.stop_propagation();
                }
            });

            // Mouse down on track (not thumb) — jump to position.
            let view_h: f32 = bounds.size.height.into();
            window.on_mouse_event({
                let track_hitbox = prepaint.track_hitbox.clone();
                let thumb_hitbox = prepaint.thumb_hitbox.clone();
                let state = state.clone();
                let total_h = self.state.read(cx).total_height();
                let track_origin_y: f32 = prepaint.track_bounds.origin.y.into();
                let thumb_h: f32 = prepaint.thumb_bounds.size.height.into();

                move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !track_hitbox.is_hovered(window)
                        || thumb_hitbox.is_hovered(window)
                    {
                        return;
                    }
                    let click_y = f32::from(event.position.y) - track_origin_y;
                    let usable = (view_h - thumb_h).max(1.0);
                    let new_thumb_top = (click_y - thumb_h / 2.0).clamp(0.0, usable);
                    let ratio = new_thumb_top / usable;
                    let max_scroll = (total_h - view_h).max(0.0);
                    let new_scroll = ratio * max_scroll;

                    state.update(cx, |s, cx| {
                        s.set_scroll_offset(px(new_scroll));
                        s.set_drag_state(Some(DragState {
                            start_mouse_y: f32::from(event.position.y),
                            start_scroll: new_scroll,
                        }));
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
            });

            // Mouse move — drag thumb.
            window.on_mouse_event({
                let state = state.clone();
                let total_h = self.state.read(cx).total_height();
                let thumb_h: f32 = prepaint.thumb_bounds.size.height.into();

                move |event: &MouseMoveEvent, phase, _window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let drag = match state.read(cx).drag_state() {
                        Some(d) if event.dragging() => d,
                        _ => return,
                    };
                    let mouse_y: f32 = event.position.y.into();
                    let delta_y = mouse_y - drag.start_mouse_y;
                    let usable = (view_h - thumb_h).max(1.0);
                    let max_scroll = (total_h - view_h).max(0.0);
                    let scroll_per_px = max_scroll / usable;
                    let new_scroll =
                        (drag.start_scroll + delta_y * scroll_per_px).clamp(0.0, max_scroll);
                    state.update(cx, |s, cx| {
                        s.set_scroll_offset(px(new_scroll));
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
            });

            // Mouse up — stop drag.
            window.on_mouse_event({
                let state = state.clone();

                move |event: &MouseUpEvent, phase, _window, cx| {
                    if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                        return;
                    }
                    if state.read(cx).drag_state().is_some() {
                        state.update(cx, |s, cx| {
                            s.set_drag_state(None);
                            cx.notify();
                        });
                    }
                }
            });
        });
    }
}

