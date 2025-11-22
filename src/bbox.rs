//! Bounding box detection and manipulation

use crate::error::{Error, Result};
use lopdf::Document;

/// A bounding box in PDF coordinates (origin at bottom-left)
///
/// Coordinates are in points (1/72 inch)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    /// Left edge (x minimum)
    pub left: f64,
    /// Bottom edge (y minimum)
    pub bottom: f64,
    /// Right edge (x maximum)
    pub right: f64,
    /// Top edge (y maximum)
    pub top: f64,
}

impl BoundingBox {
    /// Create a new bounding box
    pub fn new(left: f64, bottom: f64, right: f64, top: f64) -> Result<Self> {
        if left >= right {
            return Err(Error::InvalidBoundingBox(format!(
                "left ({}) must be less than right ({})",
                left, right
            )));
        }
        if bottom >= top {
            return Err(Error::InvalidBoundingBox(format!(
                "bottom ({}) must be less than top ({})",
                bottom, top
            )));
        }

        Ok(Self {
            left,
            bottom,
            right,
            top,
        })
    }

    /// Parse bounding box from string specification
    ///
    /// Format: "left bottom right top"
    /// Example: "10 20 200 280"
    pub fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split_whitespace().collect();

        if parts.len() != 4 {
            return Err(Error::InvalidBoundingBox(format!(
                "expected 4 values, got {}",
                parts.len()
            )));
        }

        let left = parts[0]
            .parse::<f64>()
            .map_err(|e| Error::InvalidBoundingBox(format!("invalid left value: {}", e)))?;
        let bottom = parts[1]
            .parse::<f64>()
            .map_err(|e| Error::InvalidBoundingBox(format!("invalid bottom value: {}", e)))?;
        let right = parts[2]
            .parse::<f64>()
            .map_err(|e| Error::InvalidBoundingBox(format!("invalid right value: {}", e)))?;
        let top = parts[3]
            .parse::<f64>()
            .map_err(|e| Error::InvalidBoundingBox(format!("invalid top value: {}", e)))?;

        Self::new(left, bottom, right, top)
    }

    /// Get width of the bounding box
    pub fn width(&self) -> f64 {
        self.right - self.left
    }

    /// Get height of the bounding box
    pub fn height(&self) -> f64 {
        self.top - self.bottom
    }

    /// Expand bounding box by margins
    pub fn with_margins(&self, margins: &crate::margins::Margins) -> Self {
        Self {
            left: self.left - margins.left,
            bottom: self.bottom - margins.bottom,
            right: self.right + margins.right,
            top: self.top + margins.top,
        }
    }

    /// Ensure bounding box doesn't exceed page bounds
    pub fn clamp_to_page(&self, page_width: f64, page_height: f64) -> Self {
        Self {
            left: self.left.max(0.0),
            bottom: self.bottom.max(0.0),
            right: self.right.min(page_width),
            top: self.top.min(page_height),
        }
    }
}

/// Detect bounding box of content on a PDF page
///
/// This function parses the page's content stream to find all drawing operations
/// and calculates the minimum bounding box that contains all content.
pub fn detect_bbox(doc: &Document, page_num: usize) -> Result<BoundingBox> {
    // Get the page
    let page_id = doc
        .page_iter()
        .nth(page_num)
        .ok_or_else(|| Error::InvalidPage(format!("page {} not found", page_num)))?;

    // Get page object
    let page = doc
        .get_object(page_id)
        .map_err(|e| Error::PdfParse(format!("failed to get page {}: {}", page_num, e)))?
        .as_dict()
        .map_err(|e| Error::PdfParse(format!("page {} is not a dictionary: {}", page_num, e)))?;

    // Get content stream
    let content_data = doc.get_page_content(page_id)
        .map_err(|e| Error::ContentStreamParse(format!("failed to get content stream: {}", e)))?;

    // Parse content stream to detect bounding box
    // For now, we'll use a simple approach: extract all numeric coordinates
    // from common drawing operations (text positioning, line drawing, etc.)
    let bbox = parse_content_stream(&content_data, &page)?;

    if bbox.width() <= 0.0 || bbox.height() <= 0.0 {
        return Err(Error::EmptyPage(page_num));
    }

    Ok(bbox)
}

