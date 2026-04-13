mod scrollbar;
mod view;

pub use view::PdfView;

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::*;
use mupdf::{Colorspace, Document, Matrix as MuMatrix};

const PAGE_GAP: f32 = 8.0;
pub(crate) const PADDING_Y: f32 = 16.0;
const BASE_WIDTH: f32 = 800.0;
/// Extra pages to render beyond the visible range for smooth scrolling
const BUFFER_PAGES: usize = 2;

/// Cached rendered page: PNG-encoded bytes ready for gpui.
pub struct RenderedPage {
    pub image: Arc<gpui::Image>,
    pub width: u32,
    pub height: u32,
}

/// Pre-computed layout info for each page (cheap — just bounds, no rendering).
struct PageLayout {
    height: f32,
    width: f32,
    /// Y offset from top of the document
    y_offset: f32,
    /// Scale factor from PDF coords to screen coords
    scale: f32,
}

/// An internal link on a page, with bounds in PDF coordinates.
#[derive(Clone)]
pub struct PageLink {
    /// Bounding rect in PDF page coordinates
    pub bounds: mupdf::Rect,
    /// Target page number (absolute)
    pub target_page: usize,
}

/// A table-of-contents entry extracted from the PDF outline.
#[derive(Clone, Debug)]
pub struct TocEntry {
    pub title: String,
    pub page: usize,
    /// Nesting depth (0 = top-level chapter)
    pub level: usize,
}

use crate::pdf::scrollbar::DragState;

/// State for an open PDF document.
pub struct PdfState {
    pub path: PathBuf,
    pub page_count: usize,
    pub scroll_offset: gpui::Pixels,
    pub zoom: f32,
    pub focus_handle: FocusHandle,
    page_cache: HashMap<usize, RenderedPage>,
    raw_bytes: Arc<Vec<u8>>,
    page_layouts: Vec<PageLayout>,
    pub total_height: f32,
    /// Internal links per page (only internal navigation links, not external URLs)
    page_links: HashMap<usize, Vec<PageLink>>,
    /// Pages currently being rendered on background threads
    pending_renders: HashSet<usize>,
    /// Scrollbar drag state (survives across frames)
    pub drag_state: Option<DragState>,
    /// Table of contents extracted from the PDF outline
    pub toc: Vec<TocEntry>,
    /// Per-page rotation in degrees (0, 90, 180, 270)
    pub page_rotations: HashMap<usize, i32>,
    /// Invert colors for night reading
    pub dark_mode: bool,
    /// Two-page spread mode
    pub spread_mode: bool,
}

impl PdfState {
    pub fn new(path: impl AsRef<Path>, cx: &mut Context<Self>) -> Result<Self, mupdf::Error> {
        let path = path.as_ref().to_path_buf();
        let raw_bytes = Arc::new(std::fs::read(&path).map_err(mupdf::Error::Io)?);
        let doc = Document::from_bytes(&raw_bytes, "")?;
        let page_count = doc.page_count()? as usize;
        let (page_layouts, total_height) = Self::compute_layouts(&doc, page_count, 1.0)?;
        let page_links = Self::extract_links(&doc, page_count);
        let toc = Self::extract_toc(&doc);

        Ok(Self {
            path,
            page_count,
            scroll_offset: px(0.),
            zoom: 1.0,
            focus_handle: cx.focus_handle(),
            page_cache: HashMap::new(),
            raw_bytes,
            page_layouts,
            total_height,
            page_links,
            pending_renders: HashSet::new(),
            drag_state: None,
            toc,
            page_rotations: HashMap::new(),
            dark_mode: false,
            spread_mode: false,
        })
    }

    /// Extract internal links from all pages.
    fn extract_links(doc: &Document, page_count: usize) -> HashMap<usize, Vec<PageLink>> {
        let mut all_links = HashMap::new();
        for i in 0..page_count {
            if let Ok(page) = doc.load_page(i as i32) {
                if let Ok(links) = page.links() {
                    let page_links: Vec<PageLink> = links
                        .filter_map(|link| {
                            link.dest.map(|dest| PageLink {
                                bounds: link.bounds,
                                target_page: dest.loc.page_number as usize,
                            })
                        })
                        .collect();
                    if !page_links.is_empty() {
                        all_links.insert(i, page_links);
                    }
                }
            }
        }
        all_links
    }

