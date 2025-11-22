//! PDF manipulation operations using lopdf

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};
use lopdf::{Document, Object};

/// Apply a bounding box to a PDF page by setting its CropBox
///
/// The CropBox defines the region of the page to be displayed or printed.
/// This is the primary method for "cropping" a PDF page.
pub fn apply_cropbox(doc: &mut Document, page_num: usize, bbox: &BoundingBox) -> Result<()> {
    // Get the page ID
    let page_id = doc
        .page_iter()
        .nth(page_num)
        .ok_or_else(|| Error::InvalidPage(format!("page {} not found", page_num)))?;

    // Get the page dictionary
    let page_dict = doc
        .get_object_mut(page_id)
        .map_err(|e| Error::PdfParse(format!("failed to get page {}: {}", page_num, e)))?
        .as_dict_mut()
        .map_err(|e| Error::PdfParse(format!("page {} is not a dictionary: {}", page_num, e)))?;

    // Create CropBox array: [left, bottom, right, top]
    let cropbox = Object::Array(vec![
        Object::Real(bbox.left as f32),
        Object::Real(bbox.bottom as f32),
        Object::Real(bbox.right as f32),
        Object::Real(bbox.top as f32),
    ]);

    // Set the CropBox
    page_dict.set("CropBox", cropbox);

    Ok(())
}

/// Get the MediaBox dimensions of a page
///
/// MediaBox defines the boundaries of the physical medium
pub fn get_page_dimensions(doc: &Document, page_num: usize) -> Result<(f64, f64)> {
    let page_id = doc
        .page_iter()
        .nth(page_num)
        .ok_or_else(|| Error::InvalidPage(format!("page {} not found", page_num)))?;

    let page = doc
        .get_object(page_id)
        .map_err(|e| Error::PdfParse(format!("failed to get page {}: {}", page_num, e)))?
        .as_dict()
        .map_err(|e| Error::PdfParse(format!("page {} is not a dictionary: {}", page_num, e)))?;

    let media_box = page
        .get(b"MediaBox")
        .map_err(|e| Error::PdfParse(format!("MediaBox not found: {}", e)))?
        .as_array()
        .map_err(|e| Error::PdfParse(format!("MediaBox is not an array: {}", e)))?;

    if media_box.len() != 4 {
        return Err(Error::PdfParse(format!(
            "MediaBox has wrong length: {}",
            media_box.len()
        )));
    }

    // MediaBox values can be either Integer or Real
    let left = media_box[0]
        .as_f32()
        .map(|f| f as f64)
        .or_else(|_| media_box[0].as_i64().map(|i| i as f64))
        .map_err(|e| Error::PdfParse(format!("invalid MediaBox left: {}", e)))?;
    let bottom = media_box[1]
        .as_f32()
        .map(|f| f as f64)
        .or_else(|_| media_box[1].as_i64().map(|i| i as f64))
        .map_err(|e| Error::PdfParse(format!("invalid MediaBox bottom: {}", e)))?;
    let right = media_box[2]
        .as_f32()
        .map(|f| f as f64)
        .or_else(|_| media_box[2].as_i64().map(|i| i as f64))
        .map_err(|e| Error::PdfParse(format!("invalid MediaBox right: {}", e)))?;
    let top = media_box[3]
        .as_f32()
        .map(|f| f as f64)
        .or_else(|_| media_box[3].as_i64().map(|i| i as f64))
        .map_err(|e| Error::PdfParse(format!("invalid MediaBox top: {}", e)))?;

    let width = right - left;
    let height = top - bottom;

    Ok((width, height))
}

/// Get the number of pages in a PDF document
pub fn get_page_count(doc: &Document) -> usize {
    doc.get_pages().len()
}
