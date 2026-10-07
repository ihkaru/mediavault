pub mod image;
pub mod pdf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedMedia {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub total_pages: Option<usize>,
    pub extracted_text: Option<String>,
    #[serde(skip)]
    pub thumbnail_bytes: Option<Vec<u8>>,
}

pub fn analyze_and_process(
    mime_type: &str,
    data: &[u8],
    max_thumb_width: u32,
    max_thumb_height: u32,
) -> ProcessedMedia {
    if mime_type.starts_with("image/") {
        if let Ok(meta) = image::process_image(data, max_thumb_width, max_thumb_height) {
            return ProcessedMedia {
                width: Some(meta.width),
                height: Some(meta.height),
                total_pages: None,
                extracted_text: None,
                thumbnail_bytes: meta.thumbnail_bytes,
            };
        }
    } else if mime_type == "application/pdf" {
        if let Ok(meta) = pdf::process_pdf(data) {
            return ProcessedMedia {
                width: None,
                height: None,
                total_pages: Some(meta.total_pages),
                extracted_text: meta.extracted_text,
                thumbnail_bytes: None,
            };
        }
    }

    ProcessedMedia {
        width: None,
        height: None,
        total_pages: None,
        extracted_text: None,
        thumbnail_bytes: None,
    }
}
