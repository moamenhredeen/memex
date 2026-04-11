use std::ops::Range;

use gpui::*;

// ---------------------------------------------------------------------------
// Blink cursor helper
// ---------------------------------------------------------------------------

pub struct BlinkCursor {
    visible: bool,
    epoch: usize,
    _task: Task<()>,
}

impl BlinkCursor {
    pub fn new() -> Self {
        Self {
            visible: true,
            epoch: 0,
            _task: Task::ready(()),
        }
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.epoch += 1;
        self.blink(self.epoch, cx);
    }

    fn blink(&mut self, epoch: usize, cx: &mut Context<Self>) {
        self._task = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(std::time::Duration::from_millis(500)).await;
                if let Some(this) = this.upgrade() {
                    let should_continue = this.update(cx, |this, cx| {
                        if this.epoch != epoch {
                            return false;
                        }
                        this.visible = !this.visible;
                        cx.notify();
                        true
                    }).unwrap_or(false);
                    if !should_continue {
                        break;
                    }
                } else {
                    break;
                }
            }
        });
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.epoch += 1;
        let epoch = self.epoch;
        self._task = cx.spawn(async move |this, cx| {
            Timer::after(std::time::Duration::from_millis(300)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.epoch == epoch {
                        this.blink(epoch, cx);
                    }
                }).ok();
            }
        });
        cx.notify();
    }

    pub fn visible(&self) -> bool {
        self.visible
    }
}

// ---------------------------------------------------------------------------
// Markdown style spans — computed per line for TextRun generation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct StyleSpan {
    range: Range<usize>,
    kind: StyleKind,
}

#[derive(Clone, Debug, PartialEq)]
enum StyleKind {
    Normal,
    Bold,
    Italic,
    BoldItalic,
    Code,
    Strikethrough,
    HeadingSyntax,
    CodeFence,
    HrSyntax,
}

#[derive(Clone, Debug, PartialEq)]
enum LineKind {
    Normal,
    Heading(u8),
    CodeBlock,
    ThematicBreak,
}

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

// ---------------------------------------------------------------------------
// Line analysis — determines kind and inline style spans
// ---------------------------------------------------------------------------

struct LineInfo {
    kind: LineKind,
    spans: Vec<StyleSpan>,
}

fn analyze_line(line: &str, in_code_block: &mut bool) -> LineInfo {
    let trimmed = line.trim();

    // Code fence toggle
    if trimmed.starts_with("```") {
        *in_code_block = !*in_code_block;
        return LineInfo {
            kind: LineKind::CodeBlock,
            spans: vec![StyleSpan {
                range: 0..line.len().max(1),
                kind: StyleKind::CodeFence,
            }],
        };
    }

    // Inside code block
    if *in_code_block {
        return LineInfo {
            kind: LineKind::CodeBlock,
            spans: vec![StyleSpan {
                range: 0..line.len().max(1),
                kind: StyleKind::Code,
            }],
        };
    }

    // Thematic break
    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return LineInfo {
            kind: LineKind::ThematicBreak,
            spans: vec![StyleSpan {
                range: 0..line.len().max(1),
                kind: StyleKind::HrSyntax,
            }],
        };
    }

    // Headings — check longest prefix first
    for level in (1u8..=6).rev() {
        let prefix = "#".repeat(level as usize);
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            if rest.is_empty() || rest.starts_with(' ') {
                return heading_line_info(line, level);
            }
        }
    }

    // Normal line with inline styles
    let spans = parse_inline_styles(line);
    LineInfo {
        kind: LineKind::Normal,
        spans,
    }
}

fn heading_line_info(line: &str, level: u8) -> LineInfo {
    let prefix_end = line.find(' ').map(|i| i + 1).unwrap_or(line.len());
    let mut spans = vec![StyleSpan {
        range: 0..prefix_end,
        kind: StyleKind::HeadingSyntax,
    }];
    if prefix_end < line.len() {
        let content_spans = parse_inline_styles(&line[prefix_end..]);
        for mut s in content_spans {
            s.range = (s.range.start + prefix_end)..(s.range.end + prefix_end);
            spans.push(s);
        }
    }
    LineInfo {
        kind: LineKind::Heading(level),
        spans,
    }
}

