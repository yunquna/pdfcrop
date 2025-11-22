//! PDF manipulation operations using lopdf

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};
use lopdf::{Document, Object};

/// Apply a bounding box to a PDF page by setting its CropBox
///
/// The CropBox defines the region of the page to be displayed or printed.
/// This is the primary method for "cropping" a PDF page.
///
/// If `clip_content` is true, also adds a clipping path to the content stream
/// to actually remove/hide content outside the bbox.
pub fn apply_cropbox(doc: &mut Document, page_num: usize, bbox: &BoundingBox, clip_content: bool) -> Result<()> {
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

    // If clip_content is enabled, add a clipping path to the content stream
    if clip_content {
        add_clipping_path(doc, page_id, bbox)?;
    }

    Ok(())
}

/// Add a clipping path to the beginning of a page's content stream
///
/// This inserts a rectangular clipping path that restricts all drawing
/// operations to the specified bounding box. Content outside the box
/// will be effectively invisible and can be removed during compression.
fn add_clipping_path(doc: &mut Document, page_id: (u32, u16), bbox: &BoundingBox) -> Result<()> {
    // Create the clipping path commands
    // Format: q (save state) + rectangle + W (clip) + n (end path without painting)
    let clip_commands = format!(
        "q {} {} {} {} re W n\n",
        bbox.left,
        bbox.bottom,
        bbox.width(),
        bbox.height()
    );

    // Get the page dictionary
    let page = doc
        .get_object(page_id)
        .map_err(|e| Error::PdfParse(format!("failed to get page: {}", e)))?
        .as_dict()
        .map_err(|e| Error::PdfParse(format!("page is not a dictionary: {}", e)))?;

    // Get the current Contents object
    let contents_ref = match page.get(b"Contents") {
        Ok(obj) => obj.clone(),
        Err(_) => {
            // No existing content, create new content stream with just the clipping path
            let stream_dict = lopdf::Dictionary::new();
            let stream_data = clip_commands.into_bytes();
            let stream = lopdf::Stream::new(stream_dict, stream_data);
            let stream_id = doc.add_object(stream);

            // Add Contents reference to page
            let page_dict = doc.get_object_mut(page_id)
                .map_err(|e| Error::PdfParse(format!("failed to get page mut: {}", e)))?
                .as_dict_mut()
                .map_err(|e| Error::PdfParse(format!("page is not a dictionary: {}", e)))?;
            page_dict.set("Contents", Object::Reference(stream_id));

            return Ok(());
        }
    };

    // Handle both single stream and array of streams
    match contents_ref {
        Object::Reference(ref_id) => {
            // Single content stream - prepend clipping path
            prepend_to_stream(doc, ref_id, &clip_commands)?;
        }
        Object::Array(ref streams) => {
            // Multiple content streams - prepend to first one
            if let Some(Object::Reference(ref_id)) = streams.first() {
                prepend_to_stream(doc, *ref_id, &clip_commands)?;
            }
        }
        _ => {
            return Err(Error::PdfParse(
                "Contents is not a reference or array".to_string(),
            ));
        }
    }

    Ok(())
}

/// Prepend content to an existing stream object
fn prepend_to_stream(doc: &mut Document, stream_id: (u32, u16), prefix: &str) -> Result<()> {
    let stream = doc
        .get_object_mut(stream_id)
        .map_err(|e| Error::PdfParse(format!("failed to get stream: {}", e)))?
        .as_stream_mut()
        .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

    // Try to decode the existing content - handle both compressed and uncompressed streams
    let existing_content = stream.decompressed_content().unwrap_or_else(|_| {
        // If decompression fails, the stream may already be uncompressed
        stream.content.clone()
    });

    // Prepend the clipping path
    let mut new_content = prefix.as_bytes().to_vec();
    new_content.extend_from_slice(&existing_content);

    // Update the stream content (will be recompressed when saved)
    stream.set_plain_content(new_content);

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
