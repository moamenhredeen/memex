use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

// ---------------------------------------------------------------------------
// Blink cursor helper (adapted from gpui-component)
// ---------------------------------------------------------------------------

pub struct BlinkCursor {
    visible: bool,
    paused: bool,
    epoch: usize,
    _task: Task<()>,
}

impl BlinkCursor {
    pub fn new() -> Self {
        Self {
            visible: false,
            paused: false,
            epoch: 0,
            _task: Task::ready(()),
        }
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        self.blink(self.epoch, cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.epoch = 0;
        self.visible = false;
        cx.notify();
    }

    fn next_epoch(&mut self) -> usize {
        self.epoch += 1;
        self.epoch
    }

    fn blink(&mut self, epoch: usize, cx: &mut Context<Self>) {
        if self.paused || epoch != self.epoch {
            self.visible = true;
            return;
        }
        self.visible = !self.visible;
        cx.notify();
        let epoch = self.next_epoch();
        self._task = cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(500)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| this.blink(epoch, cx)).ok();
            }
        });
    }

    pub fn visible(&self) -> bool {
        self.paused || self.visible
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.paused = true;
        self.visible = true;
        cx.notify();
        let epoch = self.next_epoch();
        self._task = cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(300)).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    this.paused = false;
                    this.blink(epoch, cx);
                })
                .ok();
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Document model — parsed from markdown
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Block {
    /// What kind of block
    pub kind: BlockKind,
    /// Range in raw content string
    pub source_range: Range<usize>,
    /// Display text (without markdown syntax characters)
    pub display_text: String,
    /// Inline formatting spans within display_text
    pub spans: Vec<InlineSpan>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Heading(u8),      // level 1-6
    Paragraph,
    CodeBlock,
    ListItem(bool),   // ordered?
    TaskItem(bool),   // checked?
    BlockQuote,
    ThematicBreak,
}

#[derive(Clone, Debug)]
pub struct InlineSpan {
    /// Range within the block's display_text
    pub range: Range<usize>,
    pub style: SpanStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpanStyle {
    Normal,
    Bold,
    Italic,
    BoldItalic,
    Code,
    Strikethrough,
}

impl BlockKind {
    pub fn font_size(&self) -> Pixels {
        match self {
            BlockKind::Heading(1) => px(28.),
            BlockKind::Heading(2) => px(24.),
            BlockKind::Heading(3) => px(20.),
            BlockKind::Heading(4) => px(18.),
            BlockKind::Heading(_) => px(16.),
            BlockKind::CodeBlock => px(14.),
            _ => px(15.),
        }
    }

    pub fn line_height_multiplier(&self) -> f32 {
        match self {
            BlockKind::Heading(_) => 1.4,
            BlockKind::CodeBlock => 1.5,
            _ => 1.6,
        }
    }

    pub fn font_weight(&self) -> FontWeight {
        match self {
            BlockKind::Heading(_) => FontWeight::BOLD,
            _ => FontWeight::NORMAL,
        }
    }

