use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::editor::EditorBuffer;
use crate::pane::ActiveItem;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ResourceKey {
    Scratch(PathBuf),
    Markdown(PathBuf),
    Pdf(PathBuf),
    Graph(PathBuf),
}

#[derive(Clone)]
pub(crate) enum BufferContent {
    Markdown(EditorBuffer),
    Pdf(PathBuf),
    Graph(PathBuf),
}

pub(crate) enum SecondaryContent {
    Item { item: ActiveItem },
    Backlinks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PdfLinkTarget {
    Page(usize),
    Annotation(String),
}

pub(crate) fn parse_pdf_link(target: &str) -> Option<(&str, PdfLinkTarget)> {
    let (file, fragment) = target.split_once('#')?;
    if !file.to_lowercase().ends_with(".pdf") {
        return None;
    }
    if let Some(page) = fragment.strip_prefix("page=") {
        return page
            .parse::<usize>()
            .ok()
            .filter(|page| *page > 0)
            .map(|page| (file, PdfLinkTarget::Page(page)));
    }
    fragment
        .strip_prefix("annotation=")
        .filter(|id| !id.is_empty())
        .map(|id| (file, PdfLinkTarget::Annotation(id.to_string())))
}

pub(crate) fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

pub(crate) fn unique_attachment_path(dir: &Path, filename: &OsStr) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }

    let original = Path::new(filename);
    let stem = original
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("attachment");
    let extension = original.extension().and_then(|extension| extension.to_str());

    for ix in 1.. {
        let filename = match extension {
            Some(extension) if !extension.is_empty() => format!("{}-{}.{}", stem, ix, extension),
            _ => format!("{}-{}", stem, ix),
        };
        let candidate = dir.join(filename);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("unbounded attachment filename search must return")
}

#[cfg(test)]
mod tests {
    use super::{PdfLinkTarget, parse_pdf_link};

    #[test]
    fn parses_pdf_page_and_annotation_links() {
        assert_eq!(
            parse_pdf_link("paper.pdf#page=12"),
            Some(("paper.pdf", PdfLinkTarget::Page(12)))
        );
        assert_eq!(
            parse_pdf_link("paper.pdf#annotation=memex:abc"),
            Some(("paper.pdf", PdfLinkTarget::Annotation("memex:abc".into())))
        );
        assert_eq!(parse_pdf_link("paper.pdf#page=0"), None);
        assert_eq!(parse_pdf_link("note#page=1"), None);
    }
}
