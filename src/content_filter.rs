//! Content stream filtering to remove elements outside crop box
//!
//! This module provides functionality to analyze PDF content streams and remove
//! drawing operations that fall completely outside the crop box, improving
//! privacy/security by ensuring clipped content is actually removed from the file.

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};
use lopdf::{
    content::{Content, Operation},
    Dictionary, Document, Object, ObjectId, Stream,
};
use std::collections::HashMap;

/// Graphics state for tracking transformations and positions
#[derive(Debug, Clone)]
struct GraphicsState {
    /// Current transformation matrix [a b c d e f]
    ctm: [f64; 6],
    /// Current text matrix
    text_matrix: [f64; 6],
    /// Current text line matrix (used for T*, Td, TD)
    text_line_matrix: [f64; 6],
    /// Current text position
    text_pos: (f64, f64),
    /// Current font size (from Tf operator)
    font_size: f64,
    /// Current font name (from Tf operator)
    font_name: Option<Vec<u8>>,
    /// Character spacing (Tc)
    char_spacing: f64,
    /// Word spacing (Tw)
    word_spacing: f64,
    /// Horizontal scaling (Tz) expressed as factor (1.0 == 100%)
    horiz_scaling: f64,
    /// Text leading (TL)
    leading: f64,
    /// Text rise (Ts)
    text_rise: f64,
}

impl Default for GraphicsState {
    fn default() -> Self {
        GraphicsState {
            ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], // Identity matrix
            text_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            text_line_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            text_pos: (0.0, 0.0),
            font_size: 12.0,
            font_name: None,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horiz_scaling: 1.0,
            leading: 0.0,
            text_rise: 0.0,
        }
    }
}

impl GraphicsState {
    /// Apply a transformation matrix to the CTM
    fn apply_transform(&mut self, matrix: &[f64; 6]) {
        let [a1, b1, c1, d1, e1, f1] = self.ctm;
        let [a2, b2, c2, d2, e2, f2] = matrix;

        self.ctm = [
            a1 * a2 + b1 * c2,
            a1 * b2 + b1 * d2,
            c1 * a2 + d1 * c2,
            c1 * b2 + d1 * d2,
            e1 * a2 + f1 * c2 + e2,
            e1 * b2 + f1 * d2 + f2,
        ];
    }

