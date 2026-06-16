mod view;

pub use view::{PdfView, PdfViewEvent};

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use gpui::*;
use mupdf::color::AnnotationColor;
use mupdf::pdf::{AnnotationFlags, PdfAnnotationType, PdfDocument, PdfObject};
use mupdf::{Colorspace, Document, Matrix as MuMatrix, Point, Quad, Rect};

use crate::command::Command;
use crate::minibuffer::Candidate;
use crate::pane::{CommandOutcome, ItemAction};

pub(crate) const PAGE_GAP: f32 = 8.0;
pub(crate) const PADDING_Y: f32 = 16.0;
const BASE_WIDTH: f32 = 800.0;
/// Extra pages to render beyond the visible range for smooth scrolling
const BUFFER_PAGES: usize = 2;

/// Cached rendered page: PNG-encoded bytes ready for gpui.
pub struct RenderedPage {
    pub image: Arc<gpui::Image>,
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
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

/// A search hit: a highlighted quad on a specific page.
#[derive(Clone, Debug)]
pub struct SearchHit {
    /// Page index (0-based).
    pub page: usize,
    /// Bounding quad in PDF page coordinates.
    pub quad: mupdf::Quad,
}

#[derive(Clone, Debug)]
pub struct PdfTextSelection {
    pub page: usize,
    pub quads: Vec<Quad>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct PdfAnnotationInfo {
    pub page: usize,
    pub xref: i32,
    pub id: Option<String>,
    pub quads: Vec<Quad>,
    pub contents: String,
}

#[derive(Clone, Copy, Debug)]
struct SelectionDrag {
    page: usize,
    start: Point,
    current: Point,
    last_preview_at: Instant,
}

use crate::ui::{DragState, Scrollable};

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
    rendered_page_bounds: HashMap<usize, Bounds<Pixels>>,
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
    /// Extracted text per page — populated once on open, used for fast search.
    /// None = extraction not yet complete.
    text_cache: Option<Vec<String>>,
    /// Current search query (empty = no active search).
    pub search_query: String,
    /// All search hits across the document.
    pub search_hits: Vec<SearchHit>,
    /// Index into `search_hits` for the currently focused match.
    pub search_current: usize,
    /// Cached context snippets for the minibuffer (page_index, snippet).
    pub search_preview: Vec<(usize, String)>,
    /// Whether a background search is running.
    pub search_pending: bool,
    /// Monotonic generation counter for debouncing — only the latest search applies.
    search_generation: u64,
    pub text_selection: Option<PdfTextSelection>,
    pub annotations: Vec<PdfAnnotationInfo>,
    pub selected_annotation: Option<(usize, i32)>,
    selection_drag: Option<SelectionDrag>,
    render_generation: u64,
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

        let annotations = load_annotations(&path).unwrap_or_default();
        Ok(Self {
            path,
            page_count,
            scroll_offset: px(0.),
            zoom: 1.0,
            focus_handle: cx.focus_handle(),
            page_cache: HashMap::new(),
            raw_bytes,
            page_layouts,
            rendered_page_bounds: HashMap::new(),
            total_height,
            page_links,
            pending_renders: HashSet::new(),
            drag_state: None,
            toc,
            page_rotations: HashMap::new(),
            dark_mode: false,
            spread_mode: false,
            text_cache: None,
            search_query: String::new(),
            search_hits: Vec::new(),
            search_current: 0,
            search_preview: Vec::new(),
            search_pending: false,
            search_generation: 0,
            text_selection: None,
            annotations,
            selected_annotation: None,
            selection_drag: None,
            render_generation: 0,
        })
    }

    pub fn begin_text_selection(&mut self, page: usize, point: Point) {
        self.text_selection = None;
        self.selected_annotation = None;
        self.selection_drag = Some(SelectionDrag {
            page,
            start: point,
            current: point,
            last_preview_at: Instant::now() - Duration::from_millis(50),
        });
    }

    pub fn update_text_selection(&mut self, page: usize, point: Point) -> Result<(), String> {
        let Some(mut drag) = self.selection_drag else {
            return Ok(());
        };
        if drag.page != page {
            return Ok(());
        }
        drag.current = point;
        let now = Instant::now();
        let should_preview = now.duration_since(drag.last_preview_at) >= Duration::from_millis(24);
        if should_preview {
            drag.last_preview_at = now;
        }
        self.selection_drag = Some(drag);
        if should_preview
            && ((drag.start.x - point.x).abs() >= 2.0 || (drag.start.y - point.y).abs() >= 2.0)
        {
            self.text_selection = extract_selection(&self.raw_bytes, page, drag.start, point)?;
        }
        Ok(())
    }

    pub fn finish_text_selection(&mut self) -> Result<bool, String> {
        let Some(drag) = self.selection_drag.take() else {
            return Ok(false);
        };
        if (drag.start.x - drag.current.x).abs() < 2.0
            && (drag.start.y - drag.current.y).abs() < 2.0
        {
            self.selected_annotation = self.hit_test_annotation(drag.page, drag.current);
            return Ok(self.selected_annotation.is_some());
        }
        let selection = extract_selection(&self.raw_bytes, drag.page, drag.start, drag.current)?;
        let selected = selection.is_some();
        self.text_selection = selection;
        Ok(selected)
    }

    pub fn clear_selection(&mut self) {
        self.selection_drag = None;
        self.text_selection = None;
        self.selected_annotation = None;
    }

    pub fn screen_to_page_point(
        &self,
        page: usize,
        viewport_width: f32,
        position: gpui::Point<Pixels>,
    ) -> Option<Point> {
        if let Some(bounds) = self.rendered_page_bounds.get(&page) {
            let x = f32::from(position.x - bounds.origin.x);
            let y = f32::from(position.y - bounds.origin.y);
            if x < 0.0
                || y < 0.0
                || x > f32::from(bounds.size.width)
                || y > f32::from(bounds.size.height)
            {
                return None;
            }
            let scale = self.page_scale(page);
            return Some(Point::new(x / scale, y / scale));
        }
        let (page_y, page_width, page_height) = self.page_layout(page);
        let page_x = (viewport_width - page_width) / 2.0;
        let x = f32::from(position.x) - page_x;
        let y = f32::from(position.y) + f32::from(self.scroll_offset) - page_y;
        if x < 0.0 || y < 0.0 || x > page_width || y > page_height {
            return None;
        }
        let scale = self.page_scale(page);
        Some(Point::new(x / scale, y / scale))
    }

