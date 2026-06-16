//! Excalidraw file schema.
//!
//! This is the on-disk and in-memory model for diagrams. A `.excalidraw` file
//! is JSON with a top-level `{ type, version, source, elements, appState,
//! files }` shape. We deserialize into typed fields for the fields we render
//! and edit, and capture every other field in a flattened `extra` map so that
//! load -> save round-trips losslessly (we never drop data we did not model).
//!
//! Element geometry: `points` on line/arrow/freedraw elements are relative to
//! the element's own `x`/`y` origin (excalidraw convention).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Top-level excalidraw document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExcalidrawFile {
    #[serde(rename = "type")]
    pub file_type: String,
    pub version: u32,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub elements: Vec<Element>,
    #[serde(default, rename = "appState")]
    pub app_state: Map<String, Value>,
    #[serde(default)]
    pub files: Map<String, Value>,
}

impl ExcalidrawFile {
    /// A new, empty diagram document with a white background.
    pub fn empty() -> Self {
        let mut app_state = Map::new();
        app_state.insert(
            "viewBackgroundColor".to_string(),
            Value::String("#ffffff".to_string()),
        );
        app_state.insert("gridSize".to_string(), Value::Null);
        Self {
            file_type: "excalidraw".to_string(),
            version: 2,
            source: "memex".to_string(),
            elements: Vec::new(),
            app_state,
            files: Map::new(),
        }
    }

    /// Parse a document from JSON bytes.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| format!("invalid excalidraw JSON: {}", e))
    }

    /// Load a document from disk.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("failed to read diagram: {}", e))?;
        Self::from_json(&bytes)
    }

    /// Serialize to pretty JSON.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("failed to encode diagram: {}", e))
    }

    /// Write the document to disk.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = self.to_json()?;
        std::fs::write(path, json).map_err(|e| format!("failed to write diagram: {}", e))
    }

    /// The view background color from appState, if set.
    pub fn background_color(&self) -> Option<&str> {
        self.app_state
            .get("viewBackgroundColor")
            .and_then(Value::as_str)
    }
}

/// A single drawing element. Excalidraw discriminates by the `type` string;
/// type-specific fields are optional. Unmodeled fields land in `extra` so we
/// preserve them on save.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub id: String,
    #[serde(rename = "type")]
    pub element_type: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    #[serde(default)]
    pub angle: f64,
    #[serde(default = "default_stroke_color")]
    pub stroke_color: String,
    #[serde(default = "default_background_color")]
    pub background_color: String,
    #[serde(default = "default_fill_style")]
    pub fill_style: String,
    #[serde(default = "default_stroke_width")]
    pub stroke_width: f64,
    #[serde(default = "default_stroke_style")]
    pub stroke_style: String,
    #[serde(default)]
    pub roughness: f64,
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    #[serde(default)]
    pub is_deleted: bool,

    // ── line / arrow / freedraw ──────────────────────────────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_arrowhead: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_arrowhead: Option<Value>,

    // ── text ─────────────────────────────────────────────────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_align: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,

    /// Every field we did not model, preserved for lossless round-trip.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Element {
    /// Build an element with excalidraw defaults; callers tweak the fields
    /// they care about. Used by importers.
    pub fn base(
        id: impl Into<String>,
        element_type: impl Into<String>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Self {
        Self {
            id: id.into(),
            element_type: element_type.into(),
            x,
            y,
            width,
            height,
            angle: 0.0,
            stroke_color: default_stroke_color(),
            background_color: default_background_color(),
            fill_style: default_fill_style(),
            stroke_width: default_stroke_width(),
            stroke_style: default_stroke_style(),
            roughness: 1.0,
            opacity: default_opacity(),
            is_deleted: false,
            points: None,
            start_arrowhead: None,
            end_arrowhead: None,
            text: None,
            font_size: None,
            font_family: None,
            text_align: None,
            vertical_align: None,
            container_id: None,
            extra: Map::new(),
        }
    }
}

fn default_stroke_color() -> String {
    "#1e1e1e".to_string()
}
fn default_background_color() -> String {
    "transparent".to_string()
}
fn default_fill_style() -> String {
    "solid".to_string()
}
fn default_stroke_width() -> f64 {
    1.0
}
fn default_stroke_style() -> String {
    "solid".to_string()
}
fn default_opacity() -> f64 {
    100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_round_trips() {
        let file = ExcalidrawFile::empty();
        let json = file.to_json().unwrap();
        let reparsed = ExcalidrawFile::from_json(json.as_bytes()).unwrap();
        assert_eq!(reparsed.file_type, "excalidraw");
        assert_eq!(reparsed.version, 2);
        assert!(reparsed.elements.is_empty());
        assert_eq!(reparsed.background_color(), Some("#ffffff"));
    }

    #[test]
    fn preserves_unmodeled_element_fields() {
        let src = r##"{
            "type": "excalidraw",
            "version": 2,
            "source": "https://excalidraw.com",
            "elements": [
                {
                    "id": "abc",
                    "type": "rectangle",
                    "x": 10.0,
                    "y": 20.0,
                    "width": 100.0,
                    "height": 50.0,
                    "strokeColor": "#1e1e1e",
                    "roundness": {"type": 3},
                    "boundElements": [{"id": "t1", "type": "text"}]
                }
            ],
            "appState": {"viewBackgroundColor": "#ffffff"},
            "files": {}
        }"##;
        let file = ExcalidrawFile::from_json(src.as_bytes()).unwrap();
        let el = &file.elements[0];
        assert_eq!(el.element_type, "rectangle");
        assert_eq!(el.x, 10.0);
        // Unmodeled fields survive into `extra`.
        assert!(el.extra.contains_key("roundness"));
        assert!(el.extra.contains_key("boundElements"));
        // And they re-serialize.
        let json = file.to_json().unwrap();
        assert!(json.contains("roundness"));
        assert!(json.contains("boundElements"));
    }

    #[test]
    fn parses_arrow_points() {
        let src = r#"{
            "type": "excalidraw",
            "version": 2,
            "elements": [
                {"id": "a", "type": "arrow", "x": 0.0, "y": 0.0,
                 "points": [[0.0, 0.0], [100.0, 40.0]]}
            ],
            "appState": {},
            "files": {}
        }"#;
        let file = ExcalidrawFile::from_json(src.as_bytes()).unwrap();
        let pts = file.elements[0].points.as_ref().unwrap();
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[1], [100.0, 40.0]);
    }
}