/// 2D transformation matrix for PDF coordinates
#[derive(Debug, Clone, Copy)]
struct Matrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Matrix {
    /// Identity matrix
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Transform a point using this matrix
    fn transform_point(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// Concatenate with another matrix (this * other)
    fn concat(&self, other: &Matrix) -> Self {
        Self {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }
}

/// Parse PDF content stream to extract bounding box
///
/// This parser tracks coordinates from common PDF operators and applies
/// coordinate transformations (CTM - Current Transformation Matrix).
///
/// Operators handled:
/// - Graphics state: q, Q, cm
/// - Path construction: m, l, c, v, y, re
/// - Text positioning: Tm, Td, TD
fn parse_content_stream(content: &[u8], page: &lopdf::Dictionary) -> Result<BoundingBox> {
    use lopdf::content::Content;

    let content = Content::decode(content)
        .map_err(|e| Error::ContentStreamParse(format!("failed to decode content: {}", e)))?;

    // Get MediaBox to use as bounds for clamping coordinates
    // This prevents including coordinates from transformed paths that fall outside the visible page
    let media_box = get_media_box(page)?;
    let page_min_x = media_box.left;
    let page_min_y = media_box.bottom;
    let page_max_x = media_box.right;
    let page_max_y = media_box.top;

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    // Track text bounds separately to detect decorative graphics
    let mut text_min_x = f64::INFINITY;
    let mut text_max_x = f64::NEG_INFINITY;

    // Current Transformation Matrix
    let mut ctm = Matrix::identity();
    let mut ctm_stack: Vec<Matrix> = Vec::new();

    // Text state
    let mut text_matrix = Matrix::identity();
    let mut text_font_size: f64 = 12.0; // Default font size
    let mut text_line_matrix = Matrix::identity();

    // Path state - track if we're building a path that will be painted
    let mut path_bbox: Option<(f64, f64, f64, f64)> = None; // (min_x, min_y, max_x, max_y)

    // Clipping state - track the current clipping region
    // If set, only content within this region is visible
    let mut clip_bbox: Option<(f64, f64, f64, f64)> = None;
    let mut clip_stack: Vec<Option<(f64, f64, f64, f64)>> = Vec::new();

    for operation in &content.operations {
        match operation.operator.as_ref() {
            // Graphics state operators
            "q" => {
                // Save graphics state (including CTM and clipping path)
                ctm_stack.push(ctm);
                clip_stack.push(clip_bbox);
            }
            "Q" => {
                // Restore graphics state
                if let Some(saved_ctm) = ctm_stack.pop() {
                    ctm = saved_ctm;
                }
                if let Some(saved_clip) = clip_stack.pop() {
                    clip_bbox = saved_clip;
                }
            }
            "cm" => {
                // Modify CTM: a b c d e f
                if operation.operands.len() >= 6 {
                    if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                        get_number(&operation, 0),
                        get_number(&operation, 1),
                        get_number(&operation, 2),
                        get_number(&operation, 3),
                        get_number(&operation, 4),
                        get_number(&operation, 5),
                    ) {
                        let new_matrix = Matrix { a, b, c, d, e, f };
                        ctm = ctm.concat(&new_matrix);
                    }
                }
            }

            // Path construction operators - build path but don't commit to bbox yet
            "m" | "l" => {
                // moveto / lineto: x y
                if let (Some(x), Some(y)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                    let (tx, ty) = ctm.transform_point(x, y);
                    add_to_path_bbox(&mut path_bbox, tx, ty, page_min_x, page_min_y, page_max_x, page_max_y);
                }
            }
            "re" => {
                // rectangle: x y width height
                if let (Some(x), Some(y), Some(w), Some(h)) = (
                    get_number(&operation, 0),
                    get_number(&operation, 1),
                    get_number(&operation, 2),
                    get_number(&operation, 3),
                ) {
                    // Transform all four corners
                    let (x1, y1) = ctm.transform_point(x, y);
                    let (x2, y2) = ctm.transform_point(x + w, y);
                    let (x3, y3) = ctm.transform_point(x, y + h);
                    let (x4, y4) = ctm.transform_point(x + w, y + h);

                    add_to_path_bbox(&mut path_bbox, x1, y1, page_min_x, page_min_y, page_max_x, page_max_y);
                    add_to_path_bbox(&mut path_bbox, x2, y2, page_min_x, page_min_y, page_max_x, page_max_y);
                    add_to_path_bbox(&mut path_bbox, x3, y3, page_min_x, page_min_y, page_max_x, page_max_y);
                    add_to_path_bbox(&mut path_bbox, x4, y4, page_min_x, page_min_y, page_max_x, page_max_y);
                }
            }
            "c" | "v" | "y" => {
                // Bezier curves: c = x1 y1 x2 y2 x3 y3, v = x2 y2 x3 y3, y = x1 y1 x3 y3
                let count = operation.operands.len() / 2;
                for i in 0..count {
                    if let (Some(x), Some(y)) = (get_number(&operation, i * 2), get_number(&operation, i * 2 + 1)) {
                        let (tx, ty) = ctm.transform_point(x, y);
                        add_to_path_bbox(&mut path_bbox, tx, ty, page_min_x, page_min_y, page_max_x, page_max_y);
                    }
                }
            }

            // Path painting operators - commit path bbox to final bbox
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                // Stroke/fill operators - the path is actually painted
                // Only include paths that are within the clipping region (if any)
                if let Some((path_min_x, path_min_y, path_max_x, path_max_y)) = path_bbox {
                    let visible_bbox = if let Some(clip) = clip_bbox {
                        // Intersect path with clipping region
                        intersect_bbox(Some((path_min_x, path_min_y, path_max_x, path_max_y)), Some(clip))
                    } else {
                        (path_min_x, path_min_y, path_max_x, path_max_y)
                    };

                    // Only add if the intersection is valid (non-empty)
                    let (vis_min_x, vis_min_y, vis_max_x, vis_max_y) = visible_bbox;
                    if vis_min_x <= vis_max_x && vis_min_y <= vis_max_y {
                        min_x = min_x.min(vis_min_x);
                        min_y = min_y.min(vis_min_y);
                        max_x = max_x.max(vis_max_x);
                        max_y = max_y.max(vis_max_y);
                    }
                }
                path_bbox = None; // Clear path
            }
            "n" => {
                // End path without painting - discard the path
                path_bbox = None;
            }
            "W" | "W*" => {
                // Set clipping path from current path
                if let Some(bbox) = path_bbox {
                    clip_bbox = Some(intersect_bbox(clip_bbox, Some(bbox)));
                }
                // Note: path is NOT cleared - W/W* modifies clip but path continues
            }

            // Text operators
            "BT" => {
                // Begin text - reset text matrix and line matrix
                text_matrix = ctm;
                text_line_matrix = ctm;
            }
            "Tf" => {
                // Set font and size: font_name size
                if let Some(size) = get_number(&operation, 1) {
                    text_font_size = size.abs(); // Font size can be negative for some transformations
                }
            }
            "Tm" => {
                // Set text matrix: a b c d e f
                // This just positions the text cursor, doesn't render anything
                if operation.operands.len() >= 6 {
                    if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                        get_number(&operation, 0),
                        get_number(&operation, 1),
                        get_number(&operation, 2),
                        get_number(&operation, 3),
                        get_number(&operation, 4),
                        get_number(&operation, 5),
                    ) {
                        text_matrix = Matrix { a, b, c, d, e, f };
                        text_line_matrix = text_matrix;
                        // Don't add to bbox - Tm just positions cursor, doesn't show text
                    }
                }
            }
            "Td" | "TD" => {
                // Text positioning: tx ty
                // This just moves the text cursor, doesn't render anything
                if let (Some(tx), Some(ty)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                    let translation = Matrix {
                        a: 1.0,
                        b: 0.0,
                        c: 0.0,
                        d: 1.0,
                        e: tx,
                        f: ty,
                    };
                    text_line_matrix = text_line_matrix.concat(&translation);
                    text_matrix = text_line_matrix;
                    // Don't add to bbox - Td just moves cursor, doesn't show text
                }
            }
            "T*" => {
                // Move to start of next line (same as "Td 0 -TL" where TL is leading)
                // This just moves the text cursor, doesn't render anything
                let leading = text_font_size * 1.2;
                let translation = Matrix {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: -leading,
                };
                text_line_matrix = text_line_matrix.concat(&translation);
                text_matrix = text_line_matrix;
                // Don't add to bbox - T* just moves cursor to next line
            }
            "Tj" | "'" => {
                // Show text string - add bounding box with estimated width (only if in clip region AND baseline on-page)
                // Check that text baseline is within page bounds (not just the text bbox)
                // This prevents including text that starts off-page but extends slightly onto page
                const TEXT_TOLERANCE: f64 = 0.0; // Strict: baseline must be on-page
                const CORNER_HORIZ_MARGIN: f64 = 40.0; // Corner detection: horizontal
                const CORNER_VERT_MARGIN: f64 = 40.0; // Corner detection: vertical

                // Exclude text in corners
                let near_left_edge = text_matrix.e < page_min_x + CORNER_HORIZ_MARGIN;
                let near_right_edge = text_matrix.e > page_max_x - CORNER_HORIZ_MARGIN;
                let near_top_edge = text_matrix.f > page_max_y - CORNER_VERT_MARGIN;
                let near_bottom_edge = text_matrix.f < page_min_y + CORNER_VERT_MARGIN;

                let in_corner = (near_left_edge || near_right_edge) && (near_top_edge || near_bottom_edge);

                // Also exclude diagram labels: very short text OR small fonts near edges
                let is_very_short = if let Some(obj) = operation.operands.get(0) {
                    obj.as_str().ok().map(|s| s.len() <= 2).unwrap_or(false)
                } else {
                    false
                };
                let is_small_font = text_font_size < 9.5; // Diagram labels typically < 10pt
                let near_left_edge_48 = text_matrix.e < page_min_x + 48.5;
                let near_right_edge_40 = text_matrix.e > page_max_x - 40.0;

                let diagram_label = (is_very_short || is_small_font) && (near_left_edge_48 || near_right_edge_40);

                let too_close_to_edge = in_corner || diagram_label;

                if !too_close_to_edge &&
                   is_in_clip(text_matrix.e, text_matrix.f, clip_bbox) &&
                   text_matrix.e >= page_min_x - TEXT_TOLERANCE && text_matrix.e <= page_max_x + TEXT_TOLERANCE &&
                   text_matrix.f >= page_min_y - TEXT_TOLERANCE && text_matrix.f <= page_max_y + TEXT_TOLERANCE {
                    // Track text bounds separately
                    text_min_x = text_min_x.min(text_matrix.e);
                    if let Some(text_width) = estimate_text_width_tj(&operation, text_font_size) {
                        let text_end_x = text_matrix.e + text_matrix.a * text_width;
                        text_max_x = text_max_x.max(text_end_x);
                        add_text_bounds_with_width(&mut min_x, &mut min_y, &mut max_x, &mut max_y,
                                                  &text_matrix, text_font_size, text_width,
                                                  page_min_x, page_min_y, page_max_x, page_max_y);
                    } else {
                        text_max_x = text_max_x.max(text_matrix.e);
                        add_text_bounds(&mut min_x, &mut min_y, &mut max_x, &mut max_y,
                                      &text_matrix, text_font_size,
                                      page_min_x, page_min_y, page_max_x, page_max_y);
                    }
                }
            }
            "TJ" => {
                // Show text with positioning - add bounding box with estimated width (only if in clip region AND baseline on-page)
                const TEXT_TOLERANCE: f64 = 0.0;
                const CORNER_HORIZ_MARGIN: f64 = 40.0;
                const CORNER_VERT_MARGIN: f64 = 40.0;

                let near_left_edge = text_matrix.e < page_min_x + CORNER_HORIZ_MARGIN;
                let near_right_edge = text_matrix.e > page_max_x - CORNER_HORIZ_MARGIN;
                let near_top_edge = text_matrix.f > page_max_y - CORNER_VERT_MARGIN;
                let near_bottom_edge = text_matrix.f < page_min_y + CORNER_VERT_MARGIN;

                let in_corner = (near_left_edge || near_right_edge) && (near_top_edge || near_bottom_edge);

                // Exclude diagram labels: very short text OR small fonts near edges
                let is_very_short = if let Some(obj) = operation.operands.get(0) {
                    if let Ok(array) = obj.as_array() {
                        let total_chars: usize = array.iter()
                            .filter_map(|item| item.as_str().ok())
                            .map(|s| s.len())
                            .sum();
                        total_chars <= 2
                    } else {
                        false
                    }
                } else {
                    false
                };
                let is_small_font = text_font_size < 9.5;
                let near_left_edge_48 = text_matrix.e < page_min_x + 48.5;
                let near_right_edge_40 = text_matrix.e > page_max_x - 40.0;

                let diagram_label = (is_very_short || is_small_font) && (near_left_edge_48 || near_right_edge_40);

                let too_close_to_edge = in_corner || diagram_label;

                if !too_close_to_edge &&
                   is_in_clip(text_matrix.e, text_matrix.f, clip_bbox) &&
                   text_matrix.e >= page_min_x - TEXT_TOLERANCE && text_matrix.e <= page_max_x + TEXT_TOLERANCE &&
                   text_matrix.f >= page_min_y - TEXT_TOLERANCE && text_matrix.f <= page_max_y + TEXT_TOLERANCE {
                    // Track text bounds separately
                    text_min_x = text_min_x.min(text_matrix.e);
                    if let Some(text_width) = estimate_text_width_tj_array(&operation, text_font_size) {
                        let text_end_x = text_matrix.e + text_matrix.a * text_width;
                        text_max_x = text_max_x.max(text_end_x);
                        add_text_bounds_with_width(&mut min_x, &mut min_y, &mut max_x, &mut max_y,
                                                  &text_matrix, text_font_size, text_width,
                                                  page_min_x, page_min_y, page_max_x, page_max_y);
                    } else {
                        text_max_x = text_max_x.max(text_matrix.e);
                        add_text_bounds(&mut min_x, &mut min_y, &mut max_x, &mut max_y,
                                      &text_matrix, text_font_size,
                                      page_min_x, page_min_y, page_max_x, page_max_y);
                    }
                }
            }
            "\"" => {
                // Set spacing and show text (only if in clip region AND baseline on-page)
                const TEXT_TOLERANCE: f64 = 0.0;
                const CORNER_HORIZ_MARGIN: f64 = 40.0;
                const CORNER_VERT_MARGIN: f64 = 40.0;

                let near_left_edge = text_matrix.e < page_min_x + CORNER_HORIZ_MARGIN;
                let near_right_edge = text_matrix.e > page_max_x - CORNER_HORIZ_MARGIN;
                let near_top_edge = text_matrix.f > page_max_y - CORNER_VERT_MARGIN;
                let near_bottom_edge = text_matrix.f < page_min_y + CORNER_VERT_MARGIN;

                let in_corner = (near_left_edge || near_right_edge) && (near_top_edge || near_bottom_edge);

                // Exclude diagram labels: very short text OR small fonts near edges
                let is_very_short = if let Some(obj) = operation.operands.get(0) {
                    if let Ok(array) = obj.as_array() {
                        let total_chars: usize = array.iter()
                            .filter_map(|item| item.as_str().ok())
                            .map(|s| s.len())
                            .sum();
                        total_chars <= 2
                    } else {
                        false
                    }
                } else {
                    false
                };
                let is_small_font = text_font_size < 9.5;
                let near_left_edge_48 = text_matrix.e < page_min_x + 48.5;
                let near_right_edge_40 = text_matrix.e > page_max_x - 40.0;

                let diagram_label = (is_very_short || is_small_font) && (near_left_edge_48 || near_right_edge_40);

                let too_close_to_edge = in_corner || diagram_label;

                if !too_close_to_edge &&
                   is_in_clip(text_matrix.e, text_matrix.f, clip_bbox) &&
                   text_matrix.e >= page_min_x - TEXT_TOLERANCE && text_matrix.e <= page_max_x + TEXT_TOLERANCE &&
                   text_matrix.f >= page_min_y - TEXT_TOLERANCE && text_matrix.f <= page_max_y + TEXT_TOLERANCE {
                    // Track text bounds separately
                    text_min_x = text_min_x.min(text_matrix.e);
                    if let Some(text_width) = estimate_text_width_tj(&operation, text_font_size) {
                        let text_end_x = text_matrix.e + text_matrix.a * text_width;
                        text_max_x = text_max_x.max(text_end_x);
                        add_text_bounds_with_width(&mut min_x, &mut min_y, &mut max_x, &mut max_y,
                                                  &text_matrix, text_font_size, text_width,
                                                  page_min_x, page_min_y, page_max_x, page_max_y);
                    } else {
                        text_max_x = text_max_x.max(text_matrix.e);
                        add_text_bounds(&mut min_x, &mut min_y, &mut max_x, &mut max_y,
                                      &text_matrix, text_font_size,
                                      page_min_x, page_min_y, page_max_x, page_max_y);
                    }
                }
            }
            _ => {}
        }
    }

    // If we didn't find any content, fall back to MediaBox
    if min_x == f64::INFINITY {
        return get_media_box(page);
    }

    // Heuristic: If there's a significant gap between graphics bounds and text bounds,
    // prefer text bounds (graphics might be decorative/white elements)
    // This helps exclude decorative graphics that Ghostscript's rendering would filter
    const GAP_THRESHOLD: f64 = 15.0; // Points

    if text_min_x != f64::INFINITY {
        // We have text content
        let left_gap = text_min_x - min_x;
        let right_gap = max_x - text_max_x;

        // If graphics extends significantly beyond text on left, use text left edge
        if left_gap > GAP_THRESHOLD {
            min_x = text_min_x;
        }

        // If graphics extends significantly beyond text on right, use text right edge
        if right_gap > GAP_THRESHOLD {
            max_x = text_max_x;
        }
    }

    BoundingBox::new(min_x, min_y, max_x, max_y)
}

