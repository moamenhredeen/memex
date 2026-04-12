use gpui::*;

use crate::markdown::{LineKind, StyleKind};

use super::{EditorState, LinePaintInfo};

pub struct EditorElement {
    state: Entity<EditorState>,
}

impl EditorElement {
    pub fn new(state: &Entity<EditorState>) -> Self {
        Self {
            state: state.clone(),
        }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct PrepaintState {
    lines: Vec<PrepaintLine>,
    cursor: Option<PaintQuad>,
    selection_quads: Vec<PaintQuad>,
}

struct PrepaintLine {
    shaped: ShapedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    content_offset: usize,
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
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
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
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
        // Update display map if content changed (needs mutable access)
        self.state.update(cx, |state, _cx| {
            let content = state.content();
            state.display_map.update(&content);
        });

        let state = self.state.read(cx);
        let padding = px(24.);
        let scroll = state.scroll_offset;
        let cursor_pos = state.cursor;
        let selected_range = state.selected_range.clone();
        let content = state.content();
        let dm = &state.display_map;
        let vim_enabled = state.vim.enabled;
        let editor_mode = state.mode;

        let text_system = window.text_system().clone();
        let text_style = window.text_style();
        let font_family: SharedString = text_style.font_family.clone();

        // Virtual scrolling: only shape lines in viewport + overscan
        let overscan = 20;
        let (vis_first, vis_last) =
            dm.visible_range(scroll, bounds.size.height, overscan);

        let mut prepaint_lines = Vec::new();
        let mut cursor_quad: Option<PaintQuad> = None;
        let mut selection_quads = Vec::new();

        // Solarized Light palette (hsla for text shaping API)
        let base_color = hsla(0.544, 0.129, 0.455, 1.0);  // base00 — body text
        let dim_color = hsla(0.500, 0.069, 0.604, 1.0);   // base1 — comments
        let code_color = hsla(0.487, 0.586, 0.398, 1.0);  // cyan — inline code
        let heading_color = hsla(0.534, 0.808, 0.143, 1.0); // base03 — headings
        let hr_color = hsla(0.117, 0.235, 0.775, 1.0);    // subtle rule

        for i in vis_first..vis_last {
            let info = dm.line_info(i);
            let line_start = dm.line_offset(i);
            let line_height = dm.line_height(i);
            let font_size = info.kind.display_font_size();
            let y = bounds.origin.y + padding - scroll + dm.line_y(i);

            let line_text_end = content[line_start..]
                .find('\n')
                .map(|p| line_start + p)
                .unwrap_or(content.len());
            let line_byte_end = line_text_end;
            let line_text = &content[line_start..line_text_end];

            let display_text: SharedString = if line_text.is_empty() {
                " ".into()
            } else {
                SharedString::from(line_text.to_string())
            };

            let mut runs: Vec<TextRun> = Vec::new();

            if line_text.is_empty() {
                runs.push(TextRun {
                    len: 1,
                    font: Font {
                        family: font_family.clone(),
                        weight: FontWeight::NORMAL,
                        style: FontStyle::Normal,
                        features: FontFeatures::default(),
                        fallbacks: None,
                    },
                    color: base_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
            } else {
                for span in &info.spans {
                    let span_start = span.range.start.min(line_text.len());
                    let span_end = span.range.end.min(line_text.len());
                    let span_len = span_end - span_start;
                    if span_len == 0 {
                        continue;
                    }

                    let (weight, fstyle, color, bg, underline, strike, use_mono) =
                        match &span.kind {
                            StyleKind::Normal => (
                                info.kind.display_font_weight(),
                                FontStyle::Normal,
                                if matches!(info.kind, LineKind::Heading(_)) {
                                    heading_color
                                } else {
                                    base_color
                                },
                                None,
                                None,
                                None,
                                false,
                            ),
                            StyleKind::Bold => (
                                FontWeight::BOLD,
                                FontStyle::Normal,
                                base_color,
                                None,
                                None,
                                None,
                                false,
                            ),
                            StyleKind::Italic => (
                                info.kind.display_font_weight(),
                                FontStyle::Italic,
                                base_color,
                                None,
                                None,
                                None,
                                false,
                            ),
                            StyleKind::BoldItalic => (
                                FontWeight::BOLD,
                                FontStyle::Italic,
                                base_color,
                                None,
                                None,
                                None,
                                false,
                            ),
                            StyleKind::Code => (
                                FontWeight::NORMAL,
                                FontStyle::Normal,
                                code_color,
                                Some(hsla(0.127, 0.424, 0.884, 1.0)),  // base2 — code bg
                                None,
                                None,
                                true,
                            ),
                            StyleKind::Strikethrough => (
                                info.kind.display_font_weight(),
                                FontStyle::Normal,
                                dim_color,
                                None,
                                None,
                                Some(StrikethroughStyle {
                                    thickness: px(1.),
                                    color: Some(dim_color),
                                }),
                                false,
                            ),
                            StyleKind::HeadingSyntax => (
                                FontWeight::BOLD,
                                FontStyle::Normal,
                                dim_color,
                                None,
                                None,
                                None,
                                false,
                            ),
                            StyleKind::CodeFence => (
                                FontWeight::NORMAL,
                                FontStyle::Normal,
                                dim_color,
                                None,
                                None,
                                None,
                                true,
                            ),
                            StyleKind::HrSyntax => (
                                FontWeight::NORMAL,
                                FontStyle::Normal,
                                hr_color,
                                None,
                                None,
                                None,
                                false,
                            ),
                            StyleKind::ListBullet => (
                                FontWeight::BOLD,
                                FontStyle::Normal,
                                dim_color,
                                None,
                                None,
                                None,
                                false,
                            ),
                            StyleKind::TableSyntax => (
                                FontWeight::NORMAL,
                                FontStyle::Normal,
                                dim_color,
                                None,
                                None,
                                None,
                                true,
                            ),
                            StyleKind::BlockQuoteSyntax => (
                                FontWeight::NORMAL,
                                FontStyle::Normal,
                                dim_color,
                                None,
                                None,
                                None,
                                false,
                            ),
                            StyleKind::Wikilink => (
                                FontWeight::NORMAL,
                                FontStyle::Normal,
                                hsla(0.569, 0.694, 0.486, 1.0),  // blue — links
                                None,
                                Some(UnderlineStyle {
                                    thickness: px(1.),
                                    color: Some(hsla(0.569, 0.694, 0.486, 1.0)),
                                    wavy: false,
                                }),
                                None,
                                false,
                            ),
                        };

                    let family = if use_mono || matches!(info.kind, LineKind::TableRow) {
                        SharedString::from("FiraCode Nerd Font Mono")
                    } else {
                        font_family.clone()
                    };

                    runs.push(TextRun {
                        len: span_len,
                        font: Font {
                            family,
                            weight,
                            style: fstyle,
                            features: FontFeatures::default(),
                            fallbacks: None,
                        },
                        color,
                        background_color: bg,
                        underline,
                        strikethrough: strike,
                    });
                }
            }

            // Validate total run length matches display text
            let total_run_len: usize = runs.iter().map(|r| r.len).sum();
            if total_run_len != display_text.len() {
                runs = vec![TextRun {
                    len: display_text.len(),
                    font: Font {
                        family: font_family.clone(),
                        weight: info.kind.display_font_weight(),
                        style: FontStyle::Normal,
                        features: FontFeatures::default(),
                        fallbacks: None,
                    },
                    color: base_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }];
            }

            let shaped = text_system.shape_line(display_text, font_size, &runs, None);

            let origin = point(bounds.origin.x + padding, y);

            // Cursor — render on the line where the cursor logically belongs
            if cursor_pos >= line_start && cursor_pos <= line_byte_end {
                let idx_in_line = cursor_pos - line_start;
                let cursor_x = shaped.x_for_index(idx_in_line);

                // Determine cursor shape based on vim mode
                let cursor_color = rgb(0x268BD2); // solarized blue
                use super::keymap::EditorMode;
                let cursor_shape = if vim_enabled {
                    match editor_mode {
                        EditorMode::Normal | EditorMode::Visual | EditorMode::VisualLine => {
                            // Block cursor: width of one character (or fallback)
                            let next_x = if idx_in_line < line_text.len() {
                                shaped.x_for_index(idx_in_line + 1)
                            } else {
                                cursor_x + px(8.) // fallback width at end of line
                            };
                            let block_w = (next_x - cursor_x).max(px(8.));
                            size(block_w, line_height)
                        }
                        EditorMode::Command => {
                            // Underline cursor
                            size(px(8.), px(2.))
                        }
                        EditorMode::Insert => {
                            size(px(2.), line_height)
                        }
                    }
                } else {
                    size(px(2.), line_height)
                };

                let cursor_y = if vim_enabled && editor_mode == EditorMode::Command {
                    y + line_height - px(2.) // underline at bottom
                } else {
                    y
                };

                cursor_quad = Some(fill(
                    Bounds::new(point(origin.x + cursor_x, cursor_y), cursor_shape),
                    cursor_color,
                ));
            }

            // Selection highlighting
            if !selected_range.is_empty() {
                let sel_start = selected_range.start.max(line_start);
                let sel_end = selected_range.end.min(line_byte_end);
                if sel_start < sel_end {
                    let x1 = shaped.x_for_index(sel_start - line_start);
                    let x2 = shaped.x_for_index(sel_end - line_start);
                    selection_quads.push(fill(
                        Bounds::from_corners(
                            point(origin.x + x1, y),
                            point(origin.x + x2, y + line_height),
                        ),
                        rgba(0x268BD230), // solarized blue selection
                    ));
                }
            }

            prepaint_lines.push(PrepaintLine {
                shaped,
                origin,
                line_height,
                content_offset: line_start,
            });
        }

        // Default cursor if none set (e.g. cursor at very end of document beyond visible lines)
        if cursor_quad.is_none() {
            let cw = if vim_enabled && !matches!(editor_mode, super::keymap::EditorMode::Insert) {
                px(8.)
            } else {
                px(2.)
            };
            cursor_quad = Some(fill(
                Bounds::new(
                    point(
                        bounds.origin.x + padding,
                        bounds.origin.y + padding - scroll,
                    ),
                    size(cw, px(24.)),
                ),
                rgb(0x268BD2), // solarized blue cursor fallback
            ));
        }

        PrepaintState {
            lines: prepaint_lines,
            cursor: cursor_quad,
            selection_quads,
        }
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
        let focus_handle = self.state.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );

        // Background
        window.paint_quad(fill(bounds, rgb(0xFDF6E3))); // solarized base3 bg

        // Selections
        for sel in &prepaint.selection_quads {
            window.paint_quad(sel.clone());
        }

        // Lines
        let mut line_paint_infos = Vec::new();
        for pl in &prepaint.lines {
            if pl.origin.y + pl.line_height < bounds.origin.y
                || pl.origin.y > bounds.origin.y + bounds.size.height
            {
                continue;
            }
            let _ = pl.shaped.paint(pl.origin, pl.line_height, window, cx);

            line_paint_infos.push(LinePaintInfo {
                content_offset: pl.content_offset,
                shaped_line: pl.shaped.clone(),
                y: pl.origin.y,
                line_height: pl.line_height,
            });
        }

        // Cursor
        let is_focused = focus_handle.is_focused(window);
        let blink_visible = self.state.read(cx).blink_cursor.read(cx).visible();

        if is_focused && blink_visible {
            if let Some(ref cursor) = prepaint.cursor {
                window.paint_quad(cursor.clone());
            }
        }

        // Store layout info for mouse positioning
        self.state.update(cx, |state, _cx| {
            state.last_line_layouts = line_paint_infos;
            state.last_bounds = Some(bounds);
        });
    }
}
