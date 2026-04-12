use std::collections::HashMap;

use crate::markdown::LineKind;

/// A heading found in the document.
#[derive(Clone, Debug, PartialEq)]
pub struct HeadingInfo {
    /// Display-line index (0-based).
    pub line_idx: usize,
    /// Heading level (1–6).
    pub level: u8,
}

/// Three-state fold cycle, matching Emacs org-mode TAB behavior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FoldCycle {
    /// Only the heading line visible; body + subheadings hidden.
    Folded,
    /// Heading + direct child headings visible (children themselves folded).
    Children,
    /// Everything under this heading visible.
    ShowAll,
}

impl FoldCycle {
    fn next(self) -> Self {
        match self {
            FoldCycle::ShowAll => FoldCycle::Folded,
            FoldCycle::Folded => FoldCycle::Children,
            FoldCycle::Children => FoldCycle::ShowAll,
        }
    }

    /// When a heading has no child headings, skip the Children state.
    fn next_leaf(self) -> Self {
        match self {
            FoldCycle::ShowAll => FoldCycle::Folded,
            FoldCycle::Folded => FoldCycle::ShowAll,
            FoldCycle::Children => FoldCycle::ShowAll,
        }
    }
}

/// Outline fold/visibility state for the editor.
pub struct OutlineState {
    /// Per-heading fold state, keyed by heading line index.
    pub fold_states: HashMap<usize, FoldCycle>,
    /// Current global fold cycle (for S-TAB).
    pub global_cycle: FoldCycle,
}

impl OutlineState {
    pub fn new() -> Self {
        Self {
            fold_states: HashMap::new(),
            global_cycle: FoldCycle::ShowAll,
        }
    }

    /// Get the fold state for a heading (defaults to ShowAll).
    pub fn fold_state(&self, heading_line: usize) -> FoldCycle {
        self.fold_states
            .get(&heading_line)
            .copied()
            .unwrap_or(FoldCycle::ShowAll)
    }

    /// Cycle the fold state for the heading at `line_idx`.
    /// Returns the new state, or None if line_idx is not a heading.
    pub fn cycle_heading(
        &mut self,
        line_idx: usize,
        headings: &[HeadingInfo],
        line_count: usize,
    ) -> Option<FoldCycle> {
        let hi = headings.iter().find(|h| h.line_idx == line_idx)?;
        let has_children = has_child_headings(hi, headings, line_count);
        let current = self.fold_state(line_idx);
        let next = if has_children {
            current.next()
        } else {
            current.next_leaf()
        };
        self.fold_states.insert(line_idx, next);
        Some(next)
    }

    /// Global cycle: apply one fold state to all top-level headings.
    pub fn global_cycle(&mut self, headings: &[HeadingInfo]) {
        self.global_cycle = self.global_cycle.next();
        for h in headings {
            self.fold_states.insert(h.line_idx, self.global_cycle);
        }
    }

