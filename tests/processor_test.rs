use image::{ImageBuffer, ImageFormat, Rgb};
use lopdf::{dictionary, Document, Object, Stream};
use std::io::Cursor;
use tempfile::tempdir;

// Helper to create a dummy valid PNG in memory
pub fn create_test_png(width: u32, height: u32) -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_fn(width, height, |x, y| Rgb([(x % 255) as u8, (y % 255) as u8, 128]));

    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).unwrap();
    buf.into_inner()
}

// Helper to create a dummy valid PDF with text in memory
pub fn create_test_pdf(text: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! {
            "F1" => font_id,
        },
    });

    let content = format!("BT /F1 24 Tf 100 100 Td ({}) Tj ET", text);
    let content_stream = Stream::new(dictionary! {}, content.as_bytes().to_vec());
    let content_id = doc.add_object(content_stream);

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    });

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
    };

    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut buf = Vec::new();
    doc.save_to(&mut buf).unwrap();
    buf
}

#[tokio::test]
async fn test_image_processing_and_thumbnail() {
    let png_data = create_test_png(200, 200);
    assert!(!png_data.is_empty());

    let meta = mediavault::processor::image::process_image(&png_data, 100, 100).expect("Image process failed");
    assert_eq!(meta.width, 200);
    assert_eq!(meta.height, 200);
    assert!(meta.thumbnail_bytes.is_some());
    assert!(!meta.thumbnail_bytes.unwrap().is_empty());
}

#[tokio::test]
async fn test_pdf_processing_text_extraction() {
    let pdf_data = create_test_pdf("Invoice Statement Hello World MediaVault");
    assert!(!pdf_data.is_empty());

    let meta = mediavault::processor::pdf::process_pdf(&pdf_data).expect("PDF process failed");
    assert_eq!(meta.total_pages, 1);
    assert!(meta.extracted_text.is_some());
    let text = meta.extracted_text.unwrap();
    assert!(text.contains("Invoice") || text.contains("Hello"));
}

#[tokio::test]
async fn test_local_storage_crud() {
    let dir = tempdir().expect("Failed to create tempdir");
    let storage = mediavault::storage::local::LocalStorage::new(dir.path()).expect("Storage init failed");
    use mediavault::storage::StorageBackend;

    let test_data = b"Hello from MediaVault LocalStorage";
    storage
        .put_object("test/sample.txt", test_data, "text/plain")
        .await
        .expect("Put object failed");

    let read_back = storage
        .get_object("test/sample.txt")
        .await
        .expect("Get object failed");
    assert_eq!(read_back, test_data);

    storage
        .delete_object("test/sample.txt")
        .await
        .expect("Delete object failed");
    assert!(!storage.exists("test/sample.txt").await.unwrap());
}
