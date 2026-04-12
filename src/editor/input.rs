use std::ops::Range;

use gpui::*;

use super::{EditorEvent, EditorState};

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
        let char_start = self.buffer.byte_to_char(range.start);
        let char_end = self.buffer.byte_to_char(range.end);
        Some(self.buffer.slice(char_start..char_end).to_string())
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
        // In vim non-insert modes, suppress OS text input entirely.
        // The keymap system handles all key→action routing, so OS text
        // input should only come through in insert mode (or non-vim mode).
        if !self.keymap.is_insert_active() {
            return;
        }

        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        // Capture old text for undo before mutating
        let old_text = {
            let char_start = self.buffer.byte_to_char(range.start);
            let char_end = self.buffer.byte_to_char(range.end);
            self.buffer.slice(char_start..char_end).to_string()
        };
        let cursor_before = self.cursor;
        let selection_before = self.selected_range.clone();

        self.rope_replace(range.clone(), new_text);

        let new_cursor = range.start + new_text.len();

        // Record undo
        self.history.record(
            super::undo::EditOp {
                range: range.clone(),
                old_text,
                new_text: new_text.to_string(),
                cursor_before,
                cursor_after: new_cursor,
            },
            selection_before,
        );

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
        // In vim non-insert modes, suppress OS text input entirely.
        if !self.keymap.is_insert_active() {
            return;
        }

        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        // Capture old text for undo before mutating
        let old_text = {
            let char_start = self.buffer.byte_to_char(range.start);
            let char_end = self.buffer.byte_to_char(range.end);
            self.buffer.slice(char_start..char_end).to_string()
        };
        let cursor_before = self.cursor;
        let selection_before = self.selected_range.clone();

        self.rope_replace(range.clone(), new_text);

        let new_cursor_pos = range.start + new_text.len();

        // Record undo
        self.history.record(
            super::undo::EditOp {
                range: range.clone(),
                old_text,
                new_text: new_text.to_string(),
                cursor_before,
                cursor_after: new_cursor_pos,
            },
            selection_before,
        );

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
                let base_x = self
                    .last_bounds
                    .as_ref()
                    .map(|b| b.left())
                    .unwrap_or(px(0.));
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
