//! Import diagrams from external formats into the native excalidraw model.

pub mod drawio;
pub mod excalidraw;

use std::path::Path;

use super::model::ExcalidrawFile;

/// Import a diagram file into an excalidraw document, dispatching on the file
/// extension. Supports `.excalidraw` (native), `.drawio`/`.xml` (mxGraph).
pub fn import_file(path: &Path) -> Result<ExcalidrawFile, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "excalidraw" => {
            let bytes = std::fs::read(path).map_err(|e| format!("failed to read file: {}", e))?;
            excalidraw::parse(&bytes)
        }
        "drawio" | "xml" => {
            let xml =
                std::fs::read_to_string(path).map_err(|e| format!("failed to read file: {}", e))?;
            let elements = drawio::parse(&xml)?;
            let mut file = ExcalidrawFile::empty();
            file.elements = elements;
            Ok(file)
        }
        other => Err(format!("unsupported diagram format: .{}", other)),
    }
}
