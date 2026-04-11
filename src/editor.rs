use std::ops::Range;

use gpui::*;

use crate::markdown::{
    analyze_line, compute_col_widths, cursor_pos_in_formatted_table, format_table,
    is_separator_row, parse_table_cells, LineInfo, LineKind, StyleKind,
};

// Tab action for key binding
actions!(editor, [TabAction, ShiftTabAction]);

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

    /// Handle tab/shift-tab inside a table. Returns true if handled.
    pub fn handle_table_tab(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let pos = self.cursor.min(self.content.len());

        // Find current line boundaries
        let line_start = self.content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = self.content[pos..]
            .find('\n')
            .map(|p| pos + p)
            .unwrap_or(self.content.len());
        let current_line = &self.content[line_start..line_end];

        // Check if current line is a table row
        let trimmed = current_line.trim();
        if !trimmed.starts_with('|') || !trimmed.ends_with('|') || trimmed.len() <= 1 {
            return false;
        }

        // Find the full table block
        let table_start = self.find_table_start(line_start);
        let table_end = self.find_table_end(line_end);

        // Determine which cell the cursor is in
        let cursor_col_in_line = pos - line_start;
        let cursor_col_idx = self.cell_index_at(current_line, cursor_col_in_line);

        // Parse the full table into rows of cells
        let table_text = self.content[table_start..table_end].to_string();
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut is_separator = Vec::new();
        for row_str in table_text.split('\n') {
            let trimmed = row_str.trim();
            if trimmed.is_empty() {
                continue;
            }
            let cells = parse_table_cells(trimmed);
            is_separator.push(is_separator_row(trimmed));
            rows.push(cells);
        }

        if rows.is_empty() {
            return false;
        }

        // Find max column count and max width per column
        let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if max_cols == 0 {
            return false;
        }
        let mut col_widths = compute_col_widths(&rows, &is_separator);

        // Determine cursor's row index within the table
        let mut cursor_row_idx = 0;
        {
            let cursor_row_start = line_start;
            let mut offset = table_start;
            for row_str in table_text.split('\n') {
                if offset == cursor_row_start {
                    break;
                }
                offset += row_str.len() + 1;
                cursor_row_idx += 1;
            }
        }

        // Calculate target cell
        let (next_row, next_col, need_new_row) = if forward {
            self.next_table_cell(cursor_row_idx, cursor_col_idx, &rows, &is_separator, max_cols)
        } else {
            self.prev_table_cell(cursor_row_idx, cursor_col_idx, &rows, &is_separator, max_cols)
        };

        if need_new_row {
            rows.push(vec![String::new(); max_cols]);
            is_separator.push(false);
        }

        // Pad all rows to max_cols
        for row in &mut rows {
            while row.len() < max_cols {
                row.push(String::new());
            }
        }

        // Recalculate widths after padding
        col_widths = compute_col_widths(&rows, &is_separator);

        // Rebuild the aligned table
        let new_table = format_table(&rows, &is_separator, &col_widths);

        // Calculate cursor position in the new table string
        let cursor_in_table = cursor_pos_in_formatted_table(
            next_row, next_col, &rows, &col_widths, &is_separator,
        );
        let new_cursor = (table_start + cursor_in_table).min(table_start + new_table.len());

        // Replace table text in content
        let mut new_content = String::with_capacity(self.content.len());
        new_content.push_str(&self.content[..table_start]);
        new_content.push_str(&new_table);
        new_content.push_str(&self.content[table_end..]);
        self.content = new_content;
        self.selected_range = new_cursor..new_cursor;
        self.cursor = new_cursor;
        self.marked_range.take();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
        true
    }

    fn find_table_start(&self, line_start: usize) -> usize {
        let mut start = line_start;
        // Walk backwards to find contiguous table rows
        while start > 0 {
            let prev_end = start - 1; // the \n before this line
            let prev_start = self.content[..prev_end]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let prev_line = self.content[prev_start..prev_end].trim();
            if prev_line.starts_with('|') && prev_line.ends_with('|') && prev_line.len() > 1 {
                start = prev_start;
            } else {
                break;
            }
        }
        start
    }

    fn find_table_end(&self, line_end: usize) -> usize {
        let mut end = line_end;
        // Walk forward to find contiguous table rows
        while end < self.content.len() {
            if self.content.as_bytes()[end] != b'\n' {
                break;
            }
            let next_start = end + 1;
            let next_end = self.content[next_start..]
                .find('\n')
                .map(|p| next_start + p)
                .unwrap_or(self.content.len());
            let next_line = self.content[next_start..next_end].trim();
            if next_line.starts_with('|') && next_line.ends_with('|') && next_line.len() > 1 {
                end = next_end;
            } else {
                break;
            }
        }
        end
    }

    fn cell_index_at(&self, line: &str, col_offset: usize) -> usize {
        // Count pipe characters before col_offset; cell index = pipes - 1
        let pipes = line[..col_offset.min(line.len())]
            .chars()
            .filter(|&c| c == '|')
            .count();
        pipes.saturating_sub(1)
    }

    fn next_table_cell(
        &self,
        row: usize,
        col: usize,
        rows: &[Vec<String>],
        is_separator: &[bool],
        max_cols: usize,
    ) -> (usize, usize, bool) {
        let mut r = row;
        let mut c = col + 1;
        loop {
            if c >= max_cols {
                c = 0;
                r += 1;
            }
            if r >= rows.len() {
                return (r, 0, true);
            }
            if !is_separator[r] {
                return (r, c, false);
            }
            c = 0;
            r += 1;
        }
    }

    fn prev_table_cell(
        &self,
        row: usize,
        col: usize,
        rows: &[Vec<String>],
        is_separator: &[bool],
        max_cols: usize,
    ) -> (usize, usize, bool) {
        let mut r = row as isize;
        let mut c = col as isize - 1;
        loop {
            if c < 0 {
                c = max_cols as isize - 1;
                r -= 1;
            }
            if r < 0 {
                // Already at the very first cell, stay put
                return (0, 0, false);
            }
            if !is_separator[r as usize] {
                return (r as usize, c as usize, false);
            }
            c = max_cols as isize - 1;
            r -= 1;
        }
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
        let scroll = state.scroll_offset;
        let cursor_pos = state.cursor;
        let selected_range = state.selected_range.clone();
        let content = state.content.clone();
        // state borrow ends here since content is cloned

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
                            true, // monospace for table alignment
                        ),
                    };

                    // Use monospace for table rows and code spans
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
            .key_context("Editor")
            .on_action(cx.listener(|this, _: &TabAction, window, cx| {
                this.state.update(cx, |state, cx| {
                    if !state.handle_table_tab(true, cx) {
                        state.replace_text_in_range(None, "    ", window, cx);
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &ShiftTabAction, _window, cx| {
                this.state.update(cx, |state, cx| {
                    state.handle_table_tab(false, cx);
                });
            }))
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