    /// Transform a point from user space to device space
    fn transform_point(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.ctm;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// Reset text-related state when entering BT or when text resources change
    fn reset_text_state(&mut self) {
        self.text_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        self.text_line_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        self.text_pos = (0.0, 0.0);
        self.char_spacing = 0.0;
        self.word_spacing = 0.0;
        self.horiz_scaling = 1.0;
        self.leading = 0.0;
        self.text_rise = 0.0;
    }

    /// Set current text matrix and update cached position/line matrix
    fn set_text_matrix(&mut self, matrix: [f64; 6]) {
        self.text_matrix = matrix;
        self.text_line_matrix = matrix;
        self.update_text_position();
    }

    /// Translate current text matrix by tx, ty
    fn translate_text_matrix(&mut self, tx: f64, ty: f64) {
        let translation = [1.0, 0.0, 0.0, 1.0, tx, ty];
        self.text_matrix = multiply_matrices(&self.text_matrix, &translation);
        self.update_text_position();
    }

    /// Translate text line matrix (affects T*, Td, TD)
    fn translate_text_line_matrix(&mut self, tx: f64, ty: f64) {
        let translation = [1.0, 0.0, 0.0, 1.0, tx, ty];
        self.text_line_matrix = multiply_matrices(&self.text_line_matrix, &translation);
        self.text_matrix = self.text_line_matrix;
        self.update_text_position();
    }

    /// Move to start of next line (T*)
    fn move_to_next_line(&mut self) {
        let ty = -self.leading;
        self.translate_text_line_matrix(0.0, ty);
    }

    /// Get combined text matrix (text matrix composed with CTM)
    fn combined_text_matrix(&self) -> [f64; 6] {
        multiply_matrices(&self.ctm, &self.text_matrix)
    }

    fn update_text_position(&mut self) {
        self.text_pos = (self.text_matrix[4], self.text_matrix[5]);
    }
}

/// Cached font metrics for calculating text bounding boxes
#[derive(Clone, Debug)]
struct FontMetrics {
    widths: HashMap<u32, f64>,
    default_width: f64,
    ascent: f64,
    descent: f64,
    is_cid: bool,
    bytes_per_char: usize,
    writing_mode: WritingMode,
}

impl FontMetrics {
    fn glyph_width(&self, code: u32) -> f64 {
        self.widths
            .get(&code)
            .copied()
            .unwrap_or(self.default_width)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WritingMode {
    Horizontal,
    Vertical,
}

/// Lazy font metrics cache keyed by font resource name
#[derive(Default)]
struct FontCache {
    cache: HashMap<Vec<u8>, FontMetrics>,
}

impl FontCache {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    fn get(
        &mut self,
        doc: &Document,
        resources: Option<&Dictionary>,
        font_name: &[u8],
    ) -> Option<FontMetrics> {
        if let Some(metrics) = self.cache.get(font_name) {
            return Some(metrics.clone());
        }

        let metrics = load_font_metrics(doc, resources, font_name)?;
        self.cache.insert(font_name.to_vec(), metrics.clone());
        Some(metrics)
    }
}

fn load_font_metrics(
    doc: &Document,
    resources: Option<&Dictionary>,
    font_name: &[u8],
) -> Option<FontMetrics> {
    let font_dict = get_font_dictionary(doc, resources, font_name)?;
    let subtype = font_dict
        .get(b"Subtype")
        .ok()
        .and_then(|obj| obj.as_name().ok())?;

    match subtype {
        b"Type0" => parse_type0_font(doc, &font_dict),
        b"Type1" | b"TrueType" => parse_type1_font(doc, &font_dict),
        b"Type3" => parse_type3_font(doc, &font_dict),
        _ => None,
    }
}

fn get_font_dictionary(
    doc: &Document,
    resources: Option<&Dictionary>,
    font_name: &[u8],
) -> Option<Dictionary> {
    let resources = resources?;
    let font_entry = resources.get(b"Font").ok()?;
    let font_dict_obj = resolve_to_owned(doc, font_entry)?;
    let font_dict = font_dict_obj.as_dict().ok()?;
    let font_obj = font_dict.get(font_name).ok()?.clone();
    match resolve_to_owned(doc, &font_obj)? {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(stream.dict),
        _ => None,
    }
}

fn resolve_to_owned(doc: &Document, obj: &Object) -> Option<Object> {
    match obj {
        Object::Reference(id) => doc.get_object(*id).ok().cloned(),
        other => Some(other.clone()),
    }
}

fn parse_type1_font(doc: &Document, font_dict: &Dictionary) -> Option<FontMetrics> {
    let first_char = font_dict
        .get(b"FirstChar")
        .ok()
        .and_then(|obj| obj.as_i64().ok())
        .unwrap_or(0) as u32;

    let widths_obj = font_dict.get(b"Widths").ok()?;
    let widths_array_obj = resolve_to_owned(doc, widths_obj)?;
    let widths_array = widths_array_obj.as_array().ok()?;

    let mut widths = HashMap::new();
    for (idx, value) in widths_array.iter().enumerate() {
        if let Some(width) = object_to_f64(value) {
            widths.insert(first_char + idx as u32, width);
        }
    }

    let descriptor_dict = font_dict
        .get(b"FontDescriptor")
        .ok()
        .and_then(|obj| resolve_to_owned(doc, obj))
        .and_then(|obj| match obj {
            Object::Dictionary(dict) => Some(dict),
            Object::Stream(stream) => Some(stream.dict),
            _ => None,
        });

    let (ascent, descent, missing_width) = descriptor_metrics(descriptor_dict.as_ref());

    Some(FontMetrics {
        widths,
        default_width: missing_width,
        ascent,
        descent,
        is_cid: false,
        bytes_per_char: 1,
        writing_mode: WritingMode::Horizontal,
    })
}

fn parse_type0_font(doc: &Document, font_dict: &Dictionary) -> Option<FontMetrics> {
    // Only handle Identity encodings; detect vertical mode for Identity-V
    let writing_mode = if let Ok(enc_name) = font_dict.get(b"Encoding").and_then(|obj| obj.as_name()) {
        match enc_name {
            b"Identity-H" => WritingMode::Horizontal,
            b"Identity-V" => WritingMode::Vertical,
            _ => return None,
        }
    } else {
        WritingMode::Horizontal
    };

    let descendant_fonts_obj = font_dict.get(b"DescendantFonts").ok()?;
    let descendant_fonts_resolved = resolve_to_owned(doc, descendant_fonts_obj)?;
    let descendant_array = descendant_fonts_resolved.as_array().ok()?;
    let first_descendant = descendant_array.first()?;
    let descendant_dict_obj = resolve_to_owned(doc, first_descendant)?;
    let descendant_dict = match descendant_dict_obj {
        Object::Dictionary(dict) => dict,
        Object::Stream(stream) => stream.dict,
        _ => return None,
    };

    let default_width = descendant_dict
        .get(b"DW")
        .ok()
        .and_then(object_to_f64)
        .unwrap_or(1000.0);

    let mut widths = HashMap::new();
    if let Ok(w_array_obj) = descendant_dict.get(b"W") {
        if let Some(resolved_w_array) = resolve_to_owned(doc, w_array_obj) {
            if let Ok(entries) = resolved_w_array.as_array() {
                parse_cid_widths(entries, &mut widths);
            }
        }
    }

    let descriptor_dict = descendant_dict
        .get(b"FontDescriptor")
        .ok()
        .and_then(|obj| resolve_to_owned(doc, obj))
        .and_then(|obj| match obj {
            Object::Dictionary(dict) => Some(dict),
            Object::Stream(stream) => Some(stream.dict),
            _ => None,
        });

    let (ascent, descent, missing_width) = descriptor_metrics(descriptor_dict.as_ref());

    Some(FontMetrics {
        widths,
        default_width: if default_width > 0.0 {
            default_width
        } else {
            missing_width
        },
        ascent,
        descent,
        is_cid: true,
        bytes_per_char: 2,
        writing_mode,
    })
}

fn parse_type3_font(doc: &Document, font_dict: &Dictionary) -> Option<FontMetrics> {
    // Type3 fonts may not have Widths; fall back to FontBBox width
    let bbox_width = font_dict
        .get(b"FontBBox")
        .ok()
        .and_then(|obj| resolve_to_owned(doc, obj))
        .and_then(|obj| obj.as_array().ok().map(|arr| arr.to_vec()))
        .and_then(|vals| {
            if vals.len() == 4 {
                let left = object_to_f64(&vals[0])?;
                let right = object_to_f64(&vals[2])?;
                Some((right - left).abs())
            } else {
                None
            }
        })
        .unwrap_or(500.0);

    let mut widths = HashMap::new();
    for code in 0..=255u32 {
        widths.insert(code, bbox_width);
    }

    let (ascent, descent, missing_width) =
        descriptor_metrics(font_dict.get(b"FontDescriptor").ok().and_then(|obj| {
            resolve_to_owned(doc, obj).and_then(|o| match o {
                Object::Dictionary(d) => Some(d),
                Object::Stream(s) => Some(s.dict),
                _ => None,
            })
        }).as_ref());

    Some(FontMetrics {
        widths,
        default_width: missing_width.max(bbox_width),
        ascent,
        descent,
        is_cid: false,
        bytes_per_char: 1,
        writing_mode: WritingMode::Horizontal,
    })
}

fn descriptor_metrics(descriptor: Option<&Dictionary>) -> (f64, f64, f64) {
    let ascent = descriptor
        .and_then(|dict| dict.get(b"Ascent").ok())
        .and_then(object_to_f64)
        .unwrap_or(800.0);
    let descent = descriptor
        .and_then(|dict| dict.get(b"Descent").ok())
        .and_then(object_to_f64)
        .unwrap_or(-200.0);
    let missing_width = descriptor
        .and_then(|dict| dict.get(b"MissingWidth").ok())
        .and_then(object_to_f64)
        .unwrap_or(500.0);

    (ascent, descent, missing_width)
}

fn parse_cid_widths(entries: &[Object], widths: &mut HashMap<u32, f64>) {
    let mut idx = 0;
    while idx < entries.len() {
        let start_code = match object_to_u32(&entries[idx]) {
            Some(val) => val,
            None => {
                idx += 1;
                continue;
            }
        };

        if idx + 1 >= entries.len() {
            break;
        }

        match &entries[idx + 1] {
            Object::Array(values) => {
                for (offset, value) in values.iter().enumerate() {
                    if let Some(width) = object_to_f64(value) {
                        widths.insert(start_code + offset as u32, width);
                    }
                }
                idx += 2;
            }
            Object::Integer(_) | Object::Real(_) => {
                if idx + 2 >= entries.len() {
                    break;
                }
                let end_code = match object_to_u32(&entries[idx + 1]) {
                    Some(val) => val,
                    None => {
                        idx += 1;
                        continue;
                    }
                };
                if let Some(width) = object_to_f64(&entries[idx + 2]) {
                    for code in start_code..=end_code {
                        widths.insert(code, width);
                    }
                }
                idx += 3;
            }
            _ => {
                idx += 1;
            }
        }
    }
}

fn object_to_f64(obj: &Object) -> Option<f64> {
    match obj {
        Object::Real(val) => Some(*val as f64),
        Object::Integer(val) => Some(*val as f64),
        _ => None,
    }
}

fn object_to_u32(obj: &Object) -> Option<u32> {
    match obj {
        Object::Integer(val) => {
            if *val >= 0 {
                Some(*val as u32)
            } else {
                None
            }
        }
        Object::Real(val) => {
            if *val >= 0.0 {
                Some(*val as u32)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Represents a filterable component in a PDF content stream
#[derive(Debug)]
enum ContentComponent {
    /// Path operations (path construction + painting operator)
    Path {
        operators: Vec<Operation>,
        bbox: Option<BoundingBox>,
    },
    /// Image XObject (Do operator with Image type)
    ImageXObject {
        operator: Operation,
        bbox: Option<BoundingBox>,
    },
    /// Form XObject (Do operator with Form type) - now with proper bbox calculation
    FormXObject {
        operator: Operation,
        bbox: Option<BoundingBox>,
    },
    /// Text block (BT...ET) - kept without filtering for safety
    TextBlock { operators: Vec<Operation> },
    /// Orphan text operators (Tj/TJ/'/") that appear outside BT/ET blocks
    OrphanText {
        operator: Operation,
        bbox: Option<BoundingBox>,
    },
    /// Graphics state operators (q, Q, cm, colors, line styles) - always kept
    GraphicsState { operators: Vec<Operation> },
}

fn flush_graphics_ops(components: &mut Vec<ContentComponent>, graphics_ops: &mut Vec<Operation>) {
    if !graphics_ops.is_empty() {
        let mut ops = Vec::new();
        std::mem::swap(&mut ops, graphics_ops);
        components.push(ContentComponent::GraphicsState { operators: ops });
    }
}

/// Parse PDF operations into filterable components
fn parse_into_components(
    doc: &Document,
    operations: &[Operation],
    resources: Option<&Dictionary>,
) -> Result<Vec<ContentComponent>> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] parse_into_components: Processing {} operations",
            operations.len()
        )));
        // Log all operators if there are few (to understand simple PDFs)
        let ops_to_log = if operations.len() <= 20 {
            operations.len()
        } else {
            10
        };
        for (i, op) in operations.iter().take(ops_to_log).enumerate() {
            let operands_str = op
                .operands
                .iter()
                .map(|o| match o {
                    Object::Name(n) => format!("Name({})", String::from_utf8_lossy(n)),
                    Object::Real(r) => format!("Real({})", r),
                    Object::Integer(i) => format!("Int({})", i),
                    Object::String(s, _) => format!("String({})", String::from_utf8_lossy(s)),
                    _ => format!("{:?}", o),
                })
                .collect::<Vec<_>>()
                .join(", ");
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "[DEBUG] Op {}: {} [{}]",
                i, op.operator, operands_str
            )));
        }
    }

    let mut components = Vec::new();
    let mut state = GraphicsState::default();
    let mut state_stack: Vec<GraphicsState> = Vec::new();

    // Buffers for building components
    let mut path_buffer: Vec<Operation> = Vec::new();
    let mut path_points: Vec<(f64, f64)> = Vec::new();
    let mut path_start = (0.0, 0.0);
    let mut in_text_block = false;
    let mut text_block_ops: Vec<Operation> = Vec::new();
    let mut graphics_state_ops: Vec<Operation> = Vec::new();
    let mut font_cache = FontCache::new();

    for (_op_idx, op) in operations.iter().enumerate() {
        let operator = op.operator.as_str();

        #[cfg(debug_assertions)]
        if operations.len() <= 5 {
            eprintln!(
                "[DEBUG] Operation {}: '{}' ({} bytes) with {} operands",
                _op_idx,
                operator,
                op.operator.len(),
                op.operands.len()
            );
            if operator == "Do" || operator.contains("o") {
                eprintln!("[DEBUG] Operator bytes: {:?}", op.operator.as_bytes());
                if let Some(first_operand) = op.operands.first() {
                    eprintln!("[DEBUG] First operand: {:?}", first_operand);
                }
            }
        }

        match operator {
            // Text block markers
            "BT" => {
                #[cfg(target_arch = "wasm32")]
                {
                    use wasm_bindgen::JsValue;
                    web_sys::console::log_1(&JsValue::from_str("[DEBUG] Found BT (Begin Text)"));
                }
                flush_graphics_ops(&mut components, &mut graphics_state_ops);
                in_text_block = true;
                text_block_ops.clear();
                text_block_ops.push(op.clone());
                state.reset_text_state();
            }
            "ET" => {
                text_block_ops.push(op.clone());
                #[cfg(target_arch = "wasm32")]
                {
                    use wasm_bindgen::JsValue;
                    web_sys::console::log_1(&JsValue::from_str(&format!(
                        "[DEBUG] Found ET (End Text) - creating TextBlock with {} operators",
                        text_block_ops.len()
                    )));
                }
                components.push(ContentComponent::TextBlock {
                    operators: text_block_ops.clone(),
                });
                text_block_ops.clear();
                in_text_block = false;
            }

            // If inside text block, add to text block buffer
            _ if in_text_block => {
                text_block_ops.push(op.clone());
                // Update text state
                match operator {
                    "Tf" => {
                        if let Some(Object::Name(font_name)) = op.operands.first() {
                            state.font_name = Some(font_name.clone());
                        }
                        if let Some(size) = extract_number(&op.operands, 1) {
                            state.font_size = size;
                        }
                    }
                    "Tm" => {
                        if let Some(matrix) = extract_matrix(&op.operands) {
                            state.set_text_matrix(matrix);
                        }
                    }
                    "Td" | "TD" => {
                        if let (Some(tx), Some(ty)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                        ) {
                            state.translate_text_line_matrix(tx, ty);
                            if operator == "TD" {
                                state.leading = -ty;
                            }
                        }
                    }
                    "T*" => {
                        state.move_to_next_line();
                    }
                    "Ts" => {
                        if let Some(rise) = extract_number(&op.operands, 0) {
                            state.text_rise = rise;
                        }
                    }
                    "Tw" => {
                        if let Some(space) = extract_number(&op.operands, 0) {
                            state.word_spacing = space;
                        }
                    }
                    "Tc" => {
                        if let Some(space) = extract_number(&op.operands, 0) {
                            state.char_spacing = space;
                        }
                    }
                    "Tz" => {
                        if let Some(scale) = extract_number(&op.operands, 0) {
                            state.horiz_scaling = scale / 100.0;
                        }
                    }
                    "TL" => {
                        if let Some(leading) = extract_number(&op.operands, 0) {
                            state.leading = leading;
                        }
                    }
                    _ => {}
                }
            }

            // Path construction operators
            "m" | "l" | "c" | "v" | "y" | "re" | "h" => {
                // Flush any pending graphics state ops
                if !graphics_state_ops.is_empty() {
                    components.push(ContentComponent::GraphicsState {
                        operators: graphics_state_ops.clone(),
                    });
                    graphics_state_ops.clear();
                }

                path_buffer.push(op.clone());

                // Track path points for bbox calculation
                match operator {
                    "m" => {
                        if let (Some(x), Some(y)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                        ) {
                            let pos = state.transform_point(x, y);
                            path_points.clear();
                            path_points.push(pos);
                            path_start = pos;
                        }
                    }
                    "l" => {
                        if let (Some(x), Some(y)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                        ) {
                            path_points.push(state.transform_point(x, y));
                        }
                    }
                    "c" | "v" | "y" => {
                        if op.operands.len() >= 2 {
                            if let (Some(x), Some(y)) = (
                                extract_number(&op.operands, op.operands.len() - 2),
                                extract_number(&op.operands, op.operands.len() - 1),
                            ) {
                                path_points.push(state.transform_point(x, y));
                            }
                        }
                    }
                    "re" => {
                        if let (Some(x), Some(y), Some(w), Some(h)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                            extract_number(&op.operands, 2),
                            extract_number(&op.operands, 3),
                        ) {
                            path_points.clear();
                            path_points.push(state.transform_point(x, y));
                            path_points.push(state.transform_point(x + w, y));
                            path_points.push(state.transform_point(x + w, y + h));
                            path_points.push(state.transform_point(x, y + h));
                        }
                    }
                    "h" => {
                        if !path_points.is_empty() {
                            path_points.push(path_start);
                        }
                    }
                    _ => {}
                }
            }

            // Path painting operators - commit the path component
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                path_buffer.push(op.clone());

                // Calculate bbox from path points
                let bbox = if path_points.is_empty() {
                    None
                } else {
                    calculate_path_bbox(&path_points)
                };

                components.push(ContentComponent::Path {
                    operators: path_buffer.clone(),
                    bbox,
                });

                path_buffer.clear();
                path_points.clear();
            }

            // Clipping operators - add to path buffer
            "W" | "W*" => {
                path_buffer.push(op.clone());
            }

            // End path without painting - discard
            "n" => {
                path_buffer.clear();
                path_points.clear();
            }

            // XObject operator (Do)
            "Do" => {
                // Flush any pending graphics state ops
                if !graphics_state_ops.is_empty() {
                    components.push(ContentComponent::GraphicsState {
                        operators: graphics_state_ops.clone(),
                    });
                    graphics_state_ops.clear();
                }

                if let Some(Object::Name(xobj_name)) = op.operands.first() {
                    if let Some(resources_dict) = resources {
                        // Try to determine if it's an Image or Form XObject
                        match get_xobject_type(doc, resources_dict, xobj_name) {
                            XObjectType::Image => {
                                // Calculate bbox for image placement
                                let bbox = calculate_image_bbox(&state.ctm);
                                components.push(ContentComponent::ImageXObject {
                                    operator: op.clone(),
                                    bbox,
                                });
                            }
                            XObjectType::Form => {
                                #[cfg(debug_assertions)]
                                eprintln!(
                                    "[DEBUG] Processing Form XObject: {}",
                                    String::from_utf8_lossy(xobj_name)
                                );

                                // Calculate bbox for Form XObject with proper transformation
                                let bbox = if let Ok(xobj_ref) =
                                    get_xobject_object_id(doc, resources_dict, xobj_name)
                                {
                                    #[cfg(debug_assertions)]
                                    eprintln!("[DEBUG] Got XObject reference: {:?}", xobj_ref);

                                    let result =
                                        calculate_form_xobject_bbox(doc, xobj_ref, &state.ctm);
                                    #[cfg(debug_assertions)]
                                    eprintln!(
                                        "[DEBUG] Form XObject bbox calculation result: {:?}",
                                        result
                                    );
                                    result
                                } else {
                                    #[cfg(debug_assertions)]
                                    eprintln!("[DEBUG] Failed to get XObject reference");
                                    None
                                };

                                #[cfg(target_arch = "wasm32")]
                                if let Some(ref b) = bbox {
                                    use wasm_bindgen::JsValue;
                                    web_sys::console::log_1(&JsValue::from_str(&format!(
                                        "[DEBUG] Form XObject bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                                        b.left, b.bottom, b.right, b.top
                                    )));
                                }

                                components.push(ContentComponent::FormXObject {
                                    operator: op.clone(),
                                    bbox,
                                });
                            }
                            XObjectType::Unknown => {
                                // Unknown type - keep as Form XObject to be safe
                                components.push(ContentComponent::FormXObject {
                                    operator: op.clone(),
                                    bbox: None,
                                });
                            }
                        }
                    } else {
                        // No resources - keep as Form XObject
                        components.push(ContentComponent::FormXObject {
                            operator: op.clone(),
                            bbox: None,
                        });
                    }
                } else {
                    // Invalid Do operator - keep it
                    graphics_state_ops.push(op.clone());
                }
            }

            // Graphics state operators - buffer them
            "q" => {
                state_stack.push(state.clone());
                graphics_state_ops.push(op.clone());
            }
            "Q" => {
                if let Some(saved_state) = state_stack.pop() {
                    state = saved_state;
                }
                graphics_state_ops.push(op.clone());
            }
            "cm" => {
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.apply_transform(&matrix);
                }
                graphics_state_ops.push(op.clone());
            }

            // Color, line style, and other graphics state operators
            "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k"
            | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" => {
                graphics_state_ops.push(op.clone());
            }

            // Marked content operators
            "BMC" | "BDC" | "EMC" | "MP" | "DP" => {
                graphics_state_ops.push(op.clone());
            }

            // Text showing operators that might appear outside BT/ET (invalid but happens)
            "Tj" | "TJ" | "'" | "\"" => {
                flush_graphics_ops(&mut components, &mut graphics_state_ops);
                if let Some(component) =
                    handle_orphan_text_operation(doc, resources, op, &mut state, &mut font_cache)
                {
                    components.push(component);
                } else {
                    components.push(ContentComponent::GraphicsState {
                        operators: vec![op.clone()],
                    });
                }
            }

            // Text state and font operators - track and keep
            "Tf" => {
                // Font selection - update font size for text bbox estimation
                if let Some(Object::Name(font_name)) = op.operands.first() {
                    state.font_name = Some(font_name.clone());
                }
                if let Some(size) = extract_number(&op.operands, 1) {
                    state.font_size = size;

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!("[DEBUG] Tf outside BT/ET: font size = {:.1}", size);
                }
                graphics_state_ops.push(op.clone());
            }

            "Ts" | "Tz" | "TL" | "Tw" | "Tc" | "Tr" => {
                match operator {
                    "Ts" => {
                        if let Some(rise) = extract_number(&op.operands, 0) {
                            state.text_rise = rise;
                        }
                    }
                    "Tz" => {
                        if let Some(scale) = extract_number(&op.operands, 0) {
                            state.horiz_scaling = scale / 100.0;
                        }
                    }
                    "TL" => {
                        if let Some(leading) = extract_number(&op.operands, 0) {
                            state.leading = leading;
                        }
                    }
                    "Tw" => {
                        if let Some(space) = extract_number(&op.operands, 0) {
                            state.word_spacing = space;
                        }
                    }
                    "Tc" => {
                        if let Some(space) = extract_number(&op.operands, 0) {
                            state.char_spacing = space;
                        }
                    }
                    _ => {}
                }
                graphics_state_ops.push(op.clone());
            }

            // Text positioning operators that might appear outside BT/ET
            "Tm" => {
                // Text matrix - sets absolute text position
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.set_text_matrix(matrix);

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!(
                        "[DEBUG] Tm outside BT/ET: pos = ({:.1}, {:.1})",
                        state.text_pos.0, state.text_pos.1
                    );
                }
                graphics_state_ops.push(op.clone());
            }

            "Td" | "TD" => {
                // Text position - relative move
                if let (Some(tx), Some(ty)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    state.translate_text_line_matrix(tx, ty);
                    if operator == "TD" {
                        state.leading = -ty;
                    }

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!(
                        "[DEBUG] {} outside BT/ET: pos = ({:.1}, {:.1})",
                        operator, state.text_pos.0, state.text_pos.1
                    );
                }
                graphics_state_ops.push(op.clone());
            }

            "T*" => {
                // Move to start of next line
                state.move_to_next_line();
                graphics_state_ops.push(op.clone());
            }

            // Unknown operators - add to graphics state to be safe
            _ => {
                graphics_state_ops.push(op.clone());
            }
        }
    }

    // Flush any remaining graphics state ops
    if !graphics_state_ops.is_empty() {
        components.push(ContentComponent::GraphicsState {
            operators: graphics_state_ops,
        });
    }

    // IMPORTANT: If we're still in a text block (unmatched BT), flush it
    if in_text_block && !text_block_ops.is_empty() {
        // #[cfg(debug_assertions)]
        // eprintln!("[WARNING] Unmatched BT - text block never ended with ET!");

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsValue;
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "[WARNING] Unmatched BT - creating TextBlock with {} operators",
                text_block_ops.len()
            )));
        }

        components.push(ContentComponent::TextBlock {
            operators: text_block_ops,
        });
    }

    Ok(components)
}

