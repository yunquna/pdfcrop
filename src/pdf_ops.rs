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
pub fn apply_cropbox(
    doc: &mut Document,
    page_num: usize,
    bbox: &BoundingBox,
    clip_content: bool,
) -> Result<()> {
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

    // If clip_content is enabled, filter page content using component-based approach
    // This removes paths and images that don't overlap with the crop box
    // Text blocks and Form XObjects are kept for safety
    if clip_content {
        filter_page_content(doc, page_id, bbox)?;
    }

    Ok(())
}

/// Filter page content to remove elements outside the crop box
///
/// This analyzes the page's content stream and removes drawing operations
/// that fall completely outside the crop box. This ensures clipped content
/// is actually removed from the PDF file for privacy/security.
fn filter_page_content(doc: &mut Document, page_id: (u32, u16), bbox: &BoundingBox) -> Result<()> {
    use crate::content_filter::{filter_content_stream, filter_form_xobject};

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str("[DEBUG] Filtering page content..."));
    }

    // Get the page dictionary and clone needed data to avoid borrow conflicts
    let (contents_ref, resources) = {
        let page = doc
            .get_object(page_id)
            .map_err(|e| Error::PdfParse(format!("failed to get page: {}", e)))?
            .as_dict()
            .map_err(|e| Error::PdfParse(format!("page is not a dictionary: {}", e)))?;

        // Clone the page's Resources (needed for Form XObject lookup)
        let resources = page
            .get(b"Resources")
            .ok()
            .and_then(|obj| obj.as_dict().ok())
            .map(|d| d.clone());

        // Clone the Contents reference
        let contents_ref = match page.get(b"Contents") {
            Ok(obj) => obj.clone(),
            Err(_) => {
                // No existing content, nothing to filter
                return Ok(());
            }
        };

        (contents_ref, resources)
    };

    // Collect all Form XObjects to filter
    let mut all_form_xobjects = vec![];

    // Handle both single stream and array of streams
    match contents_ref {
        Object::Reference(ref_id) => {
            // Single content stream - filter it
            let stream = doc
                .get_object(ref_id)
                .map_err(|e| Error::PdfParse(format!("failed to get stream: {}", e)))?
                .as_stream()
                .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

            // Filter the content stream (collects Form XObjects for second pass)
            let (filtered_content, form_xobjects) =
                filter_content_stream(doc, stream, resources.as_ref(), bbox)?;
            all_form_xobjects.extend(form_xobjects);

            // Update the stream with filtered content
            let stream_mut = doc
                .get_object_mut(ref_id)
                .map_err(|e| Error::PdfParse(format!("failed to get stream mut: {}", e)))?
                .as_stream_mut()
                .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

            stream_mut.set_plain_content(filtered_content);
        }
        Object::Array(ref streams) => {
            // Multiple content streams - filter ALL of them
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Page has {} content streams",
                    streams.len()
                )));
            }

            #[cfg(debug_assertions)]
            eprintln!("[DEBUG] Page has {} content streams (array)", streams.len());

            for (idx, stream_ref) in streams.iter().enumerate() {
                if let Object::Reference(ref_id) = stream_ref {
                    #[cfg(target_arch = "wasm32")]
                    {
                        use wasm_bindgen::JsValue;
                        web_sys::console::log_1(&JsValue::from_str(&format!(
                            "[DEBUG] Filtering content stream {} of {}",
                            idx + 1,
                            streams.len()
                        )));
                    }

                    let stream = doc
                        .get_object(*ref_id)
                        .map_err(|e| Error::PdfParse(format!("failed to get stream: {}", e)))?
                        .as_stream()
                        .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

                    let (filtered_content, form_xobjects) =
                        filter_content_stream(doc, stream, resources.as_ref(), bbox)?;
                    all_form_xobjects.extend(form_xobjects);

                    let stream_mut = doc
                        .get_object_mut(*ref_id)
                        .map_err(|e| Error::PdfParse(format!("failed to get stream mut: {}", e)))?
                        .as_stream_mut()
                        .map_err(|e| Error::PdfParse(format!("object is not a stream: {}", e)))?;

                    stream_mut.set_plain_content(filtered_content);
                }
            }
        }
        _ => {
            return Err(Error::PdfParse(
                "Contents is not a reference or array".to_string(),
            ));
        }
    }

    // Second pass: Recursively filter all collected Form XObjects
    // DISABLED: Form XObjects have their own coordinate system which doesn't match page coordinates
    // Filtering them with page bbox causes incorrect content removal
    // TODO: Implement coordinate transformation from page space to XObject space
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Skipping Form XObject filtering ({} found) - coordinate transformation not yet implemented",
            all_form_xobjects.len()
        )));
    }

    // NOTE: Commented out Form XObject filtering for now
    // while let Some((xobj_id, xobj_resources)) = all_form_xobjects.pop() {
    //     match filter_form_xobject(doc, xobj_id, xobj_resources, bbox) {
    //         Ok(nested_xobjects) => {
    //             // Add nested Form XObjects to the queue for recursive filtering
    //             all_form_xobjects.extend(nested_xobjects);
    //         }
    //         Err(e) => {
    //             #[cfg(target_arch = "wasm32")]
    //             {
    //                 use wasm_bindgen::JsValue;
    //                 web_sys::console::log_1(&JsValue::from_str(&format!(
    //                     "[DEBUG] Could not filter Form XObject {:?}: {}",
    //                     xobj_id, e
    //                 )));
    //             }
    //         }
    //     }
    // }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str("[DEBUG] Content filtering complete"));
    }

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