fn parse_inline_styles(text: &str) -> Vec<StyleSpan> {
    if text.is_empty() {
        return vec![StyleSpan {
            range: 0..0,
            kind: StyleKind::Normal,
        }];
    }

    let mut spans = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut normal_start = 0;

    while i < len {
        // Skip if not at a char boundary (inside multi-byte UTF-8)
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        // Only check ASCII markdown delimiters
        if bytes[i] == b'`' {
            if let Some(end) = find_closing(text, i + 1, "`") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 1,
                    kind: StyleKind::Code,
                });
                i = end + 1;
                normal_start = i;
                continue;
            }
        }
        if bytes[i] == b'*' && i + 2 < len && bytes[i + 1] == b'*' && bytes[i + 2] == b'*' {
            if let Some(end) = find_closing(text, i + 3, "***") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 3,
                    kind: StyleKind::BoldItalic,
                });
                i = end + 3;
                normal_start = i;
                continue;
            }
        }
        if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'*' {
            if let Some(end) = find_closing(text, i + 2, "**") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 2,
                    kind: StyleKind::Bold,
                });
                i = end + 2;
                normal_start = i;
                continue;
            }
        }
        if bytes[i] == b'*' {
            if let Some(end) = find_closing(text, i + 1, "*") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 1,
                    kind: StyleKind::Italic,
                });
                i = end + 1;
                normal_start = i;
                continue;
            }
        }
        if bytes[i] == b'~' && i + 1 < len && bytes[i + 1] == b'~' {
            if let Some(end) = find_closing(text, i + 2, "~~") {
                push_normal(&mut spans, normal_start, i);
                spans.push(StyleSpan {
                    range: i..end + 2,
                    kind: StyleKind::Strikethrough,
                });
                i = end + 2;
                normal_start = i;
                continue;
            }
        }
        i += 1;
    }

    push_normal(&mut spans, normal_start, len);

    if spans.is_empty() {
        spans.push(StyleSpan {
            range: 0..text.len(),
            kind: StyleKind::Normal,
        });
    }

    spans
}

fn push_normal(spans: &mut Vec<StyleSpan>, start: usize, end: usize) {
    if end > start {
        spans.push(StyleSpan {
            range: start..end,
            kind: StyleKind::Normal,
        });
    }
}

fn find_closing(text: &str, start: usize, delimiter: &str) -> Option<usize> {
    if start >= text.len() {
        return None;
    }
    text[start..].find(delimiter).map(|pos| start + pos)
}

// ---------------------------------------------------------------------------
// EditorState — content, cursor, selection
// ---------------------------------------------------------------------------

pub struct EditorState {
    pub content: String,
    pub cursor: usize,
    pub selected_range: Range<usize>,
    pub selection_reversed: bool,
    pub marked_range: Option<Range<usize>>,
    pub focus_handle: FocusHandle,
    pub blink_cursor: Entity<BlinkCursor>,
    pub scroll_offset: Pixels,
    pub last_line_layouts: Vec<LinePaintInfo>,
    pub last_bounds: Option<Bounds<Pixels>>,
    _blink_sub: Subscription,
}

#[derive(Clone)]
pub struct LinePaintInfo {
    pub content_offset: usize,
    pub shaped_line: ShapedLine,
    pub y: Pixels,
    pub line_height: Pixels,
}

impl EditorState {
    pub fn new(content: String, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let blink_cursor = cx.new(|_cx| BlinkCursor::new());
        let _blink_sub = cx.observe(&blink_cursor, |_, _, cx| cx.notify());

        Self {
            cursor: 0,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            content,
            focus_handle,
            blink_cursor,
            scroll_offset: px(0.),
            last_line_layouts: Vec::new(),
            last_bounds: None,
            _blink_sub,
        }
    }

    pub fn set_content(&mut self, content: String, _window: &mut Window, cx: &mut Context<Self>) {
        self.content = content;
        self.cursor = 0;
        self.selected_range = 0..0;
        self.marked_range = None;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    pub fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.cursor = offset;
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.notify();
    }