/// Helper function to extract a number from an operation's operands
fn get_number(op: &lopdf::content::Operation, index: usize) -> Option<f64> {
    op.operands.get(index).and_then(|obj| {
        obj.as_f32().ok().map(|f| f as f64).or_else(|| obj.as_i64().ok().map(|i| i as f64))
    })
}

/// Estimate text width from Tj operator (single string)
fn estimate_text_width_tj(op: &lopdf::content::Operation, font_size: f64) -> Option<f64> {
    // Tj and ' operators: operands[0] or operands[2] is the text string
    let text_obj = if op.operator == "Tj" || op.operator == "'" {
        op.operands.get(0)
    } else if op.operator == "\"" {
        // " operator has word_space, char_space, then string
        op.operands.get(2)
    } else {
        return None;
    };

    if let Some(obj) = text_obj {
        // Try to get string length
        if let Ok(bytes) = obj.as_str() {
            let char_count = bytes.len() as f64;
            // Conservative character width estimate
            // Proportional fonts average ~0.45em, but we use 0.25em to avoid over-estimation
            // (Better to slightly under-crop than leave wide margins)
            return Some(char_count * font_size * 0.25);
        }
    }
    None
}

/// Estimate text width from TJ operator (array of strings and positioning)
fn estimate_text_width_tj_array(op: &lopdf::content::Operation, font_size: f64) -> Option<f64> {
    if let Some(obj) = op.operands.get(0) {
        if let Ok(array) = obj.as_array() {
            let mut total_chars = 0;
            for item in array {
                // Array can contain strings (text) and numbers (positioning adjustments)
                if let Ok(bytes) = item.as_str() {
                    total_chars += bytes.len();
                }
            }
            if total_chars > 0 {
                return Some(total_chars as f64 * font_size * 0.25);
            }
        }
    }
    None
}