/// Calculate bounding box from path points
fn calculate_path_bbox(points: &[(f64, f64)]) -> Option<BoundingBox> {
    if points.is_empty() {
        return None;
    }

    let min_x = points
        .iter()
        .map(|(x, _)| x)
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let max_x = points
        .iter()
        .map(|(x, _)| x)
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let min_y = points
        .iter()
        .map(|(_, y)| y)
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let max_y = points
        .iter()
        .map(|(_, y)| y)
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));

    BoundingBox::new(min_x, min_y, max_x, max_y).ok()
}

fn handle_orphan_text_operation(
    doc: &Document,
    resources: Option<&Dictionary>,
    op: &Operation,
    state: &mut GraphicsState,
    font_cache: &mut FontCache,
) -> Option<ContentComponent> {
    let operator = op.operator.as_str();
    let font_name = match state.font_name.clone() {
        Some(name) => name,
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!(
                "[WARNING] Orphaned '{}' encountered without active font - keeping in stream",
                operator
            );
            return None;
        }
    };
    let metrics = match font_cache.get(doc, resources, &font_name) {
        Some(metrics) => metrics,
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            eprintln!(
                "[WARNING] Could not load metrics for font '{}' - keeping orphaned text",
                String::from_utf8_lossy(&font_name)
            );
            return None;
        }
    };

    let advance = match operator {
        "Tj" => measure_text_from_string(op.operands.first()?, &metrics, state)?,
        "TJ" => {
            let array = op.operands.first()?.as_array().ok()?;
            measure_text_from_array(array, &metrics, state)?
        }
        "'" => {
            state.move_to_next_line();
            measure_text_from_string(op.operands.first()?, &metrics, state)?
        }
        "\"" => {
            if let Some(space) = extract_number(&op.operands, 0) {
                state.word_spacing = space;
            }
            if let Some(space) = extract_number(&op.operands, 1) {
                state.char_spacing = space;
            }
            state.move_to_next_line();
            measure_text_from_string(op.operands.get(2)?, &metrics, state)?
        }
        _ => return None,
    };

    let bbox = calculate_text_bbox_from_state(state, advance, &metrics);
    if metrics.writing_mode == WritingMode::Vertical {
        state.translate_text_matrix(0.0, -advance);
    } else {
        state.translate_text_matrix(advance, 0.0);
    }

    Some(ContentComponent::OrphanText {
        operator: op.clone(),
        bbox,
    })
}