    /// Compute which lines are hidden given the current fold states.
    /// Returns a Vec<bool> of length `line_count` where true = hidden.
    pub fn compute_hidden_lines(
        &self,
        headings: &[HeadingInfo],
        line_count: usize,
    ) -> Vec<bool> {
        let mut hidden = vec![false; line_count];
        if headings.is_empty() || line_count == 0 {
            return hidden;
        }

        // Process each heading's fold state
        for (idx, hi) in headings.iter().enumerate() {
            let state = self.fold_state(hi.line_idx);
            if state == FoldCycle::ShowAll {
                continue;
            }

            let section_end = section_end_line(idx, headings, line_count);

            match state {
                FoldCycle::Folded => {
                    // Hide everything after the heading line until section end
                    for line in (hi.line_idx + 1)..section_end {
                        hidden[line] = true;
                    }
                }
                FoldCycle::Children => {
                    // Show direct child headings, hide everything else.
                    // Direct child headings are implicitly folded unless the
                    // user has explicitly expanded them.
                    let mut line = hi.line_idx + 1;
                    while line < section_end {
                        if let Some(child) = headings.iter().find(|h| h.line_idx == line) {
                            if child.level == hi.level + 1 {
                                // Direct child heading — always show it
                                line += 1;
                                continue;
                            }
                        }
                        // Check if this line is inside a direct child's section
                        if let Some(parent_child) = find_direct_child_parent(
                            line, hi, headings, line_count,
                        ) {
                            // Line is under a direct child — only show if user
                            // has explicitly set that child to ShowAll.
                            let explicitly_open = self
                                .fold_states
                                .get(&parent_child.line_idx)
                                .copied()
                                == Some(FoldCycle::ShowAll);
                            if explicitly_open {
                                line += 1;
                                continue;
                            }
                            hidden[line] = true;
                            line += 1;
                            continue;
                        }
                        // Body text directly under this heading — hide
                        hidden[line] = true;
                        line += 1;
                    }
                }
                FoldCycle::ShowAll => unreachable!(),
            }
        }

        hidden
    }
}

// ─── Heading tree helpers ────────────────────────────────────────────────────

/// Extract heading info from display-map line kinds.
pub fn extract_headings(line_kinds: &[LineKind]) -> Vec<HeadingInfo> {
    line_kinds
        .iter()
        .enumerate()
        .filter_map(|(idx, kind)| match kind {
            LineKind::Heading(level) => Some(HeadingInfo {
                line_idx: idx,
                level: *level,
            }),
            _ => None,
        })
        .collect()
}

/// End line (exclusive) of the section owned by heading at `heading_idx`.
/// A section runs from the heading to the next heading of same or higher level.
pub fn section_end_line(
    heading_idx: usize,
    headings: &[HeadingInfo],
    line_count: usize,
) -> usize {
    let hi = &headings[heading_idx];
    for next in &headings[(heading_idx + 1)..] {
        if next.level <= hi.level {
            return next.line_idx;
        }
    }
    line_count
}

/// Check if a heading has any child headings (deeper level within its section).
fn has_child_headings(
    hi: &HeadingInfo,
    headings: &[HeadingInfo],
    line_count: usize,
) -> bool {
    let idx = headings.iter().position(|h| h.line_idx == hi.line_idx).unwrap();
    let end = section_end_line(idx, headings, line_count);
    headings.iter().any(|h| {
        h.line_idx > hi.line_idx && h.line_idx < end && h.level > hi.level
    })
}

/// Find the direct child heading that owns `line`, if any.
/// A direct child has level == parent.level + 1 and its section contains `line`.
fn find_direct_child_parent<'a>(
    line: usize,
    parent: &HeadingInfo,
    headings: &'a [HeadingInfo],
    line_count: usize,
) -> Option<&'a HeadingInfo> {
    for (idx, h) in headings.iter().enumerate() {
        if h.line_idx <= parent.line_idx {
            continue;
        }
        if h.level <= parent.level {
            break; // Past parent's section
        }
        if h.level == parent.level + 1 && line > h.line_idx {
            let child_end = section_end_line(idx, headings, line_count);
            if line < child_end {
                return Some(h);
            }
        }
    }
    None
}

// ─── Heading navigation ─────────────────────────────────────────────────────

/// Find the next heading after `line_idx` (any level).
pub fn next_heading(line_idx: usize, headings: &[HeadingInfo]) -> Option<&HeadingInfo> {
    headings.iter().find(|h| h.line_idx > line_idx)
}

/// Find the previous heading before `line_idx` (any level).
pub fn prev_heading(line_idx: usize, headings: &[HeadingInfo]) -> Option<&HeadingInfo> {
    headings.iter().rev().find(|h| h.line_idx < line_idx)
}

/// Find the parent heading of the heading at `line_idx`.
pub fn parent_heading(line_idx: usize, headings: &[HeadingInfo]) -> Option<&HeadingInfo> {
    let hi = headings.iter().find(|h| h.line_idx == line_idx)?;
    headings
        .iter()
        .rev()
        .find(|h| h.line_idx < line_idx && h.level < hi.level)
}