    pub fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.cursor = self.cursor_offset();
        cx.notify();
    }

    pub fn prev_grapheme(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }
        let mut p = offset - 1;
        while p > 0 && !self.content.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    pub fn next_grapheme(&self, offset: usize) -> usize {
        if offset >= self.content.len() {
            return self.content.len();
        }
        let mut p = offset + 1;
        while p < self.content.len() && !self.content.is_char_boundary(p) {
            p += 1;
        }
        p
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let bounds = match &self.last_bounds {
            Some(b) => b,
            None => return 0,
        };

        if self.last_line_layouts.is_empty() {
            return 0;
        }

        if position.y < self.last_line_layouts[0].y {
            return 0;
        }

        for ll in &self.last_line_layouts {
            if position.y >= ll.y && position.y < ll.y + ll.line_height {
                let local_x = (position.x - bounds.left() - px(24.)).max(px(0.));
                let idx_in_line = ll.shaped_line.closest_index_for_x(local_x);
                return ll.content_offset + idx_in_line;
            }
        }

        // Below all lines
        self.content.len()
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())]
            .encode_utf16()
            .count()
    }

    fn offset_from_utf16(&self, utf16_offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= utf16_offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

pub enum EditorEvent {
    Changed,
}

impl EventEmitter<EditorEvent> for EditorState {}

// ---------------------------------------------------------------------------
// EntityInputHandler — platform text input
// ---------------------------------------------------------------------------

impl EntityInputHandler for EditorState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        let new_cursor = range.start + new_text.len();
        self.selected_range = new_cursor..new_cursor;
        self.cursor = new_cursor;
        self.marked_range.take();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();

        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|r| r.start + range.start..r.end + range.end)
            .unwrap_or_else(|| {
                let c = range.start + new_text.len();
                c..c
            });
        self.cursor = self.cursor_offset();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        for ll in &self.last_line_layouts {
            let line_end = ll.content_offset + ll.shaped_line.len();
            if range.start >= ll.content_offset && range.start <= line_end {
                let local_start = range.start - ll.content_offset;
                let local_end = (range.end - ll.content_offset).min(ll.shaped_line.len());
                let x1 = ll.shaped_line.x_for_index(local_start);
                let x2 = ll.shaped_line.x_for_index(local_end);
                let padding = px(24.);
                let base_x = self.last_bounds.as_ref().map(|b| b.left()).unwrap_or(px(0.));
                return Some(Bounds::from_corners(
                    point(base_x + padding + x1, ll.y),
                    point(base_x + padding + x2, ll.y + ll.line_height),
                ));
            }
        }
        None
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let idx = self.index_for_mouse_position(point);
        Some(self.offset_to_utf16(idx))
    }
}