fn measure_text_from_string(
    operand: &Object,
    metrics: &FontMetrics,
    state: &GraphicsState,
) -> Option<f64> {
    let bytes = extract_string_bytes(operand)?;
    Some(measure_text_displacement(&bytes, metrics, state))
}

fn measure_text_from_array(
    array: &[Object],
    metrics: &FontMetrics,
    state: &GraphicsState,
) -> Option<f64> {
    let mut width = 0.0;
    for item in array {
        match item {
            Object::String(_, _) => {
                let bytes = extract_string_bytes(item)?;
                width += measure_text_displacement(&bytes, metrics, state);
            }
            Object::Integer(val) => {
                width -= (*val as f64 / 1000.0) * state.font_size * state.horiz_scaling;
            }
            Object::Real(val) => {
                width -= (*val as f64 / 1000.0) * state.font_size * state.horiz_scaling;
            }
            _ => {}
        }
    }
    Some(width)
}

fn measure_text_displacement(bytes: &[u8], metrics: &FontMetrics, state: &GraphicsState) -> f64 {
    let mut advance_total = 0.0;
    let scale = state.horiz_scaling;
    for code in decode_text_codes(bytes, metrics) {
        let mut advance = (metrics.glyph_width(code) / 1000.0) * state.font_size;
        advance += state.char_spacing;
        if !metrics.is_cid && code == 32 {
            advance += state.word_spacing;
        }
        advance_total += advance * scale;
    }
    advance_total
}

fn decode_text_codes(bytes: &[u8], metrics: &FontMetrics) -> Vec<u32> {
    if metrics.is_cid {
        let mut codes = Vec::new();
        for chunk in bytes.chunks(metrics.bytes_per_char) {
            if chunk.len() == metrics.bytes_per_char {
                let mut value = 0u32;
                for &b in chunk {
                    value = (value << 8) | b as u32;
                }
                codes.push(value);
            }
        }
        codes
    } else {
        bytes.iter().map(|b| *b as u32).collect()
    }
}

fn extract_string_bytes(obj: &Object) -> Option<Vec<u8>> {
    match obj {
        Object::String(bytes, _) => Some(bytes.clone()),
        _ => None,
    }
}

