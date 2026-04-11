use gpui::*;

use crate::markdown::{
    LineInfo, LineKind, StyleKind, analyze_line,
};

use super::{EditorState, LinePaintInfo};

// gpui-specific helpers for LineKind
impl LineKind {
    fn font_size(&self) -> Pixels {
        match self {
            LineKind::Heading(1) => px(28.),
            LineKind::Heading(2) => px(24.),
            LineKind::Heading(3) => px(20.),
            LineKind::Heading(4) => px(18.),
            LineKind::Heading(_) => px(16.),
            LineKind::CodeBlock => px(14.),
            _ => px(15.),
        }
    }

    fn line_height(&self) -> Pixels {
        let fs = self.font_size();
        match self {
            LineKind::Heading(_) => fs * 1.5,
            _ => fs * 1.6,
        }
    }

    fn font_weight(&self) -> FontWeight {
        match self {
            LineKind::Heading(_) => FontWeight::BOLD,
            _ => FontWeight::NORMAL,
        }
    }
}

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
        let state = self.state.read(cx);
        let padding = px(24.);
        let scroll = state.scroll_offset;
        let cursor_pos = state.cursor;
        let selected_range = state.selected_range.clone();
        let content = state.content.clone();

        let text_system = window.text_system().clone();
        let text_style = window.text_style();
        let font_family: SharedString = text_style.font_family.clone();

        let raw_lines: Vec<&str> = if content.is_empty() {
            vec![""]
        } else {
            content.split('\n').collect()
        };

        let mut in_code_block = false;
        let mut line_infos: Vec<(LineInfo, usize)> = Vec::new();
        let mut offset = 0;
        for (i, raw_line) in raw_lines.iter().enumerate() {
            let info = analyze_line(raw_line, &mut in_code_block);
            line_infos.push((info, offset));
            offset += raw_line.len();
            if i < raw_lines.len() - 1 {
                offset += 1; // '\n'
            }
        }

        let mut prepaint_lines = Vec::new();
        let mut cursor_quad: Option<PaintQuad> = None;
        let mut selection_quads = Vec::new();
        let mut y = bounds.origin.y + padding - scroll;

        let base_color = hsla(0.0, 0.0, 0.16, 1.0);
        let dim_color = hsla(0.0, 0.0, 0.6, 1.0);
        let code_color = hsla(0.58, 0.6, 0.45, 1.0);
        let heading_color = hsla(0.0, 0.0, 0.08, 1.0);
        let hr_color = hsla(0.0, 0.0, 0.7, 1.0);

        for (info, content_offset) in &line_infos {
            let line_start = *content_offset;
            let line_text_end = content[line_start..]
                .find('\n')
                .map(|p| line_start + p)
                .unwrap_or(content.len());
            let line_text = &content[line_start..line_text_end];

            let font_size = info.kind.font_size();
            let line_height = info.kind.line_height();

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

                    let (weight, fstyle, color, bg, strike, use_mono) = match &span.kind {
                        StyleKind::Normal => (
                            info.kind.font_weight(),
                            FontStyle::Normal,
                            if matches!(info.kind, LineKind::Heading(_)) {
                                heading_color
                            } else {
                                base_color
                            },
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
                            false,
                        ),
                        StyleKind::Italic => (
                            info.kind.font_weight(),
                            FontStyle::Italic,
                            base_color,
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
                            false,
                        ),
                        StyleKind::Code => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            code_color,
                            Some(hsla(0.0, 0.0, 0.93, 1.0)),
                            None,
                            true,
                        ),
                        StyleKind::Strikethrough => (
                            info.kind.font_weight(),
                            FontStyle::Normal,
                            dim_color,
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
                            false,
                        ),
                        StyleKind::CodeFence => (
                            FontWeight::NORMAL,
                            FontStyle::Normal,
                            dim_color,
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
                            false,
                        ),
                        StyleKind::ListBullet => (
                            FontWeight::BOLD,
                            FontStyle::Normal,
                            dim_color,
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
                        underline: None,
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
                        weight: info.kind.font_weight(),
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

            // Cursor
            let line_byte_end = line_text_end;
            if cursor_pos >= line_start && cursor_pos <= line_byte_end {
                let at_newline = cursor_pos == line_byte_end
                    && cursor_pos < content.len()
                    && content.as_bytes()[cursor_pos] == b'\n';

                if !at_newline {
                    let idx_in_line = cursor_pos - line_start;
                    let cursor_x = shaped.x_for_index(idx_in_line);
                    cursor_quad = Some(fill(
                        Bounds::new(point(origin.x + cursor_x, y), size(px(2.), line_height)),
                        hsla(0.6, 0.8, 0.5, 1.0),
                    ));
                }
            }
            // Cursor at start of this line (right after a \n on prev line)
            if cursor_quad.is_none()
                && cursor_pos == line_start
                && line_start > 0
                && content.as_bytes()[line_start - 1] == b'\n'
            {
                cursor_quad = Some(fill(
                    Bounds::new(point(origin.x, y), size(px(2.), line_height)),
                    hsla(0.6, 0.8, 0.5, 1.0),
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
                        rgba(0x3366ff30),
                    ));
                }
            }

            prepaint_lines.push(PrepaintLine {
                shaped,
                origin,
                line_height,
                content_offset: line_start,
            });

            y += line_height;
        }

        // Default cursor if none set
        if cursor_quad.is_none() {
            cursor_quad = Some(fill(
                Bounds::new(
                    point(
                        bounds.origin.x + padding,
                        bounds.origin.y + padding - scroll,
                    ),
                    size(px(2.), px(24.)),
                ),
                hsla(0.6, 0.8, 0.5, 1.0),
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
        window.paint_quad(fill(bounds, hsla(0.0, 0.0, 0.98, 1.0)));

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