/// Find the heading that contains `line_idx` (the heading whose section includes this line).
pub fn heading_for_line(line_idx: usize, headings: &[HeadingInfo]) -> Option<&HeadingInfo> {
    // The containing heading is the last heading at or before line_idx
    headings.iter().rev().find(|h| h.line_idx <= line_idx)
}

/// Find the next sibling heading (same level under the same parent).
pub fn next_sibling(line_idx: usize, headings: &[HeadingInfo]) -> Option<&HeadingInfo> {
    let hi = headings.iter().find(|h| h.line_idx == line_idx)?;
    headings
        .iter()
        .find(|h| h.line_idx > line_idx && h.level == hi.level)
}

/// Find the previous sibling heading (same level under the same parent).
pub fn prev_sibling(line_idx: usize, headings: &[HeadingInfo]) -> Option<&HeadingInfo> {
    let hi = headings.iter().find(|h| h.line_idx == line_idx)?;
    headings
        .iter()
        .rev()
        .find(|h| h.line_idx < line_idx && h.level == hi.level)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::LineKind;

    fn make_kinds(specs: &[(&str, u8)]) -> Vec<LineKind> {
        // specs: ("h", level) for heading, ("n", 0) for normal
        specs
            .iter()
            .map(|(ty, lvl)| match *ty {
                "h" => LineKind::Heading(*lvl),
                _ => LineKind::Normal,
            })
            .collect()
    }

    #[test]
    fn test_extract_headings() {
        let kinds = make_kinds(&[("h", 1), ("n", 0), ("h", 2), ("n", 0), ("h", 1)]);
        let headings = extract_headings(&kinds);
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0], HeadingInfo { line_idx: 0, level: 1 });
        assert_eq!(headings[1], HeadingInfo { line_idx: 2, level: 2 });
        assert_eq!(headings[2], HeadingInfo { line_idx: 4, level: 1 });
    }

    #[test]
    fn test_section_end_line() {
        // H1, body, H2, body, H1
        let kinds = make_kinds(&[("h", 1), ("n", 0), ("h", 2), ("n", 0), ("h", 1)]);
        let headings = extract_headings(&kinds);
        assert_eq!(section_end_line(0, &headings, 5), 4); // H1 section ends at second H1
        assert_eq!(section_end_line(1, &headings, 5), 4); // H2 section ends at second H1
        assert_eq!(section_end_line(2, &headings, 5), 5); // last H1 goes to end
    }

    #[test]
    fn test_cycle_heading_leaf() {
        let kinds = make_kinds(&[("h", 1), ("n", 0), ("n", 0)]);
        let headings = extract_headings(&kinds);
        let mut outline = OutlineState::new();

        // ShowAll → Folded (skips Children for leaf)
        let s = outline.cycle_heading(0, &headings, 3);
        assert_eq!(s, Some(FoldCycle::Folded));

        // Folded → ShowAll
        let s = outline.cycle_heading(0, &headings, 3);
        assert_eq!(s, Some(FoldCycle::ShowAll));
    }

    #[test]
    fn test_cycle_heading_with_children() {
        // H1, body, H2, body
        let kinds = make_kinds(&[("h", 1), ("n", 0), ("h", 2), ("n", 0)]);
        let headings = extract_headings(&kinds);
        let mut outline = OutlineState::new();

        // ShowAll → Folded → Children → ShowAll
        let s = outline.cycle_heading(0, &headings, 4);
        assert_eq!(s, Some(FoldCycle::Folded));
        let s = outline.cycle_heading(0, &headings, 4);
        assert_eq!(s, Some(FoldCycle::Children));
        let s = outline.cycle_heading(0, &headings, 4);
        assert_eq!(s, Some(FoldCycle::ShowAll));
    }

    #[test]
    fn test_compute_hidden_folded() {
        // H1, body1, body2
        let kinds = make_kinds(&[("h", 1), ("n", 0), ("n", 0)]);
        let headings = extract_headings(&kinds);
        let mut outline = OutlineState::new();
        outline.fold_states.insert(0, FoldCycle::Folded);

        let hidden = outline.compute_hidden_lines(&headings, 3);
        assert_eq!(hidden, vec![false, true, true]);
    }

    #[test]
    fn test_compute_hidden_children() {
        // H1(0), body(1), H2(2), body(3), H2(4), body(5)
        let kinds = make_kinds(&[
            ("h", 1), ("n", 0), ("h", 2), ("n", 0), ("h", 2), ("n", 0),
        ]);
        let headings = extract_headings(&kinds);
        let mut outline = OutlineState::new();
        outline.fold_states.insert(0, FoldCycle::Children);

        let hidden = outline.compute_hidden_lines(&headings, 6);
        // H1 visible, body hidden, H2@2 visible, body@3 hidden, H2@4 visible, body@5 hidden
        assert_eq!(hidden, vec![false, true, false, true, false, true]);
    }

    #[test]
    fn test_global_cycle() {
        let kinds = make_kinds(&[("h", 1), ("n", 0), ("h", 1), ("n", 0)]);
        let headings = extract_headings(&kinds);
        let mut outline = OutlineState::new();

        outline.global_cycle(&headings);
        assert_eq!(outline.global_cycle, FoldCycle::Folded);
        assert_eq!(outline.fold_state(0), FoldCycle::Folded);
        assert_eq!(outline.fold_state(2), FoldCycle::Folded);
    }

    #[test]
    fn test_heading_navigation() {
        let kinds = make_kinds(&[("h", 1), ("n", 0), ("h", 2), ("n", 0), ("h", 1)]);
        let headings = extract_headings(&kinds);

        assert_eq!(next_heading(0, &headings).unwrap().line_idx, 2);
        assert_eq!(next_heading(2, &headings).unwrap().line_idx, 4);
        assert!(next_heading(4, &headings).is_none());

        assert_eq!(prev_heading(4, &headings).unwrap().line_idx, 2);
        assert_eq!(prev_heading(2, &headings).unwrap().line_idx, 0);
        assert!(prev_heading(0, &headings).is_none());

        assert_eq!(parent_heading(2, &headings).unwrap().line_idx, 0);
        assert!(parent_heading(0, &headings).is_none());
    }

    #[test]
    fn test_heading_for_line() {
        let kinds = make_kinds(&[("h", 1), ("n", 0), ("h", 2), ("n", 0)]);
        let headings = extract_headings(&kinds);

        assert_eq!(heading_for_line(0, &headings).unwrap().line_idx, 0);
        assert_eq!(heading_for_line(1, &headings).unwrap().line_idx, 0);
        assert_eq!(heading_for_line(2, &headings).unwrap().line_idx, 2);
        assert_eq!(heading_for_line(3, &headings).unwrap().line_idx, 2);
    }

    #[test]
    fn test_not_a_heading() {
        let kinds = make_kinds(&[("n", 0), ("n", 0)]);
        let headings = extract_headings(&kinds);
        let mut outline = OutlineState::new();
        assert!(outline.cycle_heading(0, &headings, 2).is_none());
    }

    #[test]
    fn test_nested_fold() {
        // H1(0), H2(1), body(2), H2(3), body(4)
        // Fold H1 → everything hidden
        let kinds = make_kinds(&[("h", 1), ("h", 2), ("n", 0), ("h", 2), ("n", 0)]);
        let headings = extract_headings(&kinds);
        let mut outline = OutlineState::new();
        outline.fold_states.insert(0, FoldCycle::Folded);

        let hidden = outline.compute_hidden_lines(&headings, 5);
        assert_eq!(hidden, vec![false, true, true, true, true]);
    }
}
