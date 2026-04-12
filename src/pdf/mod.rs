mod view;

pub use view::PdfView;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::*;
use pdfium_render::prelude::*;

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
    pdfium: Pdfium,
    raw_bytes: Vec<u8>,
}

impl PdfState {
    pub fn new(path: impl AsRef<Path>, cx: &mut Context<Self>) -> Result<Self, PdfiumError> {
        let path = path.as_ref().to_path_buf();
        let pdfium = Pdfium::default();
        let raw_bytes = std::fs::read(&path).map_err(PdfiumError::IoError)?;
        let page_count = {
            let doc = pdfium.load_pdf_from_byte_slice(&raw_bytes, None)?;
            doc.pages().len() as usize
        };

        Ok(Self {
            path,
            page_count,
            scroll_offset: px(0.),
            zoom: 1.0,
            focus_handle: cx.focus_handle(),
            page_cache: HashMap::new(),
            pdfium,
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

    fn render_page_inner(&self, page_index: usize) -> Result<RenderedPage, PdfiumError> {
        let doc = self.pdfium.load_pdf_from_byte_slice(&self.raw_bytes, None)?;
        let page = doc.pages().get(page_index as u16)?;

        let base_width = 800.0;
        let scale = (base_width * self.zoom) / page.width().value;
        let width = (page.width().value * scale) as u32;
        let height = (page.height().value * scale) as u32;

        let config = PdfRenderConfig::new()
            .set_target_width(width as i32)
            .set_maximum_height(height as i32);

        let bitmap = page.render_with_config(&config)?;
        let dyn_image = bitmap.as_image();

        // Encode to PNG for gpui
        let mut png_buf = Cursor::new(Vec::new());
        dyn_image
            .write_to(&mut png_buf, image::ImageFormat::Png)
            .map_err(|_| PdfiumError::ImageError)?;

        Ok(RenderedPage {
            image: Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                png_buf.into_inner(),
            )),
            width: dyn_image.width(),
            height: dyn_image.height(),
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
