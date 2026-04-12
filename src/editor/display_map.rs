use gpui::Pixels;

use crate::markdown::{self, LineInfo, LineKind};

/// Cached display pipeline for the editor.
///
/// Caches parsed line info and precomputes cumulative Y positions
/// to enable virtual scrolling (only shaping visible lines).
pub struct DisplayMap {
    lines: Vec<CachedLine>,
    /// Version of the content when last parsed
    version: u64,
    /// Total document height in pixels
    total_height: Pixels,
    /// Padding used for Y position calculation
    padding: Pixels,
}

struct CachedLine {
    info: LineInfo,
    content_offset: usize,
    line_height: Pixels,
    /// Cumulative Y position (relative to document top, before scroll)
    y_offset: Pixels,
    /// Whether this line is hidden by outline folding.
    hidden: bool,
}

impl DisplayMap {
    pub fn new(padding: Pixels) -> Self {
        Self {
            lines: Vec::new(),
            version: 0,
            total_height: Pixels::ZERO,
            padding,
        }
    }

    /// Increment version to trigger re-parse on next update.
    pub fn invalidate(&mut self) {
        self.version += 1;
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// Re-parse the document and update cached lines.
    pub fn update(&mut self, content: &str) {
        let parsed = markdown::parse_document(content);
        let mut y = Pixels::ZERO;
        self.lines = parsed
            .into_iter()
            .map(|(info, offset)| {
                let line_height = info.kind.line_height();
                let cached = CachedLine {
                    info,
                    content_offset: offset,
                    line_height,
                    y_offset: y,
                    hidden: false,
                };
                y += line_height;
                cached
            })
            .collect();
        self.total_height = y;
    }

    /// Apply fold visibility and recompute Y offsets.
    /// `hidden[i]` == true means line i is hidden by outline folding.
    pub fn update_visibility(&mut self, hidden: &[bool]) {
        let mut y = Pixels::ZERO;
        for (i, line) in self.lines.iter_mut().enumerate() {
            line.hidden = hidden.get(i).copied().unwrap_or(false);
            line.y_offset = y;
            if !line.hidden {
                y += line.line_height;
            }
        }
        self.total_height = y;
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn total_height(&self) -> Pixels {
        self.total_height
    }

    pub fn line_info(&self, idx: usize) -> &LineInfo {
        &self.lines[idx].info
    }

    pub fn line_offset(&self, idx: usize) -> usize {
        self.lines[idx].content_offset
    }

    pub fn line_height(&self, idx: usize) -> Pixels {
        self.lines[idx].line_height
    }

    pub fn is_line_hidden(&self, idx: usize) -> bool {
        self.lines.get(idx).map(|l| l.hidden).unwrap_or(false)
    }

    /// Collect LineKind for all lines (used by outline to extract headings).
    pub fn line_kinds(&self) -> Vec<crate::markdown::LineKind> {
        self.lines.iter().map(|l| l.info.kind.clone()).collect()
    }

    /// Y position of a line relative to document top (before scroll applied).
    pub fn line_y(&self, idx: usize) -> Pixels {
        self.lines[idx].y_offset
    }

    /// Find line index containing the given byte offset.
    pub fn line_for_offset(&self, byte_offset: usize) -> usize {
        self.lines
            .iter()
            .rposition(|l| l.content_offset <= byte_offset)
            .unwrap_or(0)
    }

    /// Find the next visible line in a given direction.
    /// Returns `None` if no visible line exists in that direction.
    pub fn next_visible_line(&self, from_line: usize, forward: bool) -> Option<usize> {
        if forward {
            (from_line + 1..self.lines.len()).find(|&i| !self.lines[i].hidden)
        } else {
            (0..from_line).rev().find(|&i| !self.lines[i].hidden)
        }
    }

    /// Check if a byte offset falls within a hidden line.
    pub fn is_offset_hidden(&self, byte_offset: usize) -> bool {
        let line = self.line_for_offset(byte_offset);
        self.is_line_hidden(line)
    }

    /// Find the range of line indices visible in the viewport.
    /// Returns (first, last_exclusive) with overscan buffer.
    pub fn visible_range(
        &self,
        scroll_offset: Pixels,
        viewport_height: Pixels,
        overscan: usize,
    ) -> (usize, usize) {
        if self.lines.is_empty() {
            return (0, 0);
        }

        // Find first line whose bottom edge is below the viewport top
        let viewport_top = scroll_offset - self.padding;
        let first = self
            .lines
            .iter()
            .position(|l| l.y_offset + l.line_height > viewport_top)
            .unwrap_or(0)
            .saturating_sub(overscan);

        // Find first line whose top edge is below the viewport bottom
        let viewport_bottom = scroll_offset + viewport_height - self.padding;
        let last = self
            .lines
            .iter()
            .position(|l| l.y_offset > viewport_bottom)
            .unwrap_or(self.lines.len())
            .saturating_add(overscan)
            .min(self.lines.len());

        (first, last)
    }
}

// Extend LineKind with line height calculation used by display map
impl LineKind {
    pub(crate) fn line_height(&self) -> Pixels {
        let fs = self.display_font_size();
        match self {
            LineKind::Heading(_) => fs * 1.5,
            _ => fs * 1.6,
        }
    }

    pub(crate) fn display_font_size(&self) -> Pixels {
        match self {
            LineKind::Heading(1) => gpui::px(28.),
            LineKind::Heading(2) => gpui::px(24.),
            LineKind::Heading(3) => gpui::px(20.),
            LineKind::Heading(4) => gpui::px(18.),
            LineKind::Heading(_) => gpui::px(16.),
            LineKind::CodeBlock => gpui::px(14.),
            _ => gpui::px(15.),
        }
    }

    pub(crate) fn display_font_weight(&self) -> gpui::FontWeight {
        match self {
            LineKind::Heading(_) => gpui::FontWeight::BOLD,
            _ => gpui::FontWeight::NORMAL,
        }
    }
}