    pub fn set_rendered_page_bounds(
        &mut self,
        bounds: impl IntoIterator<Item = (usize, Bounds<Pixels>)>,
    ) {
        self.rendered_page_bounds.clear();
        self.rendered_page_bounds.extend(bounds);
    }

    pub fn screen_to_document_point(
        &self,
        viewport_width: f32,
        position: gpui::Point<Pixels>,
    ) -> Option<(usize, Point)> {
        self.page_layouts.iter().enumerate().find_map(|(page, _)| {
            self.screen_to_page_point(page, viewport_width, position)
                .map(|point| (page, point))
        })
    }

    pub fn hit_test_link_point(&self, page: usize, point: Point) -> Option<usize> {
        self.page_links
            .get(&page)?
            .iter()
            .find(|link| link.bounds.contains(point.x, point.y))
            .map(|link| link.target_page)
    }

    pub fn selection_quads_for_page(&self, page: usize) -> Vec<Quad> {
        self.text_selection
            .as_ref()
            .filter(|s| s.page == page)
            .map(|s| s.quads.clone())
            .unwrap_or_default()
    }

    pub fn selected_annotation_quads_for_page(&self, page: usize) -> Vec<Quad> {
        let Some(selected) = self.selected_annotation else {
            return Vec::new();
        };
        self.annotations
            .iter()
            .find(|a| (a.page, a.xref) == selected && a.page == page)
            .map(|a| a.quads.clone())
            .unwrap_or_default()
    }

    fn hit_test_annotation(&self, page: usize, point: Point) -> Option<(usize, i32)> {
        self.annotations
            .iter()
            .rev()
            .find(|annotation| {
                annotation.page == page
                    && annotation
                        .quads
                        .iter()
                        .any(|q| Rect::from(q.clone()).contains(point.x, point.y))
            })
            .map(|annotation| (annotation.page, annotation.xref))
    }

    fn reload_annotations(&mut self) -> Result<(), String> {
        self.raw_bytes = Arc::new(fs::read(&self.path).map_err(|e| e.to_string())?);
        self.annotations = load_annotations(&self.path)?;
        self.render_generation = self.render_generation.wrapping_add(1);
        Ok(())
    }

    fn create_highlight(&mut self) -> Result<String, String> {
        let selection = self.text_selection.clone().ok_or("Select PDF text first")?;
        let id = format!("memex:{}", crate::vault::id::generate());
        mutate_pdf_atomically(&self.path, |doc| {
            let mut page = doc
                .load_pdf_page(selection.page as i32)
                .map_err(err_string)?;
            let mut annotation = page
                .add_highlight_annotation(selection.quads.clone())
                .map_err(err_string)?;
            annotation.set_author("Memex").map_err(err_string)?;
            annotation
                .set_contents(&selection.text)
                .map_err(err_string)?;
            annotation
                .set_color(AnnotationColor::Rgb {
                    red: 1.0,
                    green: 1.0,
                    blue: 0.0,
                })
                .map_err(err_string)?;
            annotation.set_opacity(0.35).map_err(err_string)?;
            annotation
                .set_flags(AnnotationFlags::IS_PRINT)
                .map_err(err_string)?;
            annotation
                .object()
                .dict_put("NM", PdfObject::new_string(&id).map_err(err_string)?)
                .map_err(err_string)?;
            annotation.update().map_err(err_string)?;
            page.update().map_err(err_string)?;
            Ok(())
        })?;
        self.reload_annotations()?;
        if let Some(annotation) = self
            .annotations
            .iter()
            .find(|a| a.id.as_deref() == Some(&id))
        {
            self.selected_annotation = Some((annotation.page, annotation.xref));
        }
        self.text_selection = None;
        Ok(id)
    }

    fn delete_selected_annotation(&mut self) -> Result<(), String> {
        let (page_index, xref) = self
            .selected_annotation
            .ok_or("Select a PDF annotation first")?;
        mutate_pdf_atomically(&self.path, |doc| {
            let mut page = doc.load_pdf_page(page_index as i32).map_err(err_string)?;
            let annotation = page
                .annotations()
                .find(|a| a.xref().ok() == Some(xref))
                .ok_or("Annotation no longer exists")?;
            page.delete_annotation(annotation).map_err(err_string)?;
            page.update().map_err(err_string)?;
            Ok(())
        })?;
        self.reload_annotations()?;
        self.selected_annotation = None;
        Ok(())
    }

    fn selected_annotation_link(&mut self) -> Result<Option<String>, String> {
        let Some((page_index, xref)) = self.selected_annotation else {
            return Ok(None);
        };
        let mut info = self
            .annotations
            .iter()
            .find(|a| (a.page, a.xref) == (page_index, xref))
            .cloned()
            .ok_or("Annotation no longer exists")?;
        if info.id.is_none() {
            let id = format!("memex:{}", crate::vault::id::generate());
            mutate_pdf_atomically(&self.path, |doc| {
                let page = doc.load_pdf_page(page_index as i32).map_err(err_string)?;
                let annotation = page
                    .annotations()
                    .find(|a| a.xref().ok() == Some(xref))
                    .ok_or("Annotation no longer exists")?;
                annotation
                    .object()
                    .dict_put("NM", PdfObject::new_string(&id).map_err(err_string)?)
                    .map_err(err_string)?;
                Ok(())
            })?;
            self.reload_annotations()?;
            self.selected_annotation = self
                .annotations
                .iter()
                .find(|annotation| annotation.id.as_deref() == Some(&id))
                .map(|annotation| (annotation.page, annotation.xref));
            info.id = Some(id);
        }
        let filename = self
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.pdf");
        let alias = annotation_alias(&info.contents);
        Ok(Some(format!(
            "[[{}#annotation={}|{}]]",
            filename,
            info.id.unwrap(),
            alias
        )))
    }

