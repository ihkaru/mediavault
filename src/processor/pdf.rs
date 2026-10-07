use anyhow::{Context, Result};
use lopdf::Document;

pub struct PdfMeta {
    pub total_pages: usize,
    pub extracted_text: Option<String>,
}

pub fn process_pdf(data: &[u8]) -> Result<PdfMeta> {
    let doc = Document::load_mem(data).context("Failed to parse PDF document")?;

    let pages = doc.get_pages();
    let total_pages = pages.len();

    let mut full_text = String::new();
    // Extract text from the first 25 pages to avoid out-of-memory on gigantic 1000-page books
    let pages_to_extract = pages.keys().take(25).copied().collect::<Vec<_>>();

    if let Ok(text) = doc.extract_text(&pages_to_extract) {
        let clean = text.trim();
        if !clean.is_empty() {
            // Truncate to reasonable indexing length (e.g. 50,000 characters)
            let truncated = if clean.chars().count() > 50_000 {
                clean.chars().take(50_000).collect::<String>()
            } else {
                clean.to_string()
            };
            full_text = truncated;
        }
    }

    Ok(PdfMeta {
        total_pages,
        extracted_text: if full_text.is_empty() {
            None
        } else {
            Some(full_text)
        },
    })
}