fn calculate_text_bbox_from_state(
    state: &GraphicsState,
    advance: f64,
    metrics: &FontMetrics,
) -> Option<BoundingBox> {
    if advance.abs() < f64::EPSILON {
        return None;
    }

    let ascent = (metrics.ascent / 1000.0) * state.font_size + state.text_rise;
    let descent = (metrics.descent / 1000.0) * state.font_size + state.text_rise;
    let combined = state.combined_text_matrix();

    let points = if metrics.writing_mode == WritingMode::Vertical {
        let glyph_width = (ascent - descent).abs().max(state.font_size * 0.5);
        let half_w = glyph_width / 2.0;
        [
            transform_point_with_matrix(&combined, -half_w, 0.0),
            transform_point_with_matrix(&combined, half_w, 0.0),
            transform_point_with_matrix(&combined, half_w, -advance),
            transform_point_with_matrix(&combined, -half_w, -advance),
        ]
    } else {
        [
            transform_point_with_matrix(&combined, 0.0, descent),
            transform_point_with_matrix(&combined, advance, descent),
            transform_point_with_matrix(&combined, advance, ascent.max(descent + 0.1)),
            transform_point_with_matrix(&combined, 0.0, ascent.max(descent + 0.1)),
        ]
    };

    calculate_path_bbox(&points)
}

fn transform_point_with_matrix(matrix: &[f64; 6], x: f64, y: f64) -> (f64, f64) {
    let [a, b, c, d, e, f] = matrix;
    (a * x + c * y + e, b * x + d * y + f)
}

/// Calculate bounding box for image XObject placement
/// Images are placed at (0,0)-(1,1) in user space, transformed by CTM
fn calculate_image_bbox(ctm: &[f64; 6]) -> Option<BoundingBox> {
    // Image corners in user space: (0,0), (1,0), (1,1), (0,1)
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];

    // Transform corners by CTM
    let [a, b, c, d, e, f] = ctm;
    let transformed: Vec<(f64, f64)> = corners
        .iter()
        .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f))
        .collect();

    calculate_path_bbox(&transformed)
}

/// Calculate bounding box for a Form XObject by transforming its BBox to page space
fn calculate_form_xobject_bbox(
    doc: &Document,
    xobj_ref: ObjectId,
    page_ctm: &[f64; 6],
) -> Option<BoundingBox> {
    // Get the Form XObject stream
    let xobj = doc.get_object(xobj_ref).ok()?;
    let stream = xobj.as_stream().ok()?;
    let dict = &stream.dict;

    // Get the BBox from the Form XObject (required for Form XObjects)
    let bbox_array = dict.get(b"BBox").ok()?.as_array().ok()?;
    if bbox_array.len() != 4 {
        return None;
    }

    // Parse BBox coordinates
    let x1 = bbox_array[0].as_f32().unwrap_or(0.0) as f64;
    let y1 = bbox_array[1].as_f32().unwrap_or(0.0) as f64;
    let x2 = bbox_array[2].as_f32().unwrap_or(0.0) as f64;
    let y2 = bbox_array[3].as_f32().unwrap_or(0.0) as f64;

    // Get the transformation Matrix if present (default is identity)
    let matrix = if let Ok(matrix_obj) = dict.get(b"Matrix") {
        if let Ok(matrix_array) = matrix_obj.as_array() {
            if matrix_array.len() == 6 {
                let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                for (i, val) in matrix_array.iter().enumerate() {
                    m[i] = val
                        .as_f32()
                        .unwrap_or(if i == 0 || i == 3 { 1.0 } else { 0.0 })
                        as f64;
                }
                m
            } else {
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
            }
        } else {
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        }
    } else {
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    };

    // Combine the Form XObject's matrix with the page CTM
    let combined_ctm = multiply_matrices(page_ctm, &matrix);

    // Transform the four corners of the BBox
    let corners = [(x1, y1), (x2, y1), (x2, y2), (x1, y2)];

    let [a, b, c, d, e, f] = combined_ctm;
    let transformed: Vec<(f64, f64)> = corners
        .iter()
        .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f))
        .collect();

    calculate_path_bbox(&transformed)
}

/// Multiply two transformation matrices
fn multiply_matrices(m1: &[f64; 6], m2: &[f64; 6]) -> [f64; 6] {
    let [a1, b1, c1, d1, e1, f1] = m1;
    let [a2, b2, c2, d2, e2, f2] = m2;

    [
        a1 * a2 + b1 * c2,
        a1 * b2 + b1 * d2,
        c1 * a2 + d1 * c2,
        c1 * b2 + d1 * d2,
        e1 * a2 + f1 * c2 + e2,
        e1 * b2 + f1 * d2 + f2,
    ]
}

#[derive(Debug, Clone, Copy)]
enum XObjectType {
    Image,
    Form,
    Unknown,
}

/// Determine the type of XObject (Image or Form)
fn get_xobject_type(doc: &Document, resources: &Dictionary, xobj_name: &[u8]) -> XObjectType {
    let xobject_ref = resources.get(b"XObject");
    if xobject_ref.is_err() {
        return XObjectType::Unknown;
    }

    let xobject_dict = xobject_ref.unwrap().as_dict();
    if xobject_dict.is_err() {
        return XObjectType::Unknown;
    }

    let xobj_ref = xobject_dict
        .unwrap()
        .get(xobj_name)
        .ok()
        .and_then(|obj| obj.as_reference().ok());
    if xobj_ref.is_none() {
        return XObjectType::Unknown;
    }

    let xobj_stream = doc.get_object(xobj_ref.unwrap());
    if xobj_stream.is_err() {
        return XObjectType::Unknown;
    }

    let xobj_stream = xobj_stream.unwrap().as_stream();
    if xobj_stream.is_err() {
        return XObjectType::Unknown;
    }

    // Check Subtype
    let subtype = xobj_stream
        .unwrap()
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|obj| obj.as_name().ok());

    match subtype {
        Some(b"Image") => XObjectType::Image,
        Some(b"Form") => XObjectType::Form,
        _ => XObjectType::Unknown,
    }
}

/// Filter components based on bbox overlap with crop box
fn filter_components(components: Vec<ContentComponent>, crop_box: &BoundingBox) -> Vec<Operation> {
    const SAFETY_MARGIN: f64 = 15.0; // Points to add around crop box for safety

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Filtering {} components with crop box: ({:.2}, {:.2}, {:.2}, {:.2})",
            components.len(),
            crop_box.left,
            crop_box.bottom,
            crop_box.right,
            crop_box.top
        )));
    }

    #[cfg(debug_assertions)]
    eprintln!(
        "[DEBUG] filter_components: {} components, crop: ({:.1}, {:.1}, {:.1}, {:.1})",
        components.len(),
        crop_box.left,
        crop_box.bottom,
        crop_box.right,
        crop_box.top
    );

    let mut output = Vec::new();
    let mut stats = ComponentStats::default();

    for component in components {
        match component {
            // Always keep graphics state operators
            ContentComponent::GraphicsState { operators } => {
                stats.graphics_state += 1;
                output.extend(operators);
            }

            // Always keep text blocks (too risky to filter)
            ContentComponent::TextBlock { operators } => {
                stats.text_blocks += 1;
                #[cfg(target_arch = "wasm32")]
                {
                    use wasm_bindgen::JsValue;
                    web_sys::console::log_1(&JsValue::from_str(&format!(
                        "[DEBUG] Keeping TextBlock with {} operators",
                        operators.len()
                    )));
                }
                output.extend(operators);
            }

            // Filter Form XObjects based on bbox (now with proper transformation)
            ContentComponent::FormXObject { operator, bbox } => {
                stats.form_xobjects += 1;
                if let Some(form_bbox) = bbox {
                    if has_overlap(&form_bbox, crop_box, SAFETY_MARGIN) {
                        #[cfg(target_arch = "wasm32")]
                        {
                            use wasm_bindgen::JsValue;
                            web_sys::console::log_1(&JsValue::from_str(&format!(
                                "[DEBUG] Keeping Form XObject with bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                                form_bbox.left, form_bbox.bottom, form_bbox.right, form_bbox.top
                            )));
                        }
                        output.push(operator);
                    } else {
                        #[cfg(target_arch = "wasm32")]
                        {
                            use wasm_bindgen::JsValue;
                            web_sys::console::log_1(&JsValue::from_str(&format!(
                                "[DEBUG] Removing Form XObject outside bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                                form_bbox.left, form_bbox.bottom, form_bbox.right, form_bbox.top
                            )));
                        }
                    }
                } else {
                    // No bbox calculated - keep to be safe
                    #[cfg(target_arch = "wasm32")]
                    {
                        use wasm_bindgen::JsValue;
                        web_sys::console::log_1(&JsValue::from_str(
                            "[DEBUG] Keeping Form XObject (no bbox calculated)",
                        ));
                    }
                    output.push(operator);
                }
            }

            // Filter paths based on bbox overlap
            ContentComponent::Path { operators, bbox } => {
                stats.paths_total += 1;
                if let Some(path_bbox) = bbox {
                    if has_overlap(&path_bbox, crop_box, SAFETY_MARGIN) {
                        stats.paths_kept += 1;
                        output.extend(operators);
                    } else {
                        #[cfg(target_arch = "wasm32")]
                        {
                            use wasm_bindgen::JsValue;
                            web_sys::console::log_1(&JsValue::from_str(&format!(
                                "[DEBUG] Removing path outside bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                                path_bbox.left, path_bbox.bottom, path_bbox.right, path_bbox.top
                            )));
                        }
                    }
                } else {
                    // No bbox calculated - keep to be safe
                    stats.paths_kept += 1;
                    output.extend(operators);
                }
            }

            // Filter images based on bbox overlap
            ContentComponent::ImageXObject { operator, bbox } => {
                stats.images_total += 1;
                if let Some(image_bbox) = bbox {
                    if has_overlap(&image_bbox, crop_box, SAFETY_MARGIN) {
                        stats.images_kept += 1;
                        output.push(operator);
                    } else {
                        #[cfg(target_arch = "wasm32")]
                        {
                            use wasm_bindgen::JsValue;
                            web_sys::console::log_1(&JsValue::from_str(&format!(
                                "[DEBUG] Removing image outside bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                                image_bbox.left,
                                image_bbox.bottom,
                                image_bbox.right,
                                image_bbox.top
                            )));
                        }
                    }
                } else {
                    // No bbox calculated - keep to be safe
                    stats.images_kept += 1;
                    output.push(operator);
                }
            }
            ContentComponent::OrphanText { operator, bbox } => {
                stats.orphan_text_total += 1;
                if let Some(text_bbox) = bbox {
                    if has_overlap(&text_bbox, crop_box, SAFETY_MARGIN) {
                        stats.orphan_text_kept += 1;
                        output.push(operator);
                    } else {
                        #[cfg(target_arch = "wasm32")]
                        {
                            use wasm_bindgen::JsValue;
                            web_sys::console::log_1(&JsValue::from_str(&format!(
                                "[DEBUG] Removing orphan text outside bbox: ({:.2}, {:.2}, {:.2}, {:.2})",
                                text_bbox.left,
                                text_bbox.bottom,
                                text_bbox.right,
                                text_bbox.top
                            )));
                        }
                    }
                } else {
                    // Missing bbox (e.g., font metrics unavailable) - keep to be safe
                    stats.orphan_text_kept += 1;
                    output.push(operator);
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Component stats: {} text blocks, {} graphics state, {} form XObjects, {}/{} paths kept, {}/{} images kept, {}/{} orphan text kept",
            stats.text_blocks, stats.graphics_state, stats.form_xobjects,
            stats.paths_kept, stats.paths_total,
            stats.images_kept, stats.images_total,
            stats.orphan_text_kept, stats.orphan_text_total
        )));
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Final output: {} operators",
            output.len()
        )));
    }

    output
}

