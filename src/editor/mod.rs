mod blink;
mod element;
mod input;
mod movement;
mod table;
mod view;

use std::ops::Range;

use gpui::*;

pub use blink::BlinkCursor;
pub use view::EditorView;

actions!(editor, [TabAction, ShiftTabAction]);

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
}

pub enum EditorEvent {
    Changed,
}

impl EventEmitter<EditorEvent> for EditorState {}
