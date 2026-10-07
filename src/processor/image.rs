use anyhow::{Context, Result};
use image::{imageops::FilterType, ImageFormat};
use std::io::Cursor;

pub struct ImageMeta {
    pub width: u32,
    pub height: u32,
    pub thumbnail_bytes: Option<Vec<u8>>,
}

pub fn process_image(
    data: &[u8],
    max_thumb_width: u32,
    max_thumb_height: u32,
) -> Result<ImageMeta> {
    let img = image::load_from_memory(data).context("Failed to decode image from memory")?;

    let (width, height) = (img.width(), img.height());

    // Generate thumbnail
    let thumb = img.resize(max_thumb_width, max_thumb_height, FilterType::Lanczos3);
    let mut thumb_cursor = Cursor::new(Vec::new());

    thumb
        .write_to(&mut thumb_cursor, ImageFormat::WebP)
        .context("Failed to encode thumbnail as WebP")?;

    Ok(ImageMeta {
        width,
        height,
        thumbnail_bytes: Some(thumb_cursor.into_inner()),
    })
}