    pub fn goto_annotation(&mut self, id: &str) -> bool {
        let Some(annotation) = self
            .annotations
            .iter()
            .find(|a| a.id.as_deref() == Some(id))
        else {
            return false;
        };
        self.selected_annotation = Some((annotation.page, annotation.xref));
        let annotation_y = annotation
            .quads
            .first()
            .map(|quad| Rect::from(quad.clone()).y0 * self.page_scale(annotation.page))
            .unwrap_or(0.0);
        self.scroll_offset =
            px((self.page_layouts[annotation.page].y_offset + annotation_y - PADDING_Y).max(0.0));
        true
    }

    /// Extract text from all pages on a background thread for fast search.
    /// Called once after opening a PDF.
    pub fn extract_text_cache(&mut self, cx: &mut Context<Self>) {
        let raw_bytes = self.raw_bytes.clone();
        let page_count = self.page_count;

        cx.spawn(async move |this, cx| {
            let texts = cx
                .background_executor()
                .spawn(async move { extract_all_text(&raw_bytes, page_count) })
                .await;

            this.update(cx, |state, cx| {
                state.text_cache = Some(texts);
                cx.notify();
            })
            .ok();
        })
        .detach();
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
    fn flatten_outlines(outlines: &[mupdf::Outline], level: usize, entries: &mut Vec<TocEntry>) {
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

    /// Scale factor for a page (PDF coords → screen pixels).
    pub fn page_scale(&self, page_index: usize) -> f32 {
        self.page_layouts[page_index].scale
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
            let generation = self.render_generation;

            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { render_page_background(&raw_bytes, page_index, zoom) })
                    .await;

                this.update(cx, |state, cx| {
                    state.pending_renders.remove(&page_index);
                    if state.render_generation == generation
                        && let Some(rendered) = result
                    {
                        state.page_cache.insert(page_index, rendered);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// Re-render pages after an annotation write while retaining the previous
    /// images until their replacements are ready.
    fn refresh_render_pages(&mut self, pages: &[usize], cx: &mut Context<Self>) {
        for &page_index in pages {
            if page_index >= self.page_count {
                continue;
            }
            self.pending_renders.insert(page_index);
            let raw_bytes = self.raw_bytes.clone();
            let zoom = self.zoom;
            let generation = self.render_generation;
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { render_page_background(&raw_bytes, page_index, zoom) })
                    .await;
                this.update(cx, |state, cx| {
                    state.pending_renders.remove(&page_index);
                    if state.render_generation == generation
                        && let Some(rendered) = result
                    {
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
        self.page_cache
            .retain(|&idx, _| idx >= keep_start && idx < keep_end);
    }

    /// Invalidate cached pages and recompute layouts (e.g., after zoom change).
    pub fn invalidate_cache(&mut self) {
        self.render_generation = self.render_generation.wrapping_add(1);
        self.page_cache.clear();
        self.pending_renders.clear();
        if let Ok(doc) = Document::from_bytes(&self.raw_bytes, "") {
            if let Ok((layouts, total)) = Self::compute_layouts(&doc, self.page_count, self.zoom) {
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
        if self.page_count == 0 {
            return;
        }
        // The rendering uses BASE_WIDTH * zoom as the target width
        // So zoom = viewport_width / BASE_WIDTH (with some padding)
        let target = viewport_width - 40.0; // leave some margin
        self.zoom = (target / BASE_WIDTH).max(0.3).min(3.0);
        self.invalidate_cache();
    }

    /// Compute zoom factor to fit entire page in the given viewport.
    pub fn fit_page(&mut self, viewport_width: f32, viewport_height: f32) {
        if self.page_count == 0 {
            return;
        }
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

    /// Launch a debounced search. Uses the pre-extracted text cache for instant
    /// text matching, then only calls mupdf page.search() on a background thread
    /// for highlight quads.
    pub fn request_search(&mut self, query: &str, cx: &mut Context<Self>) {
        self.search_query = query.to_string();
        self.search_generation += 1;
        let search_gen = self.search_generation;

        if query.is_empty() {
            self.search_hits.clear();
            self.search_preview.clear();
            self.search_current = 0;
            self.search_pending = false;
            cx.notify();
            return;
        }

        // If text cache is ready, search it instantly (no background thread needed
        // for previews — only for highlight quads).
        if let Some(ref texts) = self.text_cache {
            let (previews, match_pages) = search_text_cache(texts, query, 50);
            self.search_preview = previews;
            cx.notify();

            // Spawn background thread only for highlight quads on matched pages
            self.search_pending = true;
            let raw_bytes = self.raw_bytes.clone();
            let needle = query.to_string();

            cx.spawn(async move |this, cx| {
                let hits = cx
                    .background_executor()
                    .spawn(async move { search_quads_for_pages(&raw_bytes, &match_pages, &needle) })
                    .await;

                this.update(cx, |state, cx| {
                    if state.search_generation == search_gen {
                        state.search_hits = hits;
                        state.search_current = 0;
                        state.search_pending = false;
                        state.scroll_to_current_match();
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        } else {
            // Text cache not ready yet — fall back to full background search
            self.search_pending = true;
            let raw_bytes = self.raw_bytes.clone();
            let page_count = self.page_count;
            let needle = query.to_string();

            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(150))
                    .await;

                let still_current = this
                    .update(cx, |state, _| state.search_generation == search_gen)
                    .unwrap_or(false);
                if !still_current {
                    return;
                }

                let (hits, previews) = cx
                    .background_executor()
                    .spawn(async move { search_background(&raw_bytes, page_count, &needle) })
                    .await;

                this.update(cx, |state, cx| {
                    if state.search_generation == search_gen {
                        state.search_hits = hits;
                        state.search_preview = previews;
                        state.search_current = 0;
                        state.search_pending = false;
                        state.scroll_to_current_match();
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// Jump to the next search match, wrapping around.
    pub fn search_next(&mut self) {
        if self.search_hits.is_empty() {
            return;
        }
        self.search_current = (self.search_current + 1) % self.search_hits.len();
        self.scroll_to_current_match();
    }

    /// Jump to the previous search match, wrapping around.
    pub fn search_prev(&mut self) {
        if self.search_hits.is_empty() {
            return;
        }
        if self.search_current == 0 {
            self.search_current = self.search_hits.len() - 1;
        } else {
            self.search_current -= 1;
        }
        self.scroll_to_current_match();
    }

    /// Scroll so the current match is visible.
    pub fn scroll_to_current_match(&mut self) {
        if let Some(hit) = self.search_hits.get(self.search_current) {
            self.goto_page(hit.page);
        }
    }

    /// Clear the active search.
    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.search_hits.clear();
        self.search_preview.clear();
        self.search_current = 0;
        self.search_pending = false;
        self.search_generation += 1;
    }

    /// Get search hits for a specific page (for rendering highlights).
    pub fn search_hits_for_page(&self, page_index: usize) -> Vec<(usize, &SearchHit)> {
        self.search_hits
            .iter()
            .enumerate()
            .filter(|(_, h)| h.page == page_index)
            .collect()
    }

    // ─── PaneItem interface ─────────────────────────────────────────────────

    /// Commands for the command palette when PDF is the active item.
    pub fn commands() -> Vec<Command> {
        vec![
            Command {
                id: "pdf-toc",
                name: "PDF: Table of Contents",
                description: "Browse and jump to table of contents entries",
                aliases: &["toc", "outline", "contents"],
                binding: Some("o"),
            },
            Command {
                id: "pdf-goto-page",
                name: "PDF: Go to Page",
                description: "Jump to a specific page number",
                aliases: &["goto-page", "page"],
                binding: Some("P"),
            },
            Command {
                id: "pdf-bookmarks",
                name: "PDF: Bookmarks",
                description: "Browse PDF bookmarks (outline entries)",
                aliases: &["bookmarks"],
                binding: None,
            },
            Command {
                id: "pdf-fit-width",
                name: "PDF: Fit Width",
                description: "Zoom to fit page width to viewport",
                aliases: &["fit-width"],
                binding: Some("w"),
            },
            Command {
                id: "pdf-fit-page",
                name: "PDF: Fit Page",
                description: "Zoom to fit entire page in viewport",
                aliases: &["fit-page"],
                binding: Some("W"),
            },
            Command {
                id: "pdf-rotate-cw",
                name: "PDF: Rotate Clockwise",
                description: "Rotate current page 90° clockwise",
                aliases: &["rotate-cw", "rotate"],
                binding: Some("r"),
            },
            Command {
                id: "pdf-rotate-ccw",
                name: "PDF: Rotate Counter-Clockwise",
                description: "Rotate current page 90° counter-clockwise",
                aliases: &["rotate-ccw"],
                binding: Some("R"),
            },
            Command {
                id: "pdf-dark-mode",
                name: "PDF: Toggle Dark Mode",
                description: "Invert colors for night reading",
                aliases: &["dark-mode", "invert"],
                binding: None,
            },
            Command {
                id: "pdf-two-page",
                name: "PDF: Two-Page Spread",
                description: "Toggle side-by-side two-page view",
                aliases: &["spread", "two-page"],
                binding: None,
            },
            Command {
                id: "pdf-highlight-selection",
                name: "PDF: Highlight Selection",
                description: "Save the selected text as a native PDF highlight",
                aliases: &["highlight", "annotate"],
                binding: Some("h"),
            },
            Command {
                id: "pdf-delete-annotation",
                name: "PDF: Delete Annotation",
                description: "Delete the selected PDF annotation",
                aliases: &["delete-annotation"],
                binding: Some("x"),
            },
            Command {
                id: "pdf-clear-selection",
                name: "PDF: Clear Selection",
                description: "Clear the current text or annotation selection",
                aliases: &[],
                binding: Some("Escape"),
            },
            Command {
                id: "pdf-copy-link",
                name: "PDF: Copy Link",
                description: "Copy a link to the selected annotation or current page",
                aliases: &["copy-link", "yank-link"],
                binding: Some("y"),
            },
            Command {
                id: "pdf-extract-text",
                name: "PDF: Extract Page Text",
                description: "Copy text from current page to clipboard",
                aliases: &["extract-text"],
                binding: Some("Y"),
            },
            Command {
                id: "pdf-scroll-down",
                name: "PDF: Scroll Down",
                description: "Scroll PDF down one step",
                aliases: &[],
                binding: Some("j"),
            },
            Command {
                id: "pdf-scroll-up",
                name: "PDF: Scroll Up",
                description: "Scroll PDF up one step",
                aliases: &[],
                binding: Some("k"),
            },
            Command {
                id: "pdf-half-page-down",
                name: "PDF: Half Page Down",
                description: "Scroll PDF down half a page",
                aliases: &[],
                binding: Some("Ctrl-d"),
            },
            Command {
                id: "pdf-half-page-up",
                name: "PDF: Half Page Up",
                description: "Scroll PDF up half a page",
                aliases: &[],
                binding: Some("Ctrl-u"),
            },
            Command {
                id: "pdf-zoom-in",
                name: "PDF: Zoom In",
                description: "Increase PDF zoom level",
                aliases: &[],
                binding: Some("+"),
            },
            Command {
                id: "pdf-zoom-out",
                name: "PDF: Zoom Out",
                description: "Decrease PDF zoom level",
                aliases: &[],
                binding: Some("-"),
            },
            Command {
                id: "pdf-goto-first",
                name: "PDF: Go to First Page",
                description: "Jump to the first page",
                aliases: &[],
                binding: Some("g"),
            },
            Command {
                id: "pdf-goto-last",
                name: "PDF: Go to Last Page",
                description: "Jump to the last page",
                aliases: &[],
                binding: Some("G"),
            },
            Command {
                id: "pdf-search",
                name: "PDF: Search Text",
                description: "Search for text across all pages",
                aliases: &["search", "find"],
                binding: Some("/"),
            },
            Command {
                id: "pdf-search-next",
                name: "PDF: Next Match",
                description: "Jump to the next search match",
                aliases: &["next-match"],
                binding: Some("n"),
            },
            Command {
                id: "pdf-search-prev",
                name: "PDF: Previous Match",
                description: "Jump to the previous search match",
                aliases: &["prev-match"],
                binding: Some("N"),
            },
        ]
    }

    /// Execute a PDF command, returning actions for the app shell.
    pub fn execute_command(
        &mut self,
        cmd_id: &str,
        viewport: (f32, f32),
        _vim_enabled: bool,
        cx: &mut Context<Self>,
    ) -> CommandOutcome {
        let (vw, vh) = viewport;
        let actions = match cmd_id {
            "pdf-toc" | "pdf-bookmarks" => {
                vec![ItemAction::ActivateDelegate {
                    id: "pdf-toc".into(),
                    prompt: "TOC:".into(),
                    highlight_input: false,
                }]
            }
            "pdf-goto-page" => {
                let prompt = format!("Go to page (1-{}):", self.page_count);
                vec![ItemAction::ActivateDelegate {
                    id: "pdf-goto-page".into(),
                    prompt,
                    highlight_input: false,
                }]
            }
            "pdf-fit-width" => {
                self.fit_width(vw);
                cx.notify();
                vec![]
            }
            "pdf-fit-page" => {
                self.fit_page(vw, vh);
                cx.notify();
                vec![]
            }
            "pdf-rotate-cw" => {
                let (first, _) = self.visible_range(vh);
                let rotation = self.page_rotations.entry(first).or_insert(0);
                *rotation = (*rotation + 90) % 360;
                self.invalidate_cache();
                cx.notify();
                vec![ItemAction::SetMessage("Rotated clockwise".into())]
            }
            "pdf-rotate-ccw" => {
                let (first, _) = self.visible_range(vh);
                let rotation = self.page_rotations.entry(first).or_insert(0);
                *rotation = (*rotation + 270) % 360;
                self.invalidate_cache();
                cx.notify();
                vec![ItemAction::SetMessage("Rotated counter-clockwise".into())]
            }
            "pdf-dark-mode" => {
                self.dark_mode = !self.dark_mode;
                self.invalidate_cache();
                cx.notify();
                let mode = if self.dark_mode { "on" } else { "off" };
                vec![ItemAction::SetMessage(format!("Dark mode {}", mode))]
            }
            "pdf-two-page" => {
                self.spread_mode = !self.spread_mode;
                cx.notify();
                let mode = if self.spread_mode { "on" } else { "off" };
                vec![ItemAction::SetMessage(format!("Two-page spread {}", mode))]
            }
            "pdf-highlight-selection" => match self.create_highlight() {
                Ok(_) => {
                    if let Some((page, _)) = self.selected_annotation {
                        self.refresh_render_pages(&[page], cx);
                    }
                    cx.notify();
                    vec![ItemAction::SetMessage("PDF highlight saved".into())]
                }
                Err(error) => vec![ItemAction::SetMessage(error)],
            },
            "pdf-delete-annotation" => {
                let page = self.selected_annotation.map(|(page, _)| page);
                match self.delete_selected_annotation() {
                    Ok(()) => {
                        if let Some(page) = page {
                            self.refresh_render_pages(&[page], cx);
                        }
                        cx.notify();
                        vec![ItemAction::SetMessage("PDF annotation deleted".into())]
                    }
                    Err(error) => vec![ItemAction::SetMessage(error)],
                }
            }
            "pdf-clear-selection" => {
                self.clear_selection();
                cx.notify();
                vec![]
            }
            "pdf-copy-link" => match self.selected_annotation_link() {
                Ok(Some(link)) => vec![
                    ItemAction::Yank(link.clone()),
                    ItemAction::SetMessage(format!("Copied: {}", link)),
                ],
                Ok(None) => {
                    let (first, _) = self.visible_range(vh);
                    let filename = self
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file.pdf");
                    let link = format!("[[{}#page={}]]", filename, first + 1);
                    vec![
                        ItemAction::Yank(link.clone()),
                        ItemAction::SetMessage(format!("Copied: {}", link)),
                    ]
                }
                Err(error) => vec![ItemAction::SetMessage(error)],
            },
            "pdf-extract-text" => {
                let (first, _) = self.visible_range(vh);
                match self.extract_page_text(first) {
                    Some(text) if !text.is_empty() => {
                        vec![
                            ItemAction::WriteClipboard(text),
                            ItemAction::SetMessage(format!("Copied text from page {}", first + 1)),
                        ]
                    }
                    _ => {
                        vec![ItemAction::SetMessage("No text on this page".into())]
                    }
                }
            }
            "pdf-scroll-down" => {
                let max = self.max_scroll(vh);
                self.scroll_offset = (self.scroll_offset + px(60.)).min(max);
                cx.notify();
                vec![]
            }
            "pdf-scroll-up" => {
                self.scroll_offset = (self.scroll_offset - px(60.)).max(px(0.));
                cx.notify();
                vec![]
            }
            "pdf-half-page-down" => {
                let max = self.max_scroll(vh);
                self.scroll_offset = (self.scroll_offset + px(400.)).min(max);
                cx.notify();
                vec![]
            }
            "pdf-half-page-up" => {
                self.scroll_offset = (self.scroll_offset - px(400.)).max(px(0.));
                cx.notify();
                vec![]
            }
            "pdf-zoom-in" => {
                self.zoom = (self.zoom + 0.1).min(3.0);
                self.invalidate_cache();
                cx.notify();
                vec![]
            }
            "pdf-zoom-out" => {
                self.zoom = (self.zoom - 0.1).max(0.3);
                self.invalidate_cache();
                cx.notify();
                vec![]
            }
            "pdf-goto-first" => {
                self.scroll_offset = px(0.);
                cx.notify();
                vec![]
            }
            "pdf-goto-last" => {
                let max = self.max_scroll(vh);
                self.scroll_offset = max;
                cx.notify();
                vec![]
            }
            "pdf-search" => {
                self.clear_search();
                cx.notify();
                vec![ItemAction::ActivateDelegate {
                    id: "pdf-search".into(),
                    prompt: "Search:".into(),
                    highlight_input: true,
                }]
            }
            "pdf-search-next" => {
                self.search_next();
                cx.notify();
                let hits = self.search_hits.len();
                let cur = self.search_current;
                if hits > 0 {
                    vec![ItemAction::SetMessage(format!(
                        "Match {}/{}",
                        cur + 1,
                        hits
                    ))]
                } else {
                    vec![]
                }
            }
            "pdf-search-prev" => {
                self.search_prev();
                cx.notify();
                let hits = self.search_hits.len();
                let cur = self.search_current;
                if hits > 0 {
                    vec![ItemAction::SetMessage(format!(
                        "Match {}/{}",
                        cur + 1,
                        hits
                    ))]
                } else {
                    vec![]
                }
            }
            _ => return CommandOutcome::Unhandled,
        };
        CommandOutcome::handled(actions)
    }

    /// Get candidates for a PDF-owned minibuffer delegate.
    pub fn get_candidates(&self, delegate_id: &str, input: &str) -> Vec<Candidate> {
        match delegate_id {
            "pdf-toc" => self.toc_candidates(input),
            "pdf-goto-page" => self.goto_page_candidates(input),
            "pdf-search" => self.search_candidates(),
            _ => vec![],
        }
    }

    /// Handle confirm for a PDF-owned minibuffer delegate.
    pub fn handle_confirm(
        &mut self,
        delegate_id: &str,
        input: &str,
        candidate: Option<&Candidate>,
        cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        match delegate_id {
            "pdf-toc" => {
                if let Some(c) = candidate {
                    if let Ok(page) = c.data.parse::<usize>() {
                        self.goto_page(page);
                        cx.notify();
                    }
                }
                vec![ItemAction::Dismiss]
            }
            "pdf-goto-page" => {
                let page_str = candidate.map(|c| c.data.as_str()).unwrap_or(input);
                if let Ok(page_num) = page_str.trim().parse::<usize>() {
                    self.goto_page_number(page_num);
                    cx.notify();
                    vec![
                        ItemAction::Dismiss,
                        ItemAction::SetMessage(format!("Page {}", page_num)),
                    ]
                } else {
                    vec![ItemAction::SetMessage("Invalid page number".into())]
                }
            }
            "pdf-search" => {
                let query = input.to_string();
                let selected_page = candidate.and_then(|c| c.data.parse::<usize>().ok());
                if query.is_empty() {
                    self.clear_search();
                    cx.notify();
                    return vec![ItemAction::Dismiss];
                }
                if let Some(page) = selected_page {
                    self.goto_page(page);
                } else if !self.search_hits.is_empty() {
                    self.scroll_to_current_match();
                }
                cx.notify();
                let total = self.search_preview.len();
                let msg = if total > 0 {
                    format!("{} matches", total)
                } else {
                    format!("No matches for '{}'", query)
                };
                vec![ItemAction::Dismiss, ItemAction::SetMessage(msg)]
            }
            _ => vec![],
        }
    }

    /// Called when minibuffer input changes for a PDF delegate.
    pub fn on_input_changed(&mut self, delegate_id: &str, input: &str, cx: &mut Context<Self>) {
        if delegate_id == "pdf-search" {
            let query = input.trim().to_string();
            self.request_search(&query, cx);
        }
    }

    // ─── Private candidate builders ─────────────────────────────────────

    const MAX_CANDIDATES: usize = 15;

    fn toc_candidates(&self, query: &str) -> Vec<Candidate> {
        if self.toc.is_empty() {
            return vec![Candidate {
                label: "(No table of contents)".to_string(),
                detail: None,
                is_action: false,
                data: String::new(),
            }];
        }

        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, &TocEntry)> = self
            .toc
            .iter()
            .filter_map(|entry| {
                if query.is_empty() {
                    Some((0, entry))
                } else {
                    matcher.fuzzy_match(&entry.title, query).map(|s| (s, entry))
                }
            })
            .collect();

        if !query.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0));
        }

        scored
            .into_iter()
            .take(Self::MAX_CANDIDATES)
            .map(|(_, entry)| {
                let indent = "  ".repeat(entry.level);
                Candidate {
                    label: format!("{}{}", indent, entry.title),
                    detail: Some(format!("Page {}", entry.page + 1)),
                    is_action: false,
                    data: entry.page.to_string(),
                }
            })
            .collect()
    }

    fn goto_page_candidates(&self, input: &str) -> Vec<Candidate> {
        let input = input.trim();
        if input.is_empty() {
            return Vec::new();
        }
        if let Ok(num) = input.parse::<usize>() {
            if num >= 1 && num <= self.page_count {
                return vec![Candidate {
                    label: format!("Page {}", num),
                    detail: Some(format!("of {}", self.page_count)),
                    is_action: false,
                    data: num.to_string(),
                }];
            }
        }
        Vec::new()
    }

    fn search_candidates(&self) -> Vec<Candidate> {
        self.search_preview
            .iter()
            .map(|(page, snippet)| Candidate {
                label: format!("p{}: {}", page + 1, snippet),
                detail: None,
                is_action: false,
                data: page.to_string(),
            })
            .collect()
    }
}

impl Scrollable for PdfState {
    fn total_height(&self) -> f32 {
        self.total_height
    }
    fn scroll_offset(&self) -> Pixels {
        self.scroll_offset
    }
    fn set_scroll_offset(&mut self, offset: Pixels) {
        self.scroll_offset = offset;
    }
    fn drag_state(&self) -> Option<DragState> {
        self.drag_state
    }
    fn set_drag_state(&mut self, drag: Option<DragState>) {
        self.drag_state = drag;
    }
}

fn err_string(error: mupdf::Error) -> String {
    error.to_string()
}

fn annotation_alias(contents: &str) -> String {
    let compact = contents.split_whitespace().collect::<Vec<_>>().join(" ");
    let escaped = compact.replace('|', "-").replace("]]", "] ]");
    if escaped.is_empty() {
        "PDF highlight".into()
    } else {
        escaped.chars().take(120).collect()
    }
}

fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && a.x1 > b.x0 && a.y0 < b.y1 && a.y1 > b.y0
}

fn extract_selection(
    raw_bytes: &[u8],
    page_index: usize,
    start: Point,
    end: Point,
) -> Result<Option<PdfTextSelection>, String> {
    let doc = Document::from_bytes(raw_bytes, "").map_err(err_string)?;
    let page = doc.load_page(page_index as i32).map_err(err_string)?;
    let mut text_page = page
        .to_text_page(mupdf::TextPageFlags::empty())
        .map_err(err_string)?;
    let mut capacity = 64usize;
    let quads = loop {
        let buffer = vec![Quad::from(Rect::default()); capacity];
        let count = text_page
            .highlight_selection(start, end, &buffer)
            .map_err(err_string)?;
        if count <= 0 {
            return Ok(None);
        }
        if count as usize <= capacity {
            break buffer.into_iter().take(count as usize).collect::<Vec<_>>();
        }
        capacity = (count as usize).next_power_of_two().min(4096);
        if capacity >= 4096 {
            return Err("PDF text selection is too large".into());
        }
    };

    let mut lines = Vec::new();
    for block in text_page.blocks() {
        for line in block.lines() {
            let text: String = line
                .chars()
                .filter_map(|ch| {
                    let rect = Rect::from(ch.quad());
                    quads
                        .iter()
                        .any(|q| rects_intersect(rect, Rect::from(q.clone())))
                        .then(|| ch.char())
                        .flatten()
                })
                .collect();
            if !text.is_empty() {
                lines.push(text);
            }
        }
    }
    let text = lines.join("\n").trim().to_string();
    Ok(Some(PdfTextSelection {
        page: page_index,
        quads,
        text,
    }))
}

fn annotation_id(annotation: &mupdf::pdf::PdfAnnotation) -> Option<String> {
    annotation
        .object()
        .get_dict("NM")
        .ok()
        .flatten()
        .and_then(|object| object.as_string().ok().map(str::to_owned))
}

fn load_annotations(path: &Path) -> Result<Vec<PdfAnnotationInfo>, String> {
    let doc = PdfDocument::open(path).map_err(err_string)?;
    let page_count = doc.page_count().map_err(err_string)?;
    let mut annotations = Vec::new();
    for page_index in 0..page_count {
        let page = doc.load_pdf_page(page_index).map_err(err_string)?;
        for annotation in page.annotations() {
            if annotation.r#type().ok() != Some(PdfAnnotationType::Highlight) {
                continue;
            }
            annotations.push(PdfAnnotationInfo {
                page: page_index as usize,
                xref: annotation.xref().map_err(err_string)?,
                id: annotation_id(&annotation),
                quads: annotation.quad_points().unwrap_or_default(),
                contents: annotation
                    .contents()
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .to_string(),
            });
        }
    }
    Ok(annotations)
}

fn mutate_pdf_atomically(
    path: &Path,
    mutate: impl FnOnce(&mut PdfDocument) -> Result<(), String>,
) -> Result<(), String> {
    let mut doc = PdfDocument::open(path).map_err(err_string)?;
    mutate(&mut doc)?;
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document.pdf");
    let temp = parent.join(format!(
        ".{}.memex-{}.tmp",
        name,
        crate::vault::id::generate()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        doc.write_to(&mut file).map_err(err_string)?;
        file.flush().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::set_permissions(&temp, metadata.permissions()).map_err(|e| e.to_string())?;
        fs::rename(&temp, path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Extract text from all pages (runs on background thread on PDF open).
fn extract_all_text(raw_bytes: &[u8], page_count: usize) -> Vec<String> {
    let mut texts = Vec::with_capacity(page_count);
    let doc = match Document::from_bytes(raw_bytes, "") {
        Ok(d) => d,
        Err(_) => return vec![String::new(); page_count],
    };
    for i in 0..page_count {
        let text = doc
            .load_page(i as i32)
            .ok()
            .and_then(|p| p.to_text_page(mupdf::TextPageFlags::empty()).ok())
            .and_then(|tp| tp.to_text().ok())
            .unwrap_or_default();
        texts.push(text);
    }
    texts
}

/// Search the pre-cached text for a query. Returns context previews and
/// the set of pages with matches (for quad extraction).
fn search_text_cache(
    texts: &[String],
    query: &str,
    max_previews: usize,
) -> (Vec<(usize, String)>, Vec<usize>) {
    let query_lower = query.to_lowercase();
    let mut previews = Vec::new();
    let mut match_pages = Vec::new();
    let mut last_page = usize::MAX;

    for (page_idx, text) in texts.iter().enumerate() {
        let text_lower = text.to_lowercase();
        let mut search_from = 0;
        let mut page_has_match = false;

        while let Some(pos) = text_lower[search_from..].find(&query_lower) {
            page_has_match = true;
            if previews.len() < max_previews {
                let abs_pos = search_from + pos;
                let ctx_start = snap_floor(&text_lower, abs_pos.saturating_sub(40));
                let ctx_end = snap_ceil(
                    &text_lower,
                    (abs_pos + query_lower.len() + 40).min(text_lower.len()),
                );
                let mut snippet: String = text_lower[ctx_start..ctx_end]
                    .replace('\n', " ")
                    .replace('\r', "");
                if ctx_start > 0 {
                    snippet.insert_str(0, "…");
                }
                if ctx_end < text_lower.len() {
                    snippet.push('…');
                }
                previews.push((page_idx, snippet));
            }
            search_from = search_from + pos + query_lower.len();
        }

        if page_has_match && page_idx != last_page {
            match_pages.push(page_idx);
            last_page = page_idx;
        }
    }

    (previews, match_pages)
}

/// Get highlight quads only for pages known to have matches.
fn search_quads_for_pages(raw_bytes: &[u8], pages: &[usize], query: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let doc = match Document::from_bytes(raw_bytes, "") {
        Ok(d) => d,
        Err(_) => return hits,
    };
    for &page_idx in pages {
        if let Ok(page) = doc.load_page(page_idx as i32) {
            if let Ok(quads) = page.search(query, 100) {
                for quad in quads {
                    hits.push(SearchHit {
                        page: page_idx,
                        quad,
                    });
                }
            }
        }
    }
    hits
}

/// Snap a byte index to a valid char boundary (rounding down).
fn snap_floor(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Snap a byte index to a valid char boundary (rounding up).
fn snap_ceil(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Run a full-text search on a background thread.
/// Returns (search_hits for highlight quads, context previews for minibuffer).
fn search_background(
    raw_bytes: &[u8],
    page_count: usize,
    query: &str,
) -> (Vec<SearchHit>, Vec<(usize, String)>) {
    let mut hits = Vec::new();
    let mut previews = Vec::new();
    let max_previews = 50;

    let doc = match Document::from_bytes(raw_bytes, "") {
        Ok(d) => d,
        Err(_) => return (hits, previews),
    };

    let query_lower = query.to_lowercase();

    for page_idx in 0..page_count {
        let page = match doc.load_page(page_idx as i32) {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Collect highlight quads
        if let Ok(quads) = page.search(query, 100) {
            for quad in quads {
                hits.push(SearchHit {
                    page: page_idx,
                    quad,
                });
            }
        }

        // Collect context previews (up to max_previews total)
        if previews.len() < max_previews {
            if let Ok(tp) = page.to_text_page(mupdf::TextPageFlags::empty()) {
                if let Ok(text) = tp.to_text() {
                    let text_lower = text.to_lowercase();
                    let mut search_from = 0;
                    while let Some(pos) = text_lower[search_from..].find(&query_lower) {
                        if previews.len() >= max_previews {
                            break;
                        }
                        let abs_pos = search_from + pos;
                        let ctx_start = snap_floor(&text_lower, abs_pos.saturating_sub(40));
                        let ctx_end = snap_ceil(
                            &text_lower,
                            (abs_pos + query_lower.len() + 40).min(text_lower.len()),
                        );
                        let mut snippet: String = text_lower[ctx_start..ctx_end]
                            .replace('\n', " ")
                            .replace('\r', "");
                        if ctx_start > 0 {
                            snippet.insert_str(0, "…");
                        }
                        if ctx_end < text_lower.len() {
                            snippet.push('…');
                        }
                        previews.push((page_idx, snippet));
                        search_from = abs_pos + query_lower.len();
                    }
                }
            }
        }
    }

    (hits, previews)
}

/// Render a single page on a background thread. Standalone function (not on PdfState)
/// because mupdf::Document is not Send — we create it fresh from shared bytes.
fn render_page_background(raw_bytes: &[u8], page_index: usize, zoom: f32) -> Option<RenderedPage> {
    let doc = Document::from_bytes(raw_bytes, "").ok()?;
    let page = doc.load_page(page_index as i32).ok()?;
    let bounds = page.bounds().ok()?;

    let scale = (BASE_WIDTH * zoom) / bounds.width();
    let ctm = MuMatrix::new_scale(scale, scale);

    let pixmap = page
        .to_pixmap(&ctm, &Colorspace::device_rgb(), false, true)
        .ok()?;
    let width = pixmap.width();
    let height = pixmap.height();

    let mut png_buf = Cursor::new(Vec::new());
    pixmap
        .write_to(&mut png_buf, mupdf::ImageFormat::PNG)
        .ok()?;

    Some(RenderedPage {
        image: Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            png_buf.into_inner(),
        )),
        width,
        height,
    })
}

#[cfg(test)]
mod annotation_tests {
    use super::{annotation_alias, err_string, load_annotations, mutate_pdf_atomically};
    use mupdf::pdf::{PdfDocument, PdfObject};
    use mupdf::{Quad, Rect};

    #[test]
    fn annotation_alias_is_safe_for_wikilinks() {
        assert_eq!(
            annotation_alias("  selected   text | more ]]  "),
            "selected text - more ] ]"
        );
    }

    #[test]
    fn native_highlight_roundtrips_through_atomic_save() {
        let path =
            std::env::temp_dir().join(format!("memex-pdf-{}.pdf", crate::vault::id::generate()));
        let mut doc = PdfDocument::new();
        doc.new_page(mupdf::Size::new(300.0, 300.0)).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        doc.write_to(&mut file).unwrap();
        drop(file);

        mutate_pdf_atomically(&path, |doc| {
            let mut page = doc.load_pdf_page(0).map_err(err_string)?;
            let mut annotation = page
                .add_highlight_annotation(Quad::from(Rect::new(10.0, 20.0, 80.0, 35.0)))
                .map_err(err_string)?;
            annotation
                .set_contents("selected text")
                .map_err(err_string)?;
            annotation
                .object()
                .dict_put(
                    "NM",
                    PdfObject::new_string("memex:test").map_err(err_string)?,
                )
                .map_err(err_string)?;
            annotation.update().map_err(err_string)?;
            Ok(())
        })
        .unwrap();

        let annotations = load_annotations(&path).unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].id.as_deref(), Some("memex:test"));
        assert_eq!(annotations[0].contents, "selected text");
        std::fs::remove_file(path).unwrap();
    }
}