#[derive(Default)]
struct ComponentStats {
    text_blocks: usize,
    graphics_state: usize,
    form_xobjects: usize,
    paths_total: usize,
    paths_kept: usize,
    images_total: usize,
    images_kept: usize,
    orphan_text_total: usize,
    orphan_text_kept: usize,
}

/// Check if two bounding boxes have any overlap (with safety margin)
fn has_overlap(component_bbox: &BoundingBox, crop_box: &BoundingBox, margin: f64) -> bool {
    // Expand crop_box by margin for safety
    // Use a VERY large margin to avoid removing content that might be partially visible
    // PDFs can have complex clipping and transformation that makes content appear
    // in different locations than their bbox suggests
    let actual_margin = margin.max(200.0); // Large margin for complex PDFs
    let left = crop_box.left - actual_margin;
    let bottom = crop_box.bottom - actual_margin;
    let right = crop_box.right + actual_margin;
    let top = crop_box.top + actual_margin;

    // Check if bboxes overlap (not just touch)
    !(component_bbox.right < left
        || component_bbox.left > right
        || component_bbox.top < bottom
        || component_bbox.bottom > top)
}

/// Filter content stream to remove operations outside the crop box
///
/// This analyzes the content stream and removes drawing operations (text, paths, images)
/// that fall completely outside the specified crop box. Operations that are at least
/// partially inside the crop box are preserved.
///
/// # Arguments
/// * `doc` - The PDF document (for looking up Form XObjects)
/// * `stream` - The page content stream to filter
/// * `resources` - The page's Resources dictionary (for XObject lookup)
/// * `crop_box` - The bounding box to use for filtering
///
/// # Returns
/// Tuple of (filtered_content_bytes, form_xobjects_to_filter)
/// where form_xobjects_to_filter is a list of (ObjectId, Resources) for recursive filtering
pub fn filter_content_stream(
    doc: &Document,
    stream: &Stream,
    resources: Option<&Dictionary>,
    crop_box: &BoundingBox,
) -> Result<(Vec<u8>, Vec<(ObjectId, Option<Dictionary>)>)> {
    // Decode the content stream into operations
    let decoded_bytes = stream
        .decompressed_content()
        .map_err(|e| Error::PdfParse(format!("Failed to decompress content stream: {}", e)))?;

    #[cfg(debug_assertions)]
    {
        // Show first few bytes of decoded content
        let preview = if decoded_bytes.len() > 50 {
            &decoded_bytes[..50]
        } else {
            &decoded_bytes
        };
        eprintln!("[DEBUG] Raw content bytes (first 50): {:?}", preview);
        // Try to interpret as ASCII
        let ascii_preview = String::from_utf8_lossy(preview);
        eprintln!("[DEBUG] As ASCII: {}", ascii_preview);
    }

    let content = match Content::decode(&decoded_bytes) {
        Ok(c) => {
            // Check if the parsed content looks suspicious (single invalid operator)
            if c.operations.len() == 1 {
                let op = &c.operations[0];
                // "x" and "H" are not valid PDF operators
                if op.operator == "x" || op.operator == "H" {
                    #[cfg(target_arch = "wasm32")]
                    {
                        use wasm_bindgen::JsValue;
                        web_sys::console::log_1(&JsValue::from_str(&format!(
                            "[WARNING] Invalid operator '{}' detected - keeping original content",
                            op.operator
                        )));
                    }

                    // Return original content unchanged
                    return Ok((decoded_bytes, vec![]));
                }
            }
            c
        }
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("[DEBUG] Content::decode failed: {:?}", e);

            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[WARNING] Content::decode failed - keeping original content: {:?}",
                    e
                )));
            }

            // If parsing fails, return original content unchanged
            return Ok((decoded_bytes, vec![]));
        }
    };

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Content stream has {} operations",
            content.operations.len()
        )));

        // Log raw bytes info for debugging
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Decoded {} bytes from content stream",
            decoded_bytes.len()
        )));

        if content.operations.len() == 1 {
            let op = &content.operations[0];
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "[DEBUG] Single operation: '{}' with {} operands",
                op.operator,
                op.operands.len()
            )));

            // Show raw bytes for debugging single-op streams
            if decoded_bytes.len() < 100 {
                let preview = String::from_utf8_lossy(&decoded_bytes);
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Raw content: {}",
                    preview
                )));
            }
        }
    }

    // Non-WASM debug logging
    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!(
            "[DEBUG] Content stream has {} operations",
            content.operations.len()
        );
        if content.operations.len() == 1 {
            let op = &content.operations[0];
            eprintln!(
                "[DEBUG] Single op: '{}', operands: {:?}",
                op.operator, op.operands
            );
            if let Some(Object::Name(name)) = op.operands.first() {
                eprintln!(
                    "[DEBUG] First operand is Name: {}",
                    String::from_utf8_lossy(name)
                );
            }
        } else if !content.operations.is_empty() {
            // Check if stream starts with text operators (potential issue)
            let first_op = &content.operations[0];
            if matches!(first_op.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
                eprintln!(
                    "[WARNING] Stream starts with text operator '{}' without BT!",
                    first_op.operator
                );
                eprintln!(
                    "[WARNING] This is invalid PDF - text operators should be inside BT/ET blocks"
                );
            }

            // Count BT/ET pairs
            let bt_count = content
                .operations
                .iter()
                .filter(|op| op.operator == "BT")
                .count();
            let et_count = content
                .operations
                .iter()
                .filter(|op| op.operator == "ET")
                .count();
            let text_ops_count = content
                .operations
                .iter()
                .filter(|op| matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\""))
                .count();

            eprintln!(
                "[DEBUG] BT: {}, ET: {}, Text ops: {}",
                bt_count, et_count, text_ops_count
            );

            if content.operations.len() <= 10 {
                // Show first few operators for small streams
                for (i, op) in content.operations.iter().take(5).enumerate() {
                    eprintln!("[DEBUG] Op[{}]: '{}'", i, op.operator);
                }
            }
        }
    }

    // NEW: Component-based filtering
    // Parse operations into filterable components
    let components = parse_into_components(doc, &content.operations, resources)?;

    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!("[DEBUG] Parsed into {} components", components.len());
        for (i, comp) in components.iter().enumerate() {
            match comp {
                ContentComponent::Path { bbox, .. } => {
                    if let Some(b) = bbox {
                        eprintln!(
                            "[DEBUG] Component {} (Path): bbox=({:.1},{:.1},{:.1},{:.1})",
                            i, b.left, b.bottom, b.right, b.top
                        );
                    }
                }
                ContentComponent::FormXObject { bbox, .. } => {
                    if let Some(b) = bbox {
                        eprintln!(
                            "[DEBUG] Component {} (FormXObject): bbox=({:.1},{:.1},{:.1},{:.1})",
                            i, b.left, b.bottom, b.right, b.top
                        );
                    } else {
                        eprintln!("[DEBUG] Component {} (FormXObject): no bbox", i);
                    }
                }
                ContentComponent::TextBlock { operators } => {
                    eprintln!(
                        "[DEBUG] Component {} (TextBlock): {} ops",
                        i,
                        operators.len()
                    );
                }
                ContentComponent::GraphicsState { operators } => {
                    eprintln!(
                        "[DEBUG] Component {} (GraphicsState): {} ops",
                        i,
                        operators.len()
                    );
                }
                _ => {}
            }
        }
    }

    // Filter components based on bbox overlap
    let filtered_ops = filter_components(components, crop_box);

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        let original_count = content.operations.len();
        let filtered_count = filtered_ops.len();
        let removed_count = original_count.saturating_sub(filtered_count);
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Filtered to {} operations ({} removed)",
            filtered_count, removed_count
        )));

        // IMPORTANT: If nothing was filtered, return original bytes to avoid re-encoding issues
        if removed_count == 0 {
            web_sys::console::log_1(&JsValue::from_str(
                "[DEBUG] No operations removed - keeping original content stream",
            ));
            return Ok((decoded_bytes, vec![]));
        }
    }

    // Non-WASM debug logging
    #[cfg(not(target_arch = "wasm32"))]
    {
        let original_count = content.operations.len();
        let filtered_count = filtered_ops.len();
        let removed_count = original_count.saturating_sub(filtered_count);
        eprintln!(
            "[DEBUG] Filtered to {} operations ({} removed)",
            filtered_count, removed_count
        );

        // IMPORTANT: If nothing was filtered, return original bytes to avoid re-encoding issues
        if removed_count == 0 {
            eprintln!("[DEBUG] No operations removed - keeping original content stream");
            return Ok((decoded_bytes, vec![]));
        }
    }

    // Only re-encode if we actually filtered something
    // Encode back to bytes
    let filtered_content = Content {
        operations: filtered_ops,
    };

    let encoded = filtered_content
        .encode()
        .map_err(|e| Error::PdfParse(format!("Failed to encode content stream: {}", e)))?;

    // Return empty form_xobjects list (Form XObjects not filtered)
    Ok((encoded, vec![]))
}