    /// Extract table of contents from the PDF outline tree.
    fn extract_toc(doc: &Document) -> Vec<TocEntry> {
        let outlines = match doc.outlines() {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };
        let mut entries = Vec::new();
        Self::flatten_outlines(&outlines, 0, &mut entries);
        entries
    }

    /// Recursively flatten the outline tree into a flat list with depth levels.
    fn flatten_outlines(
        outlines: &[mupdf::Outline],
        level: usize,
        entries: &mut Vec<TocEntry>,
    ) {
        for outline in outlines {
            let page = outline
                .dest
                .as_ref()
                .map(|d| d.loc.page_number as usize)
                .unwrap_or(0);
            entries.push(TocEntry {
                title: outline.title.clone(),
                page,
                level,
            });
            if !outline.down.is_empty() {
                Self::flatten_outlines(&outline.down, level + 1, entries);
            }
        }
    }

    /// Go to a 1-based page number (for user-facing commands).
    pub fn goto_page_number(&mut self, page_number: usize) {
        if page_number >= 1 && page_number <= self.page_count {
            self.goto_page(page_number - 1);
        }
    }

    /// Compute page positions from bounds (no rendering — just reads dimensions).
    fn compute_layouts(
        doc: &Document,
        page_count: usize,
        zoom: f32,
    ) -> Result<(Vec<PageLayout>, f32), mupdf::Error> {
        let mut layouts = Vec::with_capacity(page_count);
        let mut y = PADDING_Y;

        for i in 0..page_count {
            let page = doc.load_page(i as i32)?;
            let bounds = page.bounds()?;
            let scale = (BASE_WIDTH * zoom) / bounds.width();
            let w = bounds.width() * scale;
            let h = bounds.height() * scale;
            layouts.push(PageLayout {
                height: h,
                width: w,
                y_offset: y,
                scale,
            });
            y += h + PAGE_GAP;
        }

        Ok((layouts, y + PADDING_Y))
    }

    /// Returns the range of page indices visible in the viewport [first, last).
    pub fn visible_range(&self, viewport_height: f32) -> (usize, usize) {
        let top: f32 = self.scroll_offset.into();
        let bottom = top + viewport_height;

        let mut first = self.page_count;
        let mut last = 0usize;

        for (i, layout) in self.page_layouts.iter().enumerate() {
            let page_bottom = layout.y_offset + layout.height;
            if page_bottom >= top && layout.y_offset <= bottom {
                if i < first {
                    first = i;
                }
                last = i;
            }
        }

        if first > last {
            return (0, 0);
        }

        let first = first.saturating_sub(BUFFER_PAGES);
        let last = (last + BUFFER_PAGES).min(self.page_count - 1);
        (first, last + 1)
    }

    /// Page layout info (y_offset, width, height).
    pub fn page_layout(&self, page_index: usize) -> (f32, f32, f32) {
        let l = &self.page_layouts[page_index];
        (l.y_offset, l.width, l.height)
    }

    /// Hit-test a click at screen coordinates against links on a page.
    /// `click_x` and `click_y` are relative to the page image's top-left corner.
    pub fn hit_test_link(&self, page_index: usize, click_x: f32, click_y: f32) -> Option<usize> {
        let links = self.page_links.get(&page_index)?;
        let scale = self.page_layouts[page_index].scale;

        // Convert screen coords back to PDF page coords
        let pdf_x = click_x / scale;
        let pdf_y = click_y / scale;

        for link in links {
            if pdf_x >= link.bounds.x0
                && pdf_x <= link.bounds.x1
                && pdf_y >= link.bounds.y0
                && pdf_y <= link.bounds.y1
            {
                return Some(link.target_page);
            }
        }
        None
    }

    /// Scroll to put a specific page at the top of the viewport.
    pub fn goto_page(&mut self, page_index: usize) {
        if page_index < self.page_count {
            self.scroll_offset = px(self.page_layouts[page_index].y_offset - PADDING_Y);
        }
    }

    /// Get a cached rendered page (does NOT trigger rendering).
    pub fn get_cached_page(&self, page_index: usize) -> Option<&RenderedPage> {
        self.page_cache.get(&page_index)
    }

    /// Check if a page is currently being rendered in the background.
    pub fn is_pending(&self, page_index: usize) -> bool {
        self.pending_renders.contains(&page_index)
    }