/// Intersect two bounding boxes (returns the overlap region)
fn intersect_bbox(
    bbox1: Option<(f64, f64, f64, f64)>,
    bbox2: Option<(f64, f64, f64, f64)>,
) -> (f64, f64, f64, f64) {
    match (bbox1, bbox2) {
        (Some((min_x1, min_y1, max_x1, max_y1)), Some((min_x2, min_y2, max_x2, max_y2))) => {
            // Return intersection
            (
                min_x1.max(min_x2),
                min_y1.max(min_y2),
                max_x1.min(max_x2),
                max_y1.min(max_y2),
            )
        }
        (None, Some(bbox)) | (Some(bbox), None) => bbox,
        (None, None) => (0.0, 0.0, 0.0, 0.0),
    }
}

/// Check if a point is within the clipping region
fn is_in_clip(x: f64, y: f64, clip: Option<(f64, f64, f64, f64)>) -> bool {
    match clip {
        Some((min_x, min_y, max_x, max_y)) => {
            x >= min_x && x <= max_x && y >= min_y && y <= max_y
        }
        None => true, // No clipping
    }
}

/// Add point to path bounding box (for paths that may be painted)
fn add_to_path_bbox(
    path_bbox: &mut Option<(f64, f64, f64, f64)>,
    x: f64,
    y: f64,
    page_min_x: f64,
    page_min_y: f64,
    page_max_x: f64,
    page_max_y: f64,
) {
    // Filter out-of-bounds coordinates
    const TOLERANCE: f64 = 1.0;
    if x < page_min_x - TOLERANCE || x > page_max_x + TOLERANCE ||
       y < page_min_y - TOLERANCE || y > page_max_y + TOLERANCE {
        return;
    }

    match path_bbox {
        Some((min_x, min_y, max_x, max_y)) => {
            *min_x = min_x.min(x);
            *min_y = min_y.min(y);
            *max_x = max_x.max(x);
            *max_y = max_y.max(y);
        }
        None => {
            *path_bbox = Some((x, y, x, y));
        }
    }
}

