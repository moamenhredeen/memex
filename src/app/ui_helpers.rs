use gpui::*;

/// Render a label with the search term highlighted in a distinct color.
pub(crate) fn render_highlighted_label(
    label: &str,
    query: &str,
    base_color: impl Into<Hsla> + Copy,
    highlight: u32,
) -> Div {
    let highlight_color = rgb(highlight);
    let base_hsla: Hsla = base_color.into();
    let label_lower = label.to_lowercase();
    let query_lower = query.to_lowercase();

    let mut container = div().text_size(px(13.)).flex().flex_row();
    let mut pos = 0;

    while pos < label.len() {
        if let Some(match_start) = label_lower[pos..].find(&query_lower) {
            let abs_start = pos + match_start;
            let abs_end = abs_start + query_lower.len();
            let abs_start = snap_to_char(label, abs_start, false);
            let abs_end = snap_to_char(label, abs_end, true);

            if abs_start > pos {
                container = container.child(
                    div()
                        .text_color(base_hsla)
                        .child(label[pos..abs_start].to_string()),
                );
            }
            container = container.child(
                div()
                    .text_color(highlight_color)
                    .font_weight(FontWeight::BOLD)
                    .child(label[abs_start..abs_end].to_string()),
            );
            pos = abs_end;
        } else {
            container =
                container.child(div().text_color(base_hsla).child(label[pos..].to_string()));
            break;
        }
    }

    container
}

fn snap_to_char(s: &str, idx: usize, ceil: bool) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    if s.is_char_boundary(idx) {
        return idx;
    }
    if ceil {
        let mut i = idx;
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        i
    } else {
        let mut i = idx;
        while i > 0 && !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

/// Extract a single-line snippet around the first match of `needle` in
/// `body`, truncated to roughly `radius` chars on each side. Case-
/// insensitive match. Returns `"…"` if nothing matches.
pub(crate) fn extract_snippet(body: &str, needle: &str, radius: usize) -> String {
    let lower = body.to_lowercase();
    let Some(pos) = lower.find(needle) else {
        return "…".to_string();
    };
    let start = body[..pos]
        .char_indices()
        .rev()
        .take(radius)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end_target = pos + needle.len() + radius;
    let mut end = end_target.min(body.len());
    while end < body.len() && !body.is_char_boundary(end) {
        end += 1;
    }
    let slice = &body[start..end];
    let slice = slice.lines().next().unwrap_or(slice);
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < body.len() { "…" } else { "" };
    format!("{}{}{}", prefix, slice.trim(), suffix)
}