/// Get Form XObject ObjectId
fn get_xobject_object_id(
    _doc: &Document,
    resources: &Dictionary,
    xobj_name: &[u8],
) -> Result<ObjectId> {
    // Look up XObject in Resources
    let xobject_ref = resources.get(b"XObject")?;
    let xobject_dict = xobject_ref
        .as_dict()
        .map_err(|_| Error::PdfParse("XObject is not a dictionary".to_string()))?;

    let xobj_ref = xobject_dict
        .get(xobj_name)
        .ok()
        .and_then(|obj| obj.as_reference().ok())
        .ok_or_else(|| {
            Error::PdfParse(format!(
                "XObject {} not found in Resources",
                String::from_utf8_lossy(xobj_name)
            ))
        })?;

    Ok(xobj_ref)
}

/// Get Form XObject reference and resources for later filtering
/// Returns (ObjectId, Option<Dictionary>) if it's a Form XObject
#[allow(dead_code)]
fn get_form_xobject_ref(
    doc: &Document,
    resources: &Dictionary,
    xobj_name: &[u8],
) -> Result<(ObjectId, Option<Dictionary>)> {
    // Look up XObject in Resources
    let xobject_ref = resources.get(b"XObject")?;
    let xobject_dict = xobject_ref
        .as_dict()
        .map_err(|_| Error::PdfParse("XObject is not a dictionary".to_string()))?;

    let xobj_ref = xobject_dict
        .get(xobj_name)
        .ok()
        .and_then(|obj| obj.as_reference().ok())
        .ok_or_else(|| {
            Error::PdfParse(format!(
                "XObject {} not found in Resources",
                String::from_utf8_lossy(xobj_name)
            ))
        })?;

    // Get the XObject stream to check if it's a Form
    let xobj_stream = doc
        .get_object(xobj_ref)
        .map_err(|e| Error::PdfParse(format!("Failed to get XObject: {}", e)))?
        .as_stream()
        .map_err(|e| Error::PdfParse(format!("XObject is not a stream: {}", e)))?;

    // Check if it's a Form XObject (Subtype = Form)
    let is_form = xobj_stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|obj| obj.as_name().ok())
        .map(|name| name == b"Form")
        .unwrap_or(false);

    if !is_form {
        // Not a Form XObject (probably an Image), skip
        return Err(Error::PdfParse("Not a Form XObject".to_string()));
    }

    // Get Form XObject's Resources (it may have its own)
    let form_resources = xobj_stream
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|obj| obj.as_dict().ok()).cloned();

    Ok((xobj_ref, form_resources))
}

/// Filter a Form XObject's content stream
/// This is called in the second pass after collecting all Form XObjects
pub fn filter_form_xobject(
    doc: &mut Document,
    xobj_id: ObjectId,
    xobj_resources: Option<Dictionary>,
    crop_box: &BoundingBox,
) -> Result<Vec<(ObjectId, Option<Dictionary>)>> {
    // Get the XObject stream (immutably first to avoid borrow conflicts)
    let xobj_stream = doc
        .get_object(xobj_id)
        .map_err(|e| Error::PdfParse(format!("Failed to get XObject: {}", e)))?
        .as_stream()
        .map_err(|e| Error::PdfParse(format!("XObject is not a stream: {}", e)))?;

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Filtering Form XObject: {:?}",
            xobj_id
        )));
    }

    // Filter the Form XObject's content stream (returns nested Form XObjects)
    let (filtered_content, nested_form_xobjects) =
        filter_content_stream(doc, xobj_stream, xobj_resources.as_ref(), crop_box)?;

    // Update the Form XObject's content
    let xobj_stream_mut = doc
        .get_object_mut(xobj_id)
        .map_err(|e| Error::PdfParse(format!("Failed to get XObject mut: {}", e)))?
        .as_stream_mut()
        .map_err(|e| Error::PdfParse(format!("XObject is not a stream (mut): {}", e)))?;

    xobj_stream_mut.set_plain_content(filtered_content);

    // Return nested Form XObjects for recursive filtering
    Ok(nested_form_xobjects)
}