/// Add text bounding box at current text matrix position with known width
fn add_text_bounds_with_width(
    min_x: &mut f64,
    min_y: &mut f64,
    max_x: &mut f64,
    max_y: &mut f64,
    text_matrix: &Matrix,
    font_size: f64,
    text_width: f64,
    page_min_x: f64,
    page_min_y: f64,
    page_max_x: f64,
    page_max_y: f64,
) {
    // Text baseline position
    let baseline_x = text_matrix.e;
    let baseline_y = text_matrix.f;

    // Standard font metrics (approximate)
    const ASCENDER_RATIO: f64 = 0.75;
    const DESCENDER_RATIO: f64 = 0.25;

    let ascender_height = font_size * ASCENDER_RATIO;
    let descender_depth = font_size * DESCENDER_RATIO;

    // Apply text matrix transformation to text width displacement vector
    // Only use the linear transformation part (a, b, c, d), not translation (e, f)
    let displaced_x = text_matrix.a * text_width;
    let displaced_y = text_matrix.b * text_width;
    let text_end_x = baseline_x + displaced_x;
    let text_end_y = baseline_y + displaced_y;

    // Calculate all four corners of text bounding box
    // Bottom-left
    let (x1, y1) = (baseline_x, baseline_y - descender_depth);
    // Bottom-right
    let (x2, y2) = (text_end_x, text_end_y - descender_depth);
    // Top-left
    let (x3, y3) = (baseline_x, baseline_y + ascender_height);
    // Top-right
    let (x4, y4) = (text_end_x, text_end_y + ascender_height);

    // Update bounds with all corners
    update_bounds(min_x, min_y, max_x, max_y, x1, y1, page_min_x, page_min_y, page_max_x, page_max_y);
    update_bounds(min_x, min_y, max_x, max_y, x2, y2, page_min_x, page_min_y, page_max_x, page_max_y);
    update_bounds(min_x, min_y, max_x, max_y, x3, y3, page_min_x, page_min_y, page_max_x, page_max_y);
    update_bounds(min_x, min_y, max_x, max_y, x4, y4, page_min_x, page_min_y, page_max_x, page_max_y);
}