    /// Request async rendering of pages that aren't cached or already pending.
    pub fn request_render_pages(&mut self, pages: &[usize], cx: &mut Context<Self>) {
        for &page_index in pages {
            if page_index >= self.page_count
                || self.page_cache.contains_key(&page_index)
                || self.pending_renders.contains(&page_index)
            {
                continue;
            }
            self.pending_renders.insert(page_index);

            let raw_bytes = self.raw_bytes.clone();
            let zoom = self.zoom;

            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        render_page_background(&raw_bytes, page_index, zoom)
                    })
                    .await;

                this.update(cx, |state, cx| {
                    state.pending_renders.remove(&page_index);
                    if let Some(rendered) = result {
                        state.page_cache.insert(page_index, rendered);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// Evict pages far from the visible range to limit memory usage.
    pub fn evict_distant_pages(&mut self, visible_first: usize, visible_last: usize) {
        let keep_start = visible_first.saturating_sub(BUFFER_PAGES * 2);
        let keep_end = (visible_last + BUFFER_PAGES * 2).min(self.page_count);
        self.page_cache.retain(|&idx, _| idx >= keep_start && idx < keep_end);
    }

    /// Invalidate cached pages and recompute layouts (e.g., after zoom change).
    pub fn invalidate_cache(&mut self) {
        self.page_cache.clear();
        self.pending_renders.clear();
        if let Ok(doc) = Document::from_bytes(&self.raw_bytes, "") {
            if let Ok((layouts, total)) =
                Self::compute_layouts(&doc, self.page_count, self.zoom)
            {
                self.page_layouts = layouts;
                self.total_height = total;
            }
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    pub fn max_scroll(&self, viewport_height: f32) -> gpui::Pixels {
        px((self.total_height - viewport_height).max(0.0))
    }

    /// Extract plain text from a page using mupdf's text extraction.
    pub fn extract_page_text(&self, page_index: usize) -> Option<String> {
        if page_index >= self.page_count {
            return None;
        }
        let doc = Document::from_bytes(&self.raw_bytes, "").ok()?;
        let page = doc.load_page(page_index as i32).ok()?;
        let tp = page.to_text_page(mupdf::TextPageFlags::empty()).ok()?;
        tp.to_text().ok()
    }

    /// Compute zoom factor to fit page width to the given viewport width.
    pub fn fit_width(&mut self, viewport_width: f32) {
        if self.page_count == 0 { return; }
        // The rendering uses BASE_WIDTH * zoom as the target width
        // So zoom = viewport_width / BASE_WIDTH (with some padding)
        let target = viewport_width - 40.0; // leave some margin
        self.zoom = (target / BASE_WIDTH).max(0.3).min(3.0);
        self.invalidate_cache();
    }

    /// Compute zoom factor to fit entire page in the given viewport.
    pub fn fit_page(&mut self, viewport_width: f32, viewport_height: f32) {
        if self.page_count == 0 { return; }
        if let Ok(doc) = Document::from_bytes(&self.raw_bytes, "") {
            if let Ok(page) = doc.load_page(0) {
                if let Ok(bounds) = page.bounds() {
                    let aspect = bounds.height() / bounds.width();
                    let target_w = viewport_width - 40.0;
                    let target_h = viewport_height - PADDING_Y * 2.0;
                    let zoom_w = target_w / BASE_WIDTH;
                    let zoom_h = target_h / (BASE_WIDTH * aspect);
                    self.zoom = zoom_w.min(zoom_h).max(0.3).min(3.0);
                    self.invalidate_cache();
                }
            }
        }
    }
}

/// Render a single page on a background thread. Standalone function (not on PdfState)
/// because mupdf::Document is not Send — we create it fresh from shared bytes.
fn render_page_background(
    raw_bytes: &[u8],
    page_index: usize,
    zoom: f32,
) -> Option<RenderedPage> {
    let doc = Document::from_bytes(raw_bytes, "").ok()?;
    let page = doc.load_page(page_index as i32).ok()?;
    let bounds = page.bounds().ok()?;

    let scale = (BASE_WIDTH * zoom) / bounds.width();
    let ctm = MuMatrix::new_scale(scale, scale);

    let pixmap = page.to_pixmap(&ctm, &Colorspace::device_rgb(), false, true).ok()?;
    let width = pixmap.width();
    let height = pixmap.height();

    let mut png_buf = Cursor::new(Vec::new());
    pixmap.write_to(&mut png_buf, mupdf::ImageFormat::PNG).ok()?;

    Some(RenderedPage {
        image: Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            png_buf.into_inner(),
        )),
        width,
        height,
    })
}
