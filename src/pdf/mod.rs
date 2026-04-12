mod view;

pub use view::PdfView;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::*;
use mupdf::{Colorspace, Document, Matrix as MuMatrix};

/// Cached rendered page: PNG-encoded bytes ready for gpui.
pub struct RenderedPage {
    pub image: Arc<gpui::Image>,
    pub width: u32,
    pub height: u32,
}

/// State for an open PDF document.
pub struct PdfState {
    pub path: PathBuf,
    pub page_count: usize,
    pub scroll_offset: gpui::Pixels,
    pub zoom: f32,
    pub focus_handle: FocusHandle,
    page_cache: HashMap<usize, RenderedPage>,
    raw_bytes: Vec<u8>,
}

impl PdfState {
    pub fn new(path: impl AsRef<Path>, cx: &mut Context<Self>) -> Result<Self, mupdf::Error> {
        let path = path.as_ref().to_path_buf();
        let raw_bytes = std::fs::read(&path)
            .map_err(mupdf::Error::Io)?;
        let doc = Document::from_bytes(&raw_bytes, "")?;
        let page_count = doc.page_count()? as usize;

        Ok(Self {
            path,
            page_count,
            scroll_offset: px(0.),
            zoom: 1.0,
            focus_handle: cx.focus_handle(),
            page_cache: HashMap::new(),
            raw_bytes,
        })
    }

    /// Render a page to a PNG-encoded gpui Image. Results are cached.
    pub fn render_page(&mut self, page_index: usize) -> Option<&RenderedPage> {
        if page_index >= self.page_count {
            return None;
        }

        if !self.page_cache.contains_key(&page_index) {
            if let Ok(rendered) = self.render_page_inner(page_index) {
                self.page_cache.insert(page_index, rendered);
            }
        }

        self.page_cache.get(&page_index)
    }

    fn render_page_inner(&self, page_index: usize) -> Result<RenderedPage, mupdf::Error> {
        let doc = Document::from_bytes(&self.raw_bytes, "")?;
        let page = doc.load_page(page_index as i32)?;
        let bounds = page.bounds()?;

        let base_width = 800.0;
        let scale = (base_width * self.zoom) / bounds.width();
        let ctm = MuMatrix::new_scale(scale, scale);

        let pixmap = page.to_pixmap(&ctm, &Colorspace::device_rgb(), false, true)?;
        let width = pixmap.width();
        let height = pixmap.height();

        let mut png_buf = Cursor::new(Vec::new());
        pixmap.write_to(&mut png_buf, mupdf::ImageFormat::PNG)?;

        Ok(RenderedPage {
            image: Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                png_buf.into_inner(),
            )),
            width,
            height,
        })
    }

    /// Invalidate cached pages (e.g., after zoom change).
    pub fn invalidate_cache(&mut self) {
        self.page_cache.clear();
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    /// Rendered page height at current zoom for layout calculations.
    pub fn page_height(&self, page_index: usize) -> gpui::Pixels {
        if let Some(cached) = self.page_cache.get(&page_index) {
            return px(cached.height as f32);
        }
        px(1100.0 * self.zoom)
    }
}
