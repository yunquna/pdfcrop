//! Main PDF cropping logic

use crate::bbox::{detect_bbox, BoundingBox};
use crate::error::Result;
use crate::pdf_ops::{apply_cropbox, get_page_count, get_page_dimensions};
use crate::CropOptions;
use lopdf::Document;

/// Crop a PDF file according to the specified options
///
/// This function:
/// 1. Loads the PDF from bytes
/// 2. For each page:
///    - Detects or uses the specified bounding box
///    - Applies margins
///    - Sets the CropBox
/// 3. Returns the cropped PDF as bytes
///
/// # Example
///
/// ```no_run
/// use pdfcrop::{crop_pdf, CropOptions, Margins};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let pdf_data = std::fs::read("input.pdf")?;
/// let options = CropOptions {
///     margins: Margins::uniform(10.0),
///     ..Default::default()
/// };
/// let cropped = crop_pdf(&pdf_data, options)?;
/// std::fs::write("output.pdf", cropped)?;
/// # Ok(())
/// # }
/// ```
pub fn crop_pdf(pdf_data: &[u8], options: CropOptions) -> Result<Vec<u8>> {
    // Load the PDF document
    let mut doc = Document::load_mem(pdf_data)?;

    let page_count = get_page_count(&doc);

    if options.verbose {
        eprintln!("Processing {} pages", page_count);
    }

    // Process each page
    for page_num in 0..page_count {
        if options.verbose {
            eprintln!("Processing page {}/{}", page_num + 1, page_count);
        }

        // Determine which bounding box to use
        let bbox = determine_bbox(pdf_data, &mut doc, page_num, &options)?;

        if options.verbose {
            eprintln!(
                "  Detected bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                bbox.left, bbox.bottom, bbox.right, bbox.top
            );
            eprintln!("  Size: {:.2} x {:.2} pts", bbox.width(), bbox.height());
        }

        // Apply margins
        let bbox_with_margins = bbox.with_margins(&options.margins);

        if options.verbose {
            eprintln!(
                "  With margins: ({:.2}, {:.2}, {:.2}, {:.2})",
                bbox_with_margins.left,
                bbox_with_margins.bottom,
                bbox_with_margins.right,
                bbox_with_margins.top
            );
        }

        // Clamp to page dimensions
        let (page_width, page_height) = get_page_dimensions(&doc, page_num)?;
        let final_bbox = bbox_with_margins.clamp_to_page(page_width, page_height);

        if options.verbose {
            eprintln!(
                "  Final bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                final_bbox.left, final_bbox.bottom, final_bbox.right, final_bbox.top
            );
        }

        // Apply the crop box
        apply_cropbox(&mut doc, page_num, &final_bbox)?;
    }

    // Save the document to bytes
    let mut output = Vec::new();
    doc.save_to(&mut output)?;

    Ok(output)
}

/// Determine which bounding box to use for a given page
fn determine_bbox(pdf_data: &[u8], doc: &mut Document, page_num: usize, options: &CropOptions) -> Result<BoundingBox> {
    // Check for page-specific override (odd/even)
    let page_number = page_num + 1; // 1-indexed for odd/even check

    if page_number % 2 == 1 {
        // Odd page
        if let Some(bbox) = options.bbox_odd {
            return Ok(bbox);
        }
    } else {
        // Even page
        if let Some(bbox) = options.bbox_even {
            return Ok(bbox);
        }
    }

    // Check for global override
    if let Some(bbox) = options.bbox_override {
        return Ok(bbox);
    }

    // Auto-detect bbox using specified method
    detect_bbox_with_method(pdf_data, doc, page_num, options.bbox_method, options.verbose)
}

/// Detect bounding box using the specified method
fn detect_bbox_with_method(
    pdf_data: &[u8],
    doc: &mut Document,
    page_num: usize,
    method: crate::BBoxMethod,
    verbose: bool,
) -> Result<BoundingBox> {
    use crate::BBoxMethod;

    match method {
        BBoxMethod::Ghostscript => {
            crate::ghostscript::detect_bbox_gs(pdf_data, page_num)
        }
        BBoxMethod::ContentStream => {
            detect_bbox(doc, page_num)
        }
        BBoxMethod::Auto => {
            // Try Ghostscript first
            match crate::ghostscript::detect_bbox_gs(pdf_data, page_num) {
                Ok(bbox) => {
                    if verbose {
                        eprintln!("  BBox method: Ghostscript");
                    }
                    Ok(bbox)
                }
                Err(e) => {
                    if verbose {
                        eprintln!("  Ghostscript unavailable ({}), using content stream parsing", e);
                    }
                    detect_bbox(doc, page_num)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::margins::Margins;

    // Note: These tests would require actual PDF files to test properly.
    // For now, we'll just test that the API compiles and basic types work.

    #[test]
    fn test_crop_options_default() {
        let options = CropOptions::default();
        assert_eq!(options.margins, Margins::none());
        assert!(options.bbox_override.is_none());
        assert!(!options.verbose);
    }

    #[test]
    fn test_crop_options_with_margins() {
        let options = CropOptions {
            margins: Margins::uniform(10.0),
            ..Default::default()
        };
        assert_eq!(options.margins.left, 10.0);
    }
}