    pub fn bottom_spacing(&self) -> Pixels {
        match self {
            BlockKind::Heading(1) => px(16.),
            BlockKind::Heading(2) => px(14.),
            BlockKind::Heading(3) => px(12.),
            BlockKind::Heading(_) => px(10.),
            BlockKind::ThematicBreak => px(16.),
            _ => px(8.),
        }
    }
}

// ---------------------------------------------------------------------------
// Markdown parser → Block AST
// ---------------------------------------------------------------------------

pub fn parse_blocks(content: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(content, opts);

    let mut current_block_kind: Option<BlockKind> = None;
    let mut display_text = String::new();
    let mut spans: Vec<InlineSpan> = Vec::new();
    let mut style_stack: Vec<SpanStyle> = vec![SpanStyle::Normal];
    let mut block_source_start: usize = 0;

    // Track source position via offset_iter
    let parser = Parser::new_ext(content, opts).into_offset_iter();

    for (event, range) in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    current_block_kind = Some(BlockKind::Heading(level as u8));
                    display_text.clear();
                    spans.clear();
                    block_source_start = range.start;
                }
                Tag::Paragraph => {
                    if current_block_kind.is_none() {
                        current_block_kind = Some(BlockKind::Paragraph);
                        display_text.clear();
                        spans.clear();
                        block_source_start = range.start;
                    }
                }
                Tag::CodeBlock(_) => {
                    current_block_kind = Some(BlockKind::CodeBlock);
                    display_text.clear();
                    spans.clear();
                    block_source_start = range.start;
                }
                Tag::List(ordered) => {
                    // List container — items handled individually
                    let _ = ordered;
                }
                Tag::Item => {
                    current_block_kind = Some(BlockKind::ListItem(false));
                    display_text.clear();
                    spans.clear();
                    block_source_start = range.start;
                }
                Tag::BlockQuote(_) => {
                    if current_block_kind.is_none() {
                        current_block_kind = Some(BlockKind::BlockQuote);
                        display_text.clear();
                        spans.clear();
                        block_source_start = range.start;
                    }
                }
                Tag::Strong => {
                    let current = style_stack.last().cloned().unwrap_or(SpanStyle::Normal);
                    let new_style = match current {
                        SpanStyle::Italic => SpanStyle::BoldItalic,
                        _ => SpanStyle::Bold,
                    };
                    style_stack.push(new_style);
                }
                Tag::Emphasis => {
                    let current = style_stack.last().cloned().unwrap_or(SpanStyle::Normal);
                    let new_style = match current {
                        SpanStyle::Bold => SpanStyle::BoldItalic,
                        _ => SpanStyle::Italic,
                    };
                    style_stack.push(new_style);
                }
                Tag::Strikethrough => {
                    style_stack.push(SpanStyle::Strikethrough);
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::CodeBlock
                | TagEnd::Item | TagEnd::BlockQuote(_) => {
                    if let Some(kind) = current_block_kind.take() {
                        blocks.push(Block {
                            kind,
                            source_range: block_source_start..range.end,
                            display_text: display_text.clone(),
                            spans: spans.clone(),
                        });
                    }
                    display_text.clear();
                    spans.clear();
                    style_stack.clear();
                    style_stack.push(SpanStyle::Normal);
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                _ => {}
            },
            Event::Text(text) => {
                if current_block_kind.is_some() {
                    let start = display_text.len();
                    display_text.push_str(&text);
                    let end = display_text.len();
                    let style = style_stack.last().cloned().unwrap_or(SpanStyle::Normal);
                    spans.push(InlineSpan {
                        range: start..end,
                        style,
                    });
                }
            }
            Event::Code(code) => {
                if current_block_kind.is_some() {
                    let start = display_text.len();
                    display_text.push_str(&code);
                    let end = display_text.len();
                    spans.push(InlineSpan {
                        range: start..end,
                        style: SpanStyle::Code,
                    });
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if current_block_kind.is_some() {
                    let start = display_text.len();
                    display_text.push('\n');
                    let end = display_text.len();
                    spans.push(InlineSpan {
                        range: start..end,
                        style: SpanStyle::Normal,
                    });
                }
            }
            Event::Rule => {
                blocks.push(Block {
                    kind: BlockKind::ThematicBreak,
                    source_range: range.clone(),
                    display_text: String::new(),
                    spans: Vec::new(),
                });
            }
            Event::TaskListMarker(checked) => {
                if let Some(ref mut kind) = current_block_kind {
                    *kind = BlockKind::TaskItem(checked);
                }
            }
            _ => {}
        }
    }

    // If content is empty or has no markdown, treat as single paragraph
    if blocks.is_empty() && !content.is_empty() {
        blocks.push(Block {
            kind: BlockKind::Paragraph,
            source_range: 0..content.len(),
            display_text: content.to_string(),
            spans: vec![InlineSpan {
                range: 0..content.len(),
                style: SpanStyle::Normal,
            }],
        });
    }

    blocks
}

// ---------------------------------------------------------------------------
// Cursor mapping: raw content offset ↔ block + display offset
// ---------------------------------------------------------------------------