// ---------------------------------------------------------------------------
// EditorElement — custom gpui Element
// ---------------------------------------------------------------------------

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
        let available_width = (bounds.size.width - padding * 2.0).max(px(100.));
        let scroll = state.scroll_offset;
        let cursor_pos = state.cursor;
        let selected_range = state.selected_range.clone();
        let content = state.content.clone();
        // state borrow ends here since content is cloned

        let text_system = window.text_system().clone();
        let font_family: SharedString = "sans-serif".into();

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

            // Build display text (use space for empty lines so ShapedLine works)
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
                    };

                    let family = if use_mono {
                        SharedString::from("monospace")
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

            let shaped = text_system.shape_line(
                display_text,
                font_size,
                &runs,
                None,
            );

            let origin = point(bounds.origin.x + padding, y);

            // Cursor: check if cursor falls on this line
            let line_byte_end = line_text_end;
            if cursor_pos >= line_start && cursor_pos <= line_byte_end {
                // Cursor is on this line (or at end of it, not at a \n that leads to next line)
                let at_newline = cursor_pos == line_byte_end
                    && cursor_pos < content.len()
                    && content.as_bytes()[cursor_pos] == b'\n';

                if !at_newline {
                    let idx_in_line = cursor_pos - line_start;
                    let cursor_x = shaped.x_for_index(idx_in_line);
                    cursor_quad = Some(fill(
                        Bounds::new(
                            point(origin.x + cursor_x, y),
                            size(px(2.), line_height),
                        ),
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
                    Bounds::new(
                        point(origin.x, y),
                        size(px(2.), line_height),
                    ),
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
                    point(bounds.origin.x + padding, bounds.origin.y + padding - scroll),
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
        // Register input handler FIRST (following gpui example pattern)
        let focus_handle = self.state.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );

        // Paint background
        window.paint_quad(fill(bounds, hsla(0.0, 0.0, 0.98, 1.0)));

        // Paint selections
        for sel in &prepaint.selection_quads {
            window.paint_quad(sel.clone());
        }

        // Paint lines and collect layout info
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

        // Paint cursor
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

// ---------------------------------------------------------------------------
// EditorView — Render wrapper with keyboard/mouse handling
// ---------------------------------------------------------------------------

pub struct EditorView {
    pub state: Entity<EditorState>,
    focus_handle: FocusHandle,
    is_selecting: bool,
    _observe_state: Subscription,
}

impl EditorView {
    pub fn new(state: Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        // Re-render EditorView whenever EditorState changes
        let _observe_state = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            focus_handle,
            is_selecting: false,
            _observe_state,
        }
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("editor-view")
            .size_full()
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let shift = e.keystroke.modifiers.shift;

                this.state.update(cx, |state, cx| {
                    match key {
                        "backspace" => {
                            if state.selected_range.is_empty() {
                                state.select_to(state.prev_grapheme(state.cursor_offset()), cx);
                            }
                            state.replace_text_in_range(None, "", window, cx);
                        }
                        "delete" => {
                            if state.selected_range.is_empty() {
                                state.select_to(state.next_grapheme(state.cursor_offset()), cx);
                            }
                            state.replace_text_in_range(None, "", window, cx);
                        }
                        "left" => {
                            if shift {
                                state.select_to(state.prev_grapheme(state.cursor_offset()), cx);
                            } else if state.selected_range.is_empty() {
                                state.move_to(state.prev_grapheme(state.cursor_offset()), cx);
                            } else {
                                state.move_to(state.selected_range.start, cx);
                            }
                        }
                        "right" => {
                            if shift {
                                state.select_to(state.next_grapheme(state.cursor_offset()), cx);
                            } else if state.selected_range.is_empty() {
                                state.move_to(state.next_grapheme(state.cursor_offset()), cx);
                            } else {
                                state.move_to(state.selected_range.end, cx);
                            }
                        }
                        "up" => {
                            let pos = state.cursor;
                            let before = &state.content[..pos.min(state.content.len())];
                            let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                            let col = pos - line_start;
                            if line_start == 0 {
                                state.move_to(0, cx);
                            } else {
                                let prev_end = line_start - 1;
                                let prev_start = state.content[..prev_end]
                                    .rfind('\n')
                                    .map(|i| i + 1)
                                    .unwrap_or(0);
                                let prev_len = prev_end - prev_start;
                                state.move_to(prev_start + col.min(prev_len), cx);
                            }
                        }
                        "down" => {
                            let pos = state.cursor;
                            let before = &state.content[..pos.min(state.content.len())];
                            let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                            let col = pos - line_start;
                            let after = &state.content[pos..];
                            if let Some(nl) = after.find('\n') {
                                let next_start = pos + nl + 1;
                                let rest = &state.content[next_start..];
                                let next_len = rest.find('\n').unwrap_or(rest.len());
                                state.move_to(next_start + col.min(next_len), cx);
                            } else {
                                state.move_to(state.content.len(), cx);
                            }
                        }
                        "home" => {
                            let pos = state.cursor.min(state.content.len());
                            let line_start = state.content[..pos]
                                .rfind('\n')
                                .map(|i| i + 1)
                                .unwrap_or(0);
                            state.move_to(line_start, cx);
                        }
                        "end" => {
                            let pos = state.cursor.min(state.content.len());
                            let line_end = state.content[pos..]
                                .find('\n')
                                .map(|p| pos + p)
                                .unwrap_or(state.content.len());
                            state.move_to(line_end, cx);
                        }
                        "enter" => {
                            state.replace_text_in_range(None, "\n", window, cx);
                        }
                        "tab" => {
                            state.replace_text_in_range(None, "    ", window, cx);
                        }
                        _ => {}
                    }
                });
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    this.is_selecting = true;
                    let pos = this.state.read(cx).index_for_mouse_position(e.position);
                    this.state.update(cx, |state, cx| {
                        if e.modifiers.shift {
                            state.select_to(pos, cx);
                        } else {
                            state.move_to(pos, cx);
                        }
                        state.focus_handle.focus(window);
                        state.blink_cursor.update(cx, |bc, cx| bc.start(cx));
                    });
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, _cx| {
                    this.is_selecting = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e: &MouseUpEvent, _window, _cx| {
                    this.is_selecting = false;
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _window, cx| {
                if this.is_selecting {
                    let pos = this.state.read(cx).index_for_mouse_position(e.position);
                    this.state.update(cx, |state, cx| {
                        state.select_to(pos, cx);
                    });
                }
            }))
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _window, cx| {
                this.state.update(cx, |state, cx| {
                    let delta = match e.delta {
                        ScrollDelta::Lines(lines) => lines.y * px(20.),
                        ScrollDelta::Pixels(pixels) => pixels.y,
                    };
                    state.scroll_offset = (state.scroll_offset - delta).max(px(0.));
                    cx.notify();
                });
            }))
            .child(EditorElement::new(&self.state))
    }
}
