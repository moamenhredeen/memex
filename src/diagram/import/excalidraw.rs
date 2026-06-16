//! Excalidraw import. The native format is excalidraw, so importing is just
//! parsing (and re-normalising) the document.

use crate::diagram::model::ExcalidrawFile;

/// Parse an excalidraw document from JSON bytes.
pub fn parse(bytes: &[u8]) -> Result<ExcalidrawFile, String> {
    let mut file = ExcalidrawFile::from_json(bytes)?;
    // Normalise: ensure our own provenance and a current version on save.
    file.file_type = "excalidraw".to_string();
    if file.version == 0 {
        file.version = 2;
    }
    Ok(file)
}
