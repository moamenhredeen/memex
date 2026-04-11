use std::ops::Range;

use gpui::*;

use super::EditorState;

impl EditorState {
    pub fn prev_grapheme(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }
        let offset = offset.min(self.content_len());
        let char_idx = self.buffer.byte_to_char(offset);
        if char_idx == 0 {
            return 0;
        }
        self.buffer.char_to_byte(char_idx - 1)
    }

    pub fn next_grapheme(&self, offset: usize) -> usize {
        let len = self.content_len();
        if offset >= len {
            return len;
        }
        let char_idx = self.buffer.byte_to_char(offset);
        if char_idx + 1 >= self.buffer.len_chars() {
            return len;
        }
        self.buffer.char_to_byte(char_idx + 1)
    }

    pub(crate) fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
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
        self.content_len()
    }

    pub(crate) fn offset_to_utf16(&self, offset: usize) -> usize {
        let content = self.content();
        content[..offset.min(content.len())]
            .encode_utf16()
            .count()
    }

    pub(crate) fn offset_from_utf16(&self, utf16_offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.buffer.chars() {
            if utf16_count >= utf16_offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    pub(crate) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub(crate) fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}