/// Filter operations based on crop box intersection
/// Collects Form XObjects for later filtering (two-pass approach)
#[allow(dead_code)]
fn filter_operations(
    doc: &Document,
    operations: &[Operation],
    resources: Option<&Dictionary>,
    crop_box: &BoundingBox,
) -> Result<(Vec<Operation>, Vec<(ObjectId, Option<Dictionary>)>)> {
    let mut filtered = vec![];
    let mut form_xobjects: Vec<(ObjectId, Option<Dictionary>)> = vec![];
    let mut state = GraphicsState::default();
    let mut state_stack: Vec<GraphicsState> = vec![];
    let mut current_path: Vec<(f64, f64)> = vec![];
    let mut path_start = (0.0, 0.0);
    let mut path_ops_buffer: Vec<Operation> = vec![]; // Buffer for path construction operators

    // Note: We don't add a clipping path here - we just filter operations
    // The CropBox will handle visual cropping

    for op in operations {
        let operator = op.operator.as_str();
        let should_keep = match operator {
            // Graphics state operators - always keep
            "q" => {
                state_stack.push(state.clone());
                true
            }
            "Q" => {
                if let Some(saved_state) = state_stack.pop() {
                    state = saved_state;
                }
                true
            }
            "cm" => {
                // Transformation matrix
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.apply_transform(&matrix);
                }
                true
            }

            // Text state operators - always keep (needed for subsequent text)
            "Tf" => {
                // Font and size
                if let Some(size) = extract_number(&op.operands, 1) {
                    state.font_size = size;
                }
                true
            }
            "Tc" | "Tw" | "Tz" | "TL" | "Tr" | "Ts" => true, // Text rendering params

            // Text positioning operators
            "Td" | "TD" => {
                if let (Some(tx), Some(ty)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    state.text_pos = (state.text_pos.0 + tx, state.text_pos.1 + ty);
                }
                true
            }
            "Tm" => {
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.text_matrix = matrix;
                    state.text_pos = (matrix[4], matrix[5]);
                }
                true
            }
            "T*" => true,

            // Text block operators
            "BT" => {
                state.text_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                state.text_pos = (0.0, 0.0);
                true
            }
            "ET" => true,

            // Text showing operators - always keep (too risky to filter)
            // Text extent is hard to calculate (depends on font metrics, rotation, etc.)
            // Better to keep all text than accidentally clip user's content
            "Tj" | "TJ" | "'" | "\"" => true,

            // Path construction operators - buffer them for now
            "m" => {
                // Move to
                if let (Some(x), Some(y)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    let pos = state.transform_point(x, y);
                    current_path.clear();
                    current_path.push(pos);
                    path_start = pos;
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "l" => {
                // Line to
                if let (Some(x), Some(y)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    current_path.push(state.transform_point(x, y));
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "c" | "v" | "y" => {
                // Bezier curves - just track end point
                if op.operands.len() >= 2 {
                    if let (Some(x), Some(y)) = (
                        extract_number(&op.operands, op.operands.len() - 2),
                        extract_number(&op.operands, op.operands.len() - 1),
                    ) {
                        current_path.push(state.transform_point(x, y));
                    }
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "re" => {
                // Rectangle
                if let (Some(x), Some(y), Some(w), Some(h)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                    extract_number(&op.operands, 2),
                    extract_number(&op.operands, 3),
                ) {
                    current_path.clear();
                    current_path.push(state.transform_point(x, y));
                    current_path.push(state.transform_point(x + w, y));
                    current_path.push(state.transform_point(x + w, y + h));
                    current_path.push(state.transform_point(x, y + h));
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "h" => {
                // Close path
                if !current_path.is_empty() {
                    current_path.push(path_start);
                }
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }

            // Path painting operators - check if path intersects crop box
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                let keep = path_intersects_box(&current_path, crop_box);
                if keep {
                    // Commit buffered path construction operators
                    filtered.append(&mut path_ops_buffer);
                    filtered.push(op.clone());
                } else {
                    // Discard buffered path construction operators
                    path_ops_buffer.clear();
                }
                current_path.clear();
                false // Already added above if needed
            }

            // Clipping operators - buffer them (they're part of the path)
            "W" | "W*" => {
                path_ops_buffer.push(op.clone());
                false // Don't add to filtered yet
            }
            "n" => {
                // End path without painting - discard buffered path ops
                current_path.clear();
                path_ops_buffer.clear();
                false // No need to keep 'n' if path was discarded
            }

            // Color operators - always keep
            "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" => true,

            // XObject operator - collect Form XObjects for later filtering
            "Do" => {
                // Extract XObject name and collect it for second pass
                if let Some(Object::Name(xobj_name)) = op.operands.first() {
                    if let Some(resources_dict) = resources {
                        // Try to get the XObject reference
                        if let Ok((xobj_id, xobj_resources)) =
                            get_form_xobject_ref(doc, resources_dict, xobj_name)
                        {
                            form_xobjects.push((xobj_id, xobj_resources));
                        }
                    }
                }
                // Always keep Do operators
                true
            }

            // Line width and other graphics state - always keep
            "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" => true,

            // Marked content operators - always keep
            "BMC" | "BDC" | "EMC" | "MP" | "DP" => true,

            // Unknown operators - keep to be safe
            _ => true,
        };

        if should_keep {
            filtered.push(op.clone());
        } else {
            #[cfg(target_arch = "wasm32")]
            {
                use wasm_bindgen::JsValue;
                web_sys::console::log_1(&JsValue::from_str(&format!(
                    "[DEBUG] Filtered out: {}",
                    operator
                )));
            }

            // Non-WASM debug logging
            #[cfg(not(target_arch = "wasm32"))]
            {
                eprintln!(
                    "[DEBUG] Filtered out: {} (operands: {})",
                    operator,
                    op.operands.len()
                );
            }
        }
    }

    Ok((filtered, form_xobjects))
}

/// Create PDF operations for a rectangular clipping path
#[allow(dead_code)]
fn create_clipping_path_operations(bbox: &BoundingBox) -> Vec<lopdf::content::Operation> {
    use lopdf::content::Operation;

    vec![
        // q - Save graphics state
        Operation::new("q", vec![]),
        // x y width height re - Rectangle
        Operation::new(
            "re",
            vec![
                Object::Real(bbox.left as f32),
                Object::Real(bbox.bottom as f32),
                Object::Real(bbox.width() as f32),
                Object::Real(bbox.height() as f32),
            ],
        ),
        // W - Clip
        Operation::new("W", vec![]),
        // n - End path without painting
        Operation::new("n", vec![]),
    ]
}

/// Extract a transformation matrix from PDF operands
fn extract_matrix(operands: &[Object]) -> Option<[f64; 6]> {
    if operands.len() >= 6 {
        Some([
            extract_number(operands, 0)?,
            extract_number(operands, 1)?,
            extract_number(operands, 2)?,
            extract_number(operands, 3)?,
            extract_number(operands, 4)?,
            extract_number(operands, 5)?,
        ])
    } else {
        None
    }
}

/// Extract a numeric value from PDF operands at the given index
fn extract_number(operands: &[Object], index: usize) -> Option<f64> {
    operands.get(index).and_then(|obj| match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(f) => Some(*f as f64),
        _ => None,
    })
}

/// Check if a point is within or near the bounding box
#[allow(dead_code)]
fn is_point_near_box(point: (f64, f64), bbox: &BoundingBox, margin: f64) -> bool {
    let (x, y) = point;
    x >= bbox.left - margin
        && x <= bbox.right + margin
        && y >= bbox.bottom - margin
        && y <= bbox.top + margin
}

/// Check if a path intersects with the bounding box
#[allow(dead_code)]
fn path_intersects_box(path: &[(f64, f64)], bbox: &BoundingBox) -> bool {
    if path.is_empty() {
        return true; // Keep if we can't determine
    }

    // Check if any point of the path is inside or near the box
    for &(x, y) in path {
        if is_point_near_box((x, y), bbox, 10.0) {
            return true;
        }
    }

    // Compute bounding box of the path
    let min_x = path
        .iter()
        .map(|(x, _)| x)
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let max_x = path
        .iter()
        .map(|(x, _)| x)
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let min_y = path
        .iter()
        .map(|(_, y)| y)
        .fold(f64::INFINITY, |a, &b| a.min(b));
    let max_y = path
        .iter()
        .map(|(_, y)| y)
        .fold(f64::NEG_INFINITY, |a, &b| a.max(b));

    // Check if path bounding box intersects with crop box
    !(max_x < bbox.left || min_x > bbox.right || max_y < bbox.bottom || min_y > bbox.top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_clipping_path() {
        let bbox = BoundingBox::new(100.0, 100.0, 500.0, 700.0).unwrap();
        let ops = create_clipping_path_operations(&bbox);
        assert_eq!(ops.len(), 4);
        assert_eq!(ops[0].operator, "q");
        assert_eq!(ops[1].operator, "re");
        assert_eq!(ops[2].operator, "W");
        assert_eq!(ops[3].operator, "n");
    }

    #[test]
    fn test_is_point_near_box() {
        let bbox = BoundingBox::new(100.0, 100.0, 500.0, 700.0).unwrap();

        // Inside
        assert!(is_point_near_box((300.0, 400.0), &bbox, 0.0));

        // Outside
        assert!(!is_point_near_box((50.0, 50.0), &bbox, 0.0));

        // Near with margin
        assert!(is_point_near_box((95.0, 100.0), &bbox, 10.0));
    }

    #[test]
    fn test_extract_number() {
        let operands = vec![Object::Integer(42), Object::Real(3.14)];
        assert_eq!(extract_number(&operands, 0), Some(42.0));
        let real_value = extract_number(&operands, 1).unwrap();
        assert!((real_value - 3.14).abs() < 1e-6);
        assert_eq!(extract_number(&operands, 2), None);
    }
}