/// Add text bounding box at current text matrix position
///
/// Estimates text bounds using standard font metrics:
/// - Baseline at (e, f) from text matrix
/// - Ascender height: ~0.75 * font_size above baseline
/// - Descender depth: ~0.25 * font_size below baseline
fn add_text_bounds(
    min_x: &mut f64,
    min_y: &mut f64,
    max_x: &mut f64,
    max_y: &mut f64,
    text_matrix: &Matrix,
    font_size: f64,
    page_min_x: f64,
    page_min_y: f64,
    page_max_x: f64,
    page_max_y: f64,
) {
    // Fallback when we can't determine text width - use small estimate
    add_text_bounds_with_width(min_x, min_y, max_x, max_y, text_matrix, font_size,
                               font_size * 0.25, page_min_x, page_min_y, page_max_x, page_max_y);
}

/// Update bounding box bounds with a new point
///
/// Coordinates outside the page bounds are ignored to exclude content that falls
/// outside the visible page area (e.g., from rotated/transformed paths)
fn update_bounds(
    min_x: &mut f64,
    min_y: &mut f64,
    max_x: &mut f64,
    max_y: &mut f64,
    x: f64,
    y: f64,
    page_min_x: f64,
    page_min_y: f64,
    page_max_x: f64,
    page_max_y: f64,
) {
    // Allow a small tolerance (1pt) outside MediaBox for rounding errors
    const TOLERANCE: f64 = 1.0;

    // Skip coordinates that fall outside page bounds (with tolerance)
    if x < page_min_x - TOLERANCE || x > page_max_x + TOLERANCE ||
       y < page_min_y - TOLERANCE || y > page_max_y + TOLERANCE {
        return;
    }

    *min_x = min_x.min(x);
    *min_y = min_y.min(y);
    *max_x = max_x.max(x);
    *max_y = max_y.max(y);
}