/// Given a cursor byte offset in raw content, find which block it belongs to
/// and where in the display text it maps to.
pub fn cursor_to_block_position(cursor: usize, blocks: &[Block]) -> Option<(usize, usize)> {
    for (i, block) in blocks.iter().enumerate() {
        if cursor >= block.source_range.start && cursor <= block.source_range.end {
            // Map raw offset to display text offset
            // Simple approach: clamp within display text length
            let offset_in_source = cursor - block.source_range.start;
            let display_offset = offset_in_source.min(block.display_text.len());
            return Some((i, display_offset));
        }
    }
    // Cursor is beyond all blocks — put at end of last block
    if let Some(last) = blocks.last() {
        Some((blocks.len() - 1, last.display_text.len()))
    } else {
        None
    }
}

/// Given a block index and display text offset, map back to raw content offset.
pub fn block_position_to_cursor(block_idx: usize, display_offset: usize, blocks: &[Block]) -> usize {
    if let Some(block) = blocks.get(block_idx) {
        let clamped = display_offset.min(block.display_text.len());
        // Approximate: source_range.start + display offset
        // This works well for plain paragraphs, headings (syntax is prefix-only)
        (block.source_range.start + clamped).min(block.source_range.end)
    } else if let Some(last) = blocks.last() {
        last.source_range.end
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// EditorState — the entity holding all editor state
// ---------------------------------------------------------------------------

pub struct EditorState {
    pub content: String,
    pub cursor: usize,
    pub selection: Option<(usize, usize)>, // anchor, head
    pub blocks: Vec<Block>,
    pub focus_handle: FocusHandle,
    pub blink_cursor: Entity<BlinkCursor>,
    pub scroll_offset: Pixels,
    _blink_sub: Subscription,
}

impl EditorState {
    pub fn new(content: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let blocks = parse_blocks(&content);
        let focus_handle = cx.focus_handle();
        let blink_cursor = cx.new(|_cx| BlinkCursor::new());

        let _blink_sub = cx.observe(&blink_cursor, |_, _, cx| {
            cx.notify();
        });

        Self {
            cursor: content.len(), // Start cursor at end
            content,
            selection: None,
            blocks,
            focus_handle,
            blink_cursor,
            scroll_offset: px(0.),
            _blink_sub,
        }
    }

    pub fn reparse(&mut self) {
        self.blocks = parse_blocks(&self.content);
    }

    pub fn set_content(&mut self, content: String, window: &mut Window, cx: &mut Context<Self>) {
        self.content = content;
        self.cursor = self.content.len();
        self.selection = None;
        self.reparse();
        cx.notify();
    }

    /// Insert text at cursor position
    pub fn insert(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        // Clamp cursor
        let pos = self.cursor.min(self.content.len());

        // Ensure we're at a char boundary
        let pos = self.snap_to_char_boundary(pos);

        self.content.insert_str(pos, text);
        self.cursor = pos + text.len();
        self.selection = None;
        self.reparse();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Delete character before cursor (backspace)
    pub fn backspace(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.cursor == 0 {
            return;
        }
        let pos = self.cursor.min(self.content.len());
        // Find previous char boundary
        let prev = self.prev_char_boundary(pos);
        self.content.drain(prev..pos);
        self.cursor = prev;
        self.selection = None;
        self.reparse();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Delete character after cursor
    pub fn delete(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let pos = self.cursor.min(self.content.len());
        if pos >= self.content.len() {
            return;
        }
        let next = self.next_char_boundary(pos);
        self.content.drain(pos..next);
        self.selection = None;
        self.reparse();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Move cursor left
    pub fn move_left(&mut self, cx: &mut Context<Self>) {
        if self.cursor > 0 {
            self.cursor = self.prev_char_boundary(self.cursor);
            self.selection = None;
            self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
            cx.notify();
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self, cx: &mut Context<Self>) {
        if self.cursor < self.content.len() {
            self.cursor = self.next_char_boundary(self.cursor);
            self.selection = None;
            self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
            cx.notify();
        }
    }

    /// Move cursor to start of line/block
    pub fn move_home(&mut self, cx: &mut Context<Self>) {
        // Find start of current line
        let before = &self.content[..self.cursor.min(self.content.len())];
        if let Some(nl) = before.rfind('\n') {
            self.cursor = nl + 1;
        } else {
            self.cursor = 0;
        }
        self.selection = None;
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.notify();
    }

    /// Move cursor to end of line/block
    pub fn move_end(&mut self, cx: &mut Context<Self>) {
        let after = &self.content[self.cursor.min(self.content.len())..];
        if let Some(nl) = after.find('\n') {
            self.cursor = self.cursor + nl;
        } else {
            self.cursor = self.content.len();
        }
        self.selection = None;
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.notify();
    }

    /// Move cursor up one line
    pub fn move_up(&mut self, cx: &mut Context<Self>) {
        let pos = self.cursor.min(self.content.len());
        let before = &self.content[..pos];

        // Find current line start
        let current_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = pos - current_line_start;

        if current_line_start == 0 {
            // Already on first line
            self.cursor = 0;
        } else {
            // Find previous line
            let prev_line_end = current_line_start - 1; // the '\n'
            let prev_before = &self.content[..prev_line_end];
            let prev_line_start = prev_before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let prev_line_len = prev_line_end - prev_line_start;
            self.cursor = prev_line_start + col.min(prev_line_len);
        }

        self.selection = None;
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.notify();
    }

    /// Move cursor down one line
    pub fn move_down(&mut self, cx: &mut Context<Self>) {
        let pos = self.cursor.min(self.content.len());
        let before = &self.content[..pos];

        let current_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = pos - current_line_start;

        // Find next line
        let after = &self.content[pos..];
        if let Some(nl) = after.find('\n') {
            let next_line_start = pos + nl + 1;
            let rest = &self.content[next_line_start..];
            let next_line_len = rest.find('\n').unwrap_or(rest.len());
            self.cursor = next_line_start + col.min(next_line_len);
        } else {
            // Already on last line
            self.cursor = self.content.len();
        }

        self.selection = None;
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn snap_to_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos.min(self.content.len());
        while p > 0 && !self.content.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn prev_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos.saturating_sub(1);
        while p > 0 && !self.content.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn next_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos + 1;
        while p < self.content.len() && !self.content.is_char_boundary(p) {
            p += 1;
        }
        p.min(self.content.len())
    }
}

pub enum EditorEvent {
    Changed,
}

impl EventEmitter<EditorEvent> for EditorState {}

// ---------------------------------------------------------------------------
// EntityInputHandler — receive typed characters from the platform
// ---------------------------------------------------------------------------

impl EntityInputHandler for EditorState {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        // Convert UTF-16 range to UTF-8
        let utf8_range = self.utf16_to_utf8_range(&range);
        Some(self.content[utf8_range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let cursor_utf16 = self.utf8_to_utf16_offset(self.cursor);
        Some(UTF16Selection {
            range: cursor_utf16..cursor_utf16,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(range_utf16) = range {
            let utf8_range = self.utf16_to_utf8_range(&range_utf16);
            self.content.replace_range(utf8_range.clone(), text);
            self.cursor = utf8_range.start + text.len();
        } else {
            // Replace at cursor (or selection if any)
            let pos = self.cursor.min(self.content.len());
            self.content.insert_str(pos, text);
            self.cursor = pos + text.len();
        }
        self.selection = None;
        self.reparse();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range, new_text, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl EditorState {
    fn utf8_to_utf16_offset(&self, utf8_offset: usize) -> usize {
        self.content[..utf8_offset.min(self.content.len())]
            .encode_utf16()
            .count()
    }

    fn utf16_to_utf8_range(&self, utf16_range: &Range<usize>) -> Range<usize> {
        let mut utf16_count = 0;
        let mut start_utf8 = self.content.len();
        let mut end_utf8 = self.content.len();

        for (i, _ch) in self.content.char_indices() {
            if utf16_count == utf16_range.start {
                start_utf8 = i;
            }
            if utf16_count == utf16_range.end {
                end_utf8 = i;
                break;
            }
            utf16_count += _ch.len_utf16();
        }
        if utf16_count <= utf16_range.end {
            end_utf8 = self.content.len();
        }
        start_utf8..end_utf8
    }
}

// ---------------------------------------------------------------------------
// EditorElement — custom gpui Element for WYSIWYG rendering
// ---------------------------------------------------------------------------

/// Layout info for one rendered block (computed during prepaint)
struct BlockLayout {
    /// Y position relative to editor origin
    y: Pixels,
    /// Height of this block
    height: Pixels,
    /// The shaped line for this block's display text
    line: ShapedLine,
    /// The line layout for cursor/position calculations
    line_layout: Arc<LineLayout>,
    /// Font size used for this block
    font_size: Pixels,
    /// Line height in pixels
    line_height: Pixels,
    /// Block index in the blocks array
    block_idx: usize,
    /// Optional prefix text (for list items)
    prefix: Option<String>,
    /// The block kind (cached to avoid re-reading blocks in paint)
    kind: BlockKind,
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

    fn build_text_runs(block: &Block, font_family: &SharedString, base_color: Hsla) -> Vec<TextRun> {
        if block.display_text.is_empty() {
            return vec![TextRun {
                len: 0,
                font: Font {
                    family: font_family.clone(),
                    weight: block.kind.font_weight(),
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

        let mut runs = Vec::new();

        for span in &block.spans {
            let (weight, style, color, bg, strikethrough) = match span.style {
                SpanStyle::Normal => (
                    block.kind.font_weight(),
                    FontStyle::Normal,
                    base_color,
                    None,
                    None,
                ),
                SpanStyle::Bold => (
                    FontWeight::BOLD,
                    FontStyle::Normal,
                    base_color,
                    None,
                    None,
                ),
                SpanStyle::Italic => (
                    block.kind.font_weight(),
                    FontStyle::Italic,
                    base_color,
                    None,
                    None,
                ),
                SpanStyle::BoldItalic => (
                    FontWeight::BOLD,
                    FontStyle::Italic,
                    base_color,
                    None,
                    None,
                ),
                SpanStyle::Code => (
                    FontWeight::NORMAL,
                    FontStyle::Normal,
                    hsla(0.58, 0.6, 0.45, 1.0), // code color
                    Some(hsla(0.0, 0.0, 0.93, 1.0)), // light bg
                    None,
                ),
                SpanStyle::Strikethrough => (
                    block.kind.font_weight(),
                    FontStyle::Normal,
                    hsla(0.0, 0.0, 0.6, 1.0),
                    None,
                    Some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(hsla(0.0, 0.0, 0.6, 1.0)),
                    }),
                ),
            };

            let font_family_for_span = if span.style == SpanStyle::Code {
                SharedString::from("monospace")
            } else {
                font_family.clone()
            };

            runs.push(TextRun {
                len: span.range.end - span.range.start,
                font: Font {
                    family: font_family_for_span,
                    weight,
                    style,
                    features: FontFeatures::default(),
                    fallbacks: None,
                },
                color,
                background_color: bg,
                underline: None,
                strikethrough,
            });
        }

        runs
    }
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct EditorPrepaintState {
    block_layouts: Vec<BlockLayout>,
    cursor_position: Option<Point<Pixels>>,
    cursor_height: Pixels,
    total_height: Pixels,
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = EditorPrepaintState;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name("memex-editor".into()))
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
        let style = Style {
            size: gpui::Size {
                width: Length::Definite(DefiniteLength::Fraction(1.0)),
                height: Length::Definite(DefiniteLength::Fraction(1.0)),
            },
            overflow: gpui::Point {
                x: Overflow::Hidden,
                y: Overflow::Scroll,
            },
            ..Style::default()
        };
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
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
        let available_width = bounds.size.width - padding * 2.0;

        let font_family: SharedString = "sans-serif".into();
        let base_color = hsla(0.0, 0.0, 0.16, 1.0); // dark text
        let heading_color = hsla(0.0, 0.0, 0.08, 1.0); // darker for headings

        let mut block_layouts = Vec::new();
        let mut y_offset = padding;

        let cursor_pos = state.cursor;

        // Find which block the cursor is in
        let cursor_block_info = cursor_to_block_position(cursor_pos, &state.blocks);

        let mut cursor_position: Option<Point<Pixels>> = None;
        let mut cursor_height = px(20.);

        let text_system = window.text_system().clone();

        for (block_idx, block) in state.blocks.iter().enumerate() {
            let font_size = block.kind.font_size();
            let line_height = font_size * block.kind.line_height_multiplier();

            let color = match block.kind {
                BlockKind::Heading(_) => heading_color,
                _ => base_color,
            };

            // Handle thematic break specially
            if block.kind == BlockKind::ThematicBreak {
                let empty_layout = text_system.layout_line("", font_size, &[], None);
                block_layouts.push(BlockLayout {
                    y: y_offset,
                    height: px(2.),
                    line: text_system.shape_line(
                        SharedString::from(""),
                        font_size,
                        &[],
                        None,
                    ),
                    line_layout: empty_layout,
                    font_size,
                    line_height,
                    block_idx,
                    prefix: None,
                    kind: BlockKind::ThematicBreak,
                });
                y_offset += px(2.) + block.kind.bottom_spacing();
                continue;
            }

            // Build text to render, with optional prefix
            let prefix = match &block.kind {
                BlockKind::ListItem(false) => Some("• ".to_string()),
                BlockKind::ListItem(true) => Some("1. ".to_string()),
                BlockKind::TaskItem(checked) => {
                    if *checked {
                        Some("☑ ".to_string())
                    } else {
                        Some("☐ ".to_string())
                    }
                }
                BlockKind::BlockQuote => Some("│ ".to_string()),
                _ => None,
            };

            let display_with_prefix = if let Some(ref p) = prefix {
                format!("{}{}", p, &block.display_text)
            } else {
                block.display_text.clone()
            };

            // Build text runs
            let mut runs = Vec::new();
            if let Some(ref p) = prefix {
                // Prefix run
                runs.push(TextRun {
                    len: p.len(),
                    font: Font {
                        family: font_family.clone(),
                        weight: FontWeight::NORMAL,
                        style: FontStyle::Normal,
                        features: FontFeatures::default(),
                        fallbacks: None,
                    },
                    color: hsla(0.0, 0.0, 0.5, 1.0),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
            }
            runs.extend(Self::build_text_runs(block, &font_family, color));

            let text = if display_with_prefix.is_empty() {
                // Empty block — render a space so we get line height
                SharedString::from(" ")
            } else {
                SharedString::from(display_with_prefix.clone())
            };

            // Adjust runs for empty block
            let final_runs = if display_with_prefix.is_empty() {
                vec![TextRun {
                    len: 1,
                    font: Font {
                        family: font_family.clone(),
                        weight: block.kind.font_weight(),
                        style: FontStyle::Normal,
                        features: FontFeatures::default(),
                        fallbacks: None,
                    },
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }]
            } else {
                runs
            };

            let shaped_line = text_system.shape_line(
                text.clone(),
                font_size,
                &final_runs,
                Some(available_width),
            );

            // Also layout for cursor calculation
            let line_layout = text_system.layout_line(
                &text,
                font_size,
                &final_runs,
                Some(available_width),
            );

            let block_height = line_height;

            // Calculate cursor position if cursor is in this block
            if let Some((cursor_block, cursor_display_offset)) = cursor_block_info {
                if cursor_block == block_idx {
                    let prefix_len = prefix.as_ref().map(|p| p.len()).unwrap_or(0);
                    let visual_offset = prefix_len + cursor_display_offset;
                    let x = line_layout.x_for_index(visual_offset);
                    cursor_position = Some(Point::new(
                        bounds.origin.x + padding + x,
                        bounds.origin.y + y_offset - state.scroll_offset,
                    ));
                    cursor_height = line_height;
                }
            }

            let kind = block.kind.clone();
            let bottom_spacing = kind.bottom_spacing();

            block_layouts.push(BlockLayout {
                y: y_offset,
                height: block_height,
                line: shaped_line,
                line_layout,
                font_size,
                line_height,
                block_idx,
                prefix,
                kind,
            });

            y_offset += block_height + bottom_spacing;
        }

        // If no blocks at all, cursor at origin
        if cursor_position.is_none() {
            cursor_position = Some(Point::new(
                bounds.origin.x + padding,
                bounds.origin.y + padding - state.scroll_offset,
            ));
            cursor_height = px(20.);
        }

        EditorPrepaintState {
            block_layouts,
            cursor_position,
            cursor_height,
            total_height: y_offset,
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
        let scroll = self.state.read(cx).scroll_offset;
        let padding = px(24.);

        // Paint background
        window.paint_quad(fill(bounds, hsla(0.0, 0.0, 0.98, 1.0)));

        // Paint each block (using cached kind from BlockLayout, not state.blocks)
        for bl in &prepaint.block_layouts {
            let origin = Point::new(
                bounds.origin.x + padding,
                bounds.origin.y + bl.y - scroll,
            );

            // Skip if out of view
            if origin.y + bl.height < bounds.origin.y || origin.y > bounds.origin.y + bounds.size.height {
                continue;
            }

            // Thematic break: draw a line
            if bl.kind == BlockKind::ThematicBreak {
                let line_bounds = Bounds::new(
                    Point::new(bounds.origin.x + padding, origin.y),
                    gpui::Size {
                        width: bounds.size.width - padding * 2.0,
                        height: px(1.),
                    },
                );
                window.paint_quad(fill(line_bounds, hsla(0.0, 0.0, 0.8, 1.0)));
                continue;
            }

            // Code block background
            if bl.kind == BlockKind::CodeBlock {
                let bg_bounds = Bounds::new(
                    Point::new(bounds.origin.x + padding - px(8.), origin.y - px(4.)),
                    gpui::Size {
                        width: bounds.size.width - padding * 2.0 + px(16.),
                        height: bl.height + px(8.),
                    },
                );
                window.paint_quad(PaintQuad {
                    bounds: bg_bounds,
                    corner_radii: (px(4.)).into(),
                    background: hsla(0.0, 0.0, 0.93, 1.0).into(),
                    border_widths: (px(0.)).into(),
                    border_color: transparent_black(),
                    border_style: BorderStyle::default(),
                });
            }

            // Paint the shaped text
            let _ = bl.line.paint(origin, bl.line_height, window, cx);
        }

        // Paint cursor
        let state = self.state.read(cx);
        let blink_visible = state.blink_cursor.read(cx).visible();
        let is_focused = state.focus_handle.is_focused(window);
        let focus_handle = state.focus_handle.clone();
        drop(state);

        if is_focused && blink_visible {
            if let Some(cursor_pos) = prepaint.cursor_position {
                // Only draw if within bounds
                if cursor_pos.y >= bounds.origin.y
                    && cursor_pos.y < bounds.origin.y + bounds.size.height
                {
                    let cursor_bounds = Bounds::new(
                        cursor_pos,
                        gpui::Size {
                            width: px(1.5),
                            height: prepaint.cursor_height,
                        },
                    );
                    window.paint_quad(fill(cursor_bounds, hsla(0.6, 0.8, 0.5, 1.0)));
                }
            }
        }

        // Register input handler for text input
        if is_focused {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.state.clone()),
                cx,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// EditorView — the Render-implementing wrapper
// ---------------------------------------------------------------------------

pub struct EditorView {
    pub state: Entity<EditorState>,
    focus_handle: FocusHandle,
}

impl EditorView {
    pub fn new(state: Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        let focus_handle = state.read(cx).focus_handle.clone();
        Self { state, focus_handle }
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_entity = self.state.clone();

        div()
            .id("editor-container")
            .size_full()
            .focusable()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                let ctrl = e.keystroke.modifiers.control;

                this.state.update(cx, |state, cx| {
                    match key {
                        "backspace" => state.backspace(window, cx),
                        "delete" => state.delete(window, cx),
                        "left" => state.move_left(cx),
                        "right" => state.move_right(cx),
                        "up" => state.move_up(cx),
                        "down" => state.move_down(cx),
                        "home" => state.move_home(cx),
                        "end" => state.move_end(cx),
                        "enter" => state.insert("\n", window, cx),
                        "tab" => state.insert("    ", window, cx),
                        _ => {}
                    }
                });
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                // Focus the editor on click
                this.state.read(cx).focus(window);
                this.state.update(cx, |state, cx| {
                    state.blink_cursor.update(cx, |bc, cx| bc.start(cx));
                });
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
