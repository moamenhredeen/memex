use gpui::*;

use crate::markdown::{compute_col_widths, cursor_pos_in_formatted_table, format_table, is_separator_row, parse_table_cells};

use super::{EditorEvent, EditorState};

impl EditorState {
    /// Handle tab/shift-tab inside a table. Returns true if handled.
    pub fn handle_table_tab(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let content = self.content();
        let pos = self.cursor.min(content.len());

        // Find current line boundaries
        let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = content[pos..]
            .find('\n')
            .map(|p| pos + p)
            .unwrap_or(content.len());
        let current_line = &content[line_start..line_end];

        // Check if current line is a table row
        let trimmed = current_line.trim();
        if !trimmed.starts_with('|') || !trimmed.ends_with('|') || trimmed.len() <= 1 {
            return false;
        }

        // Find the full table block
        let table_start = Self::find_table_start_in(&content, line_start);
        let table_end = Self::find_table_end_in(&content, line_end);

        // Determine which cell the cursor is in
        let cursor_col_in_line = pos - line_start;
        let cursor_col_idx = Self::cell_index_at_in(current_line, cursor_col_in_line);

        // Parse the full table into rows of cells
        let table_text = &content[table_start..table_end];
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

        // Find max column count
        let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if max_cols == 0 {
            return false;
        }

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
            Self::next_table_cell(cursor_row_idx, cursor_col_idx, &rows, &is_separator, max_cols)
        } else {
            Self::prev_table_cell(cursor_row_idx, cursor_col_idx, &rows, &is_separator, max_cols)
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
        let col_widths = compute_col_widths(&rows, &is_separator);

        // Rebuild the aligned table
        let new_table = format_table(&rows, &is_separator, &col_widths);

        // Calculate cursor position in the new table string
        let cursor_in_table =
            cursor_pos_in_formatted_table(next_row, next_col, &rows, &col_widths, &is_separator);
        let new_cursor = (table_start + cursor_in_table).min(table_start + new_table.len());

        // Replace table text in the rope buffer
        let char_start = self.buffer.byte_to_char(table_start);
        let char_end = self.buffer.byte_to_char(table_end);
        self.buffer.remove(char_start..char_end);
        self.buffer.insert(char_start, &new_table);

        self.selected_range = new_cursor..new_cursor;
        self.cursor = new_cursor;
        self.marked_range.take();
        self.blink_cursor.update(cx, |bc, cx| bc.pause(cx));
        cx.emit(EditorEvent::Changed);
        cx.notify();
        true
    }

    fn find_table_start_in(content: &str, line_start: usize) -> usize {
        let mut start = line_start;
        while start > 0 {
            let prev_end = start - 1;
            let prev_start = content[..prev_end]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let prev_line = content[prev_start..prev_end].trim();
            if prev_line.starts_with('|') && prev_line.ends_with('|') && prev_line.len() > 1 {
                start = prev_start;
            } else {
                break;
            }
        }
        start
    }

    fn find_table_end_in(content: &str, line_end: usize) -> usize {
        let mut end = line_end;
        while end < content.len() {
            if content.as_bytes()[end] != b'\n' {
                break;
            }
            let next_start = end + 1;
            let next_end = content[next_start..]
                .find('\n')
                .map(|p| next_start + p)
                .unwrap_or(content.len());
            let next_line = content[next_start..next_end].trim();
            if next_line.starts_with('|') && next_line.ends_with('|') && next_line.len() > 1 {
                end = next_end;
            } else {
                break;
            }
        }
        end
    }

    fn cell_index_at_in(line: &str, col_offset: usize) -> usize {
        let pipes = line[..col_offset.min(line.len())]
            .chars()
            .filter(|&c| c == '|')
            .count();
        pipes.saturating_sub(1)
    }

    fn next_table_cell(
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