/// Get the MediaBox from a page dictionary
fn get_media_box(page: &lopdf::Dictionary) -> Result<BoundingBox> {
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

    BoundingBox::new(left, bottom, right, top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bbox_new() {
        let bbox = BoundingBox::new(10.0, 20.0, 100.0, 200.0).unwrap();
        assert_eq!(bbox.left, 10.0);
        assert_eq!(bbox.bottom, 20.0);
        assert_eq!(bbox.right, 100.0);
        assert_eq!(bbox.top, 200.0);
    }

    #[test]
    fn test_bbox_invalid() {
        assert!(BoundingBox::new(100.0, 20.0, 10.0, 200.0).is_err());
        assert!(BoundingBox::new(10.0, 200.0, 100.0, 20.0).is_err());
    }

    #[test]
    fn test_bbox_dimensions() {
        let bbox = BoundingBox::new(10.0, 20.0, 110.0, 220.0).unwrap();
        assert_eq!(bbox.width(), 100.0);
        assert_eq!(bbox.height(), 200.0);
    }

    #[test]
    fn test_bbox_from_str() {
        let bbox = BoundingBox::from_str("10 20 100 200").unwrap();
        assert_eq!(bbox.left, 10.0);
        assert_eq!(bbox.bottom, 20.0);
        assert_eq!(bbox.right, 100.0);
        assert_eq!(bbox.top, 200.0);
    }

    #[test]
    fn test_bbox_with_margins() {
        use crate::margins::Margins;
        let bbox = BoundingBox::new(10.0, 20.0, 100.0, 200.0).unwrap();
        let margins = Margins::uniform(5.0);
        let expanded = bbox.with_margins(&margins);

        assert_eq!(expanded.left, 5.0);
        assert_eq!(expanded.bottom, 15.0);
        assert_eq!(expanded.right, 105.0);
        assert_eq!(expanded.top, 205.0);
    }
}
