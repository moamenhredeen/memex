use gpui::*;
use std::collections::HashMap;

use crate::markdown::{LineKind, StyleKind, StyleSpan};
use crate::theme::Theme;

use super::{DIAGRAM_EMBED_HEIGHT_PX, EditorState, LinePaintInfo};

pub struct EditorElement {
    state: Entity<EditorState>,
    theme: Theme,
    editor_width: Pixels,
}

impl EditorElement {
    pub fn new(state: &Entity<EditorState>, theme: Theme, editor_width: Pixels) -> Self {
        Self {
            state: state.clone(),
            theme,
            editor_width,
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
    shaped: WrappedLine,
    origin: Point<Pixels>,
    row_height: Pixels,
    line_height: Pixels,
    is_diagram_embed: bool,
    content_offset: usize,
    source_len: usize,
    source_to_display: Vec<usize>,
    display_to_source: Vec<usize>,
}

const MIN_HORIZONTAL_PADDING: f32 = 24.0;

fn content_inset(viewport_width: Pixels, editor_width: Pixels) -> Pixels {
    let viewport_width: f32 = viewport_width.into();
    let editor_width: f32 = editor_width.into();
    px(((viewport_width - editor_width) / 2.0).max(MIN_HORIZONTAL_PADDING))
}

fn content_width(viewport_width: Pixels, editor_width: Pixels) -> Pixels {
    viewport_width - content_inset(viewport_width, editor_width) * 2.0
}

struct DisplayLine {
    text: String,
    spans: Vec<StyleSpan>,
    source_to_display: Vec<usize>,
    display_to_source: Vec<usize>,
}

fn wikilink_label(raw: &str, titles: &HashMap<String, String>) -> Option<String> {
    let inner = raw.strip_prefix("[[")?.strip_suffix("]]")?;
    let (target, alias) = inner
        .split_once('|')
        .map(|(target, alias)| (target.trim(), Some(alias.trim())))
        .unwrap_or_else(|| (inner.trim(), None));
    if let Some(alias) = alias.filter(|alias| !alias.is_empty()) {
        return Some(alias.to_string());
    }
    Some(
        titles
            .get(&target.to_lowercase())
            .cloned()
            .unwrap_or_else(|| target.to_string()),
    )
}

fn build_display_line(
    source: &str,
    spans: &[StyleSpan],
    titles: &HashMap<String, String>,
    cursor_offset: Option<usize>,
) -> DisplayLine {
    let mut text = String::new();
    let mut display_spans = Vec::with_capacity(spans.len());
    let mut source_to_display = vec![0; source.len() + 1];
    let mut display_to_source = vec![0];

    for span in spans {
        let source_start = span.range.start.min(source.len());
        let source_end = span.range.end.min(source.len());
        let raw = &source[source_start..source_end];
        let cursor_is_in_span =
            cursor_offset.is_some_and(|offset| offset >= source_start && offset < source_end);
        let replacement = if span.kind == StyleKind::Wikilink && !cursor_is_in_span {
            wikilink_label(raw, titles).unwrap_or_else(|| raw.to_string())
        } else {
            raw.to_string()
        };
        let display_start = text.len();
        text.push_str(&replacement);
        let display_end = text.len();

        if span.kind == StyleKind::Wikilink && replacement != raw {
            for offset in source_start..source_end {
                source_to_display[offset] = display_start;
            }
            source_to_display[source_end] = display_end;
            display_to_source.resize(display_end + 1, source_start);
            display_to_source[display_end] = source_end;
        } else {
            for offset in source_start..=source_end {
                source_to_display[offset] = display_start + offset - source_start;
            }
            for offset in display_start + 1..=display_end {
                display_to_source.push(source_start + offset - display_start);
            }
        }

        display_spans.push(StyleSpan {
            range: display_start..display_end,
            kind: span.kind.clone(),
        });
    }

    DisplayLine {
        text,
        spans: display_spans,
        source_to_display,
        display_to_source,
    }
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
        let horizontal_inset = content_inset(bounds.size.width, self.editor_width);
        let wrap_width = content_width(bounds.size.width, self.editor_width);

        // Update display map if content changed (needs mutable access)
        self.state.update(cx, |state, _cx| {
            let content = state.content();
            state.display_map.update(&content);
            if state.wrap_width != wrap_width {
                state.wrap_width = wrap_width;
                state.display_map.reset_line_heights();
            }
            // Reapply outline fold visibility after display map refresh
            let kinds = state.display_map.line_kinds();
            let headings = crate::editor::outline::extract_headings(&kinds);
            let line_count = state.display_map.line_count();
            let hidden = state.outline.compute_hidden_lines(&headings, line_count);
            state.display_map.update_visibility(&hidden);

            // Cache viewport height so follow-cursor scrolling + scroll-wheel
            // clamping have the true available height to work with.
            state.viewport_height = bounds.size.height;

            // Follow cursor: if a cursor-moving action has set the flag, nudge
            // scroll so the cursor is on-screen. Do nothing if the user just
            // scroll-wheeled — that doesn't set the flag.
            if state.needs_scroll_to_cursor {
                state.scroll_cursor_into_view();
                state.needs_scroll_to_cursor = false;
            }
        });

        let state = self.state.read(cx);
        let vertical_padding = px(24.);
        let scroll = state.scroll_offset;
        let cursor_pos = state.cursor;
        let selected_range = state.selected_range.clone();
        let content = state.content();
        let dm = &state.display_map;
        let vim_enabled = state.vim_enabled;
        let is_insert = state.insert_mode;

        let text_system = window.text_system().clone();
        let text_style = window.text_style();
        let font_family: SharedString = text_style.font_family.clone();

        // Virtual scrolling: only shape lines in viewport + overscan
        let overscan = 20;
        let (vis_first, vis_last) = dm.visible_range(scroll, bounds.size.height, overscan);

        let mut prepaint_lines = Vec::new();
        let mut cursor_quad: Option<PaintQuad> = None;
        let mut selection_quads = Vec::new();
        let base_color: Hsla = rgb(self.theme.text).into();
        let dim_color: Hsla = rgb(self.theme.text_muted).into();
        let code_color: Hsla = rgb(self.theme.cyan).into();
        let heading_color: Hsla = rgb(self.theme.text_strong).into();
        let hr_color: Hsla = rgb(self.theme.border).into();
        let mut height_updates = Vec::new();
        let mut accumulated_height_delta = px(0.);

        for i in vis_first..vis_last {
            // Skip lines hidden by outline folding
            if dm.is_line_hidden(i) {
                continue;
            }

            let info = dm.line_info(i);
            let line_start = dm.line_offset(i);
            let cached_line_height = dm.line_height(i);
            let row_height = info.kind.line_height();
            let font_size = info.kind.display_font_size();
            let y = bounds.origin.y + vertical_padding - scroll
                + dm.line_y(i)
                + accumulated_height_delta;

            // Check if this heading is folded (next line hidden)
            let heading_is_folded = matches!(&info.kind, LineKind::Heading(_))
                && i + 1 < dm.line_count()
                && dm.is_line_hidden(i + 1);

            let line_text_end = content[line_start..]
                .find('\n')
                .map(|p| line_start + p)
                .unwrap_or(content.len());
            let line_byte_end = line_text_end;
            let line_text = &content[line_start..line_text_end];

            let ellipsis_suffix = "...";
            let cursor_offset = (cursor_pos >= line_start && cursor_pos <= line_byte_end)
                .then_some(cursor_pos - line_start);
            let is_diagram_embed =
                cursor_offset.is_none() && state.diagram_embed_for_line(line_text).is_some();
            let mut display_line = build_display_line(
                line_text,
                &info.spans,
                &state.wikilink_titles,
                cursor_offset,
            );
            if heading_is_folded && !line_text.is_empty() {
                display_line.text.push_str(ellipsis_suffix);
                display_line
                    .display_to_source
                    .resize(display_line.text.len() + 1, line_text.len());
            }
            let display_text: SharedString = if line_text.is_empty() {
                display_line.display_to_source.push(0);
                " ".into()
            } else {
                SharedString::from(display_line.text.clone())
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
                for span in &display_line.spans {
                    let span_start = span.range.start.min(display_line.text.len());
                    let span_end = span.range.end.min(display_line.text.len());
                    let span_len = span_end - span_start;
                    if span_len == 0 {
                        continue;
                    }

                    let (weight, fstyle, color, bg, underline, strike, use_mono) = match &span.kind
                    {
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
                            Some(rgb(self.theme.code_background).into()),
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
                            rgb(self.theme.accent).into(),
                            None,
                            Some(UnderlineStyle {
                                thickness: px(1.),
                                color: Some(rgb(self.theme.accent).into()),
                                wavy: false,
                            }),
                            None,
                            false,
                        ),
                        StyleKind::Frontmatter => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            dim_color, // base1 — reads as metadata, not body
                            None,
                            None,
                            None,
                            false,
                        ),
                        StyleKind::Tag => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            rgb(self.theme.warning).into(),
                            None,
                            None,
                            None,
                            false,
                        ),
                        StyleKind::SyntaxComment => (
                            FontWeight::NORMAL,
                            FontStyle::Italic,
                            dim_color,
                            Some(rgb(self.theme.code_background).into()),
                            None,
                            None,
                            true,
                        ),
                        StyleKind::SyntaxString => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            rgb(self.theme.success).into(),
                            Some(rgb(self.theme.code_background).into()),
                            None,
                            None,
                            true,
                        ),
                        StyleKind::SyntaxNumber | StyleKind::SyntaxProperty => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            rgb(self.theme.warning).into(),
                            Some(rgb(self.theme.code_background).into()),
                            None,
                            None,
                            true,
                        ),
                        StyleKind::SyntaxKeyword | StyleKind::SyntaxConstant => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            rgb(self.theme.violet).into(),
                            Some(rgb(self.theme.code_background).into()),
                            None,
                            None,
                            true,
                        ),
                        StyleKind::SyntaxType => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            rgb(self.theme.cyan).into(),
                            Some(rgb(self.theme.code_background).into()),
                            None,
                            None,
                            true,
                        ),
                        StyleKind::SyntaxFunction => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            rgb(self.theme.accent).into(),
                            Some(rgb(self.theme.code_background).into()),
                            None,
                            None,
                            true,
                        ),
                        StyleKind::SyntaxOperator => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            code_color,
                            Some(rgb(self.theme.code_background).into()),
                            None,
                            None,
                            true,
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

            // Add ellipsis run for folded headings
            if heading_is_folded && !line_text.is_empty() {
                runs.push(TextRun {
                    len: ellipsis_suffix.len(),
                    font: Font {
                        family: font_family.clone(),
                        weight: FontWeight::NORMAL,
                        style: FontStyle::Normal,
                        features: FontFeatures::default(),
                        fallbacks: None,
                    },
                    color: dim_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
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

            let shaped = text_system
                .shape_text(display_text, font_size, &runs, Some(wrap_width), None)
                .expect("single editor line should shape")
                .pop()
                .expect("single editor line should produce a layout");
            let line_height = if is_diagram_embed {
                px(DIAGRAM_EMBED_HEIGHT_PX)
            } else {
                shaped.size(row_height).height
            };
            height_updates.push((i, line_height));
            accumulated_height_delta += line_height - cached_line_height;

            let origin = point(bounds.origin.x + horizontal_inset, y);

            // Cursor — render on the line where the cursor logically belongs
            if cursor_pos >= line_start && cursor_pos <= line_byte_end {
                let idx_in_line = cursor_pos - line_start;
                let display_idx = display_line.source_to_display[idx_in_line];
                let cursor_position = shaped
                    .position_for_index(display_idx, row_height)
                    .unwrap_or_default();
                let cursor_x = cursor_position.x;

                // Determine cursor shape based on vim mode
                let cursor_color = rgb(self.theme.accent);
                let cursor_shape = if vim_enabled && !is_insert {
                    // Block cursor for normal/visual modes
                    let next_x = if idx_in_line < line_text.len() {
                        let next_position = shaped
                            .position_for_index(
                                display_line.source_to_display[idx_in_line + 1],
                                row_height,
                            )
                            .unwrap_or(cursor_position);
                        if next_position.y == cursor_position.y {
                            next_position.x
                        } else {
                            cursor_x + px(8.)
                        }
                    } else {
                        cursor_x + px(8.) // fallback width at end of line
                    };
                    let block_w = (next_x - cursor_x).max(px(8.));
                    size(block_w, row_height)
                } else {
                    size(px(2.), row_height)
                };

                let cursor_y = y + cursor_position.y;

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
                    let start = shaped
                        .position_for_index(
                            display_line.source_to_display[sel_start - line_start],
                            row_height,
                        )
                        .unwrap_or_default();
                    let end = shaped
                        .position_for_index(
                            display_line.source_to_display[sel_end - line_start],
                            row_height,
                        )
                        .unwrap_or(start);
                    let color = rgba((self.theme.accent << 8) | 0x30);
                    if start.y == end.y {
                        selection_quads.push(fill(
                            Bounds::from_corners(
                                point(origin.x + start.x, y + start.y),
                                point(origin.x + end.x, y + start.y + row_height),
                            ),
                            color,
                        ));
                    } else {
                        selection_quads.push(fill(
                            Bounds::from_corners(
                                point(origin.x + start.x, y + start.y),
                                point(origin.x + wrap_width, y + start.y + row_height),
                            ),
                            color,
                        ));
                        let mut row_y = start.y + row_height;
                        while row_y < end.y {
                            selection_quads.push(fill(
                                Bounds::new(
                                    point(origin.x, y + row_y),
                                    size(wrap_width, row_height),
                                ),
                                color,
                            ));
                            row_y += row_height;
                        }
                        selection_quads.push(fill(
                            Bounds::from_corners(
                                point(origin.x, y + end.y),
                                point(origin.x + end.x, y + end.y + row_height),
                            ),
                            color,
                        ));
                    }
                }
            }

            prepaint_lines.push(PrepaintLine {
                shaped,
                origin,
                row_height,
                line_height,
                is_diagram_embed,
                content_offset: line_start,
                source_len: line_text.len(),
                source_to_display: display_line.source_to_display,
                display_to_source: display_line.display_to_source,
            });
        }

        self.state.update(cx, |state, cx| {
            if state.display_map.update_line_heights(&height_updates) {
                cx.notify();
            }
        });

        // Default cursor if none set (e.g. cursor at very end of document beyond visible lines)
        if cursor_quad.is_none() {
            let cw = if vim_enabled && !is_insert {
                px(8.)
            } else {
                px(2.)
            };
            cursor_quad = Some(fill(
                Bounds::new(
                    point(
                        bounds.origin.x + horizontal_inset,
                        bounds.origin.y + vertical_padding - scroll,
                    ),
                    size(cw, px(24.)),
                ),
                rgb(self.theme.accent),
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
        window.paint_quad(fill(bounds, rgb(self.theme.background)));

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
            if !pl.is_diagram_embed {
                let _ =
                    pl.shaped
                        .paint(pl.origin, pl.row_height, TextAlign::Left, None, window, cx);
            }

            line_paint_infos.push(LinePaintInfo {
                content_offset: pl.content_offset,
                shaped_line: pl.shaped.clone(),
                origin_x: pl.origin.x,
                source_len: pl.source_len,
                source_to_display: pl.source_to_display.clone(),
                display_to_source: pl.display_to_source.clone(),
                y: pl.origin.y,
                row_height: pl.row_height,
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

#[cfg(test)]
mod tests {
    use super::{build_display_line, content_inset};
    use crate::markdown::parse_inline_styles;
    use gpui::px;
    use std::collections::HashMap;

    #[test]
    fn centers_configured_editor_width_in_wide_viewport() {
        let inset: f32 = content_inset(px(1200.0), px(760.0)).into();
        assert_eq!(inset, 220.0);
    }

    #[test]
    fn keeps_minimum_padding_in_narrow_viewport() {
        let inset: f32 = content_inset(px(700.0), px(760.0)).into();
        assert_eq!(inset, 24.0);
    }

    #[test]
    fn note_wikilink_renders_canonical_title() {
        let source = "See [[id:abc]] now";
        let spans = parse_inline_styles(source);
        let titles = HashMap::from([("id:abc".to_string(), "My Note".to_string())]);

        let display = build_display_line(source, &spans, &titles, None);

        assert_eq!(display.text, "See My Note now");
    }

    #[test]
    fn pdf_annotation_wikilink_renders_selected_text_alias() {
        let source = "[[paper.pdf#annotation=memex:abc|selected text]]";
        let spans = parse_inline_styles(source);

        let display = build_display_line(source, &spans, &HashMap::new(), None);

        assert_eq!(display.text, "selected text");
        assert_eq!(display.display_to_source[1], 0);
        assert_eq!(display.source_to_display[source.len()], display.text.len());
    }

    #[test]
    fn unresolved_wikilink_renders_target_without_markup() {
        let source = "[[Missing Note]]";
        let spans = parse_inline_styles(source);

        let display = build_display_line(source, &spans, &HashMap::new(), None);

        assert_eq!(display.text, "Missing Note");
    }

    #[test]
    fn wikilink_under_cursor_renders_raw_source() {
        let source = "See [[id:abc]] now";
        let spans = parse_inline_styles(source);
        let titles = HashMap::from([("id:abc".to_string(), "My Note".to_string())]);

        let display = build_display_line(source, &spans, &titles, Some(8));

        assert_eq!(display.text, source);
        assert_eq!(display.source_to_display[8], 8);
        assert_eq!(display.display_to_source[8], 8);
    }

    #[test]
    fn only_wikilink_under_cursor_renders_raw_source() {
        let source = "[[id:a]] and [[id:b]]";
        let spans = parse_inline_styles(source);
        let titles = HashMap::from([
            ("id:a".to_string(), "First".to_string()),
            ("id:b".to_string(), "Second".to_string()),
        ]);

        let display = build_display_line(source, &spans, &titles, Some(3));

        assert_eq!(display.text, "[[id:a]] and Second");
    }
}
