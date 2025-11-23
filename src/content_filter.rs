//! Content stream filtering to remove elements outside crop box
//!
//! This module provides functionality to analyze PDF content streams and remove
//! drawing operations that fall completely outside the crop box, improving
//! privacy/security by ensuring clipped content is actually removed from the file.

use crate::bbox::BoundingBox;
use crate::error::{Error, Result};
use lopdf::{content::{Content, Operation}, Dictionary, Document, Object, ObjectId, Stream};

/// Graphics state for tracking transformations and positions
#[derive(Debug, Clone)]
struct GraphicsState {
    /// Current transformation matrix [a b c d e f]
    ctm: [f64; 6],
    /// Current text matrix
    text_matrix: [f64; 6],
    /// Current text position
    text_pos: (f64, f64),
    /// Current font size (from Tf operator)
    font_size: f64,
}

impl Default for GraphicsState {
    fn default() -> Self {
        GraphicsState {
            ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], // Identity matrix
            text_matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            text_pos: (0.0, 0.0),
            font_size: 12.0,
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
    TextBlock {
        operators: Vec<Operation>,
    },
    /// Graphics state operators (q, Q, cm, colors, line styles) - always kept
    GraphicsState {
        operators: Vec<Operation>,
    },
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
        let ops_to_log = if operations.len() <= 20 { operations.len() } else { 10 };
        for (i, op) in operations.iter().take(ops_to_log).enumerate() {
            let operands_str = op.operands.iter()
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

    for (op_idx, op) in operations.iter().enumerate() {
        let operator = op.operator.as_str();

        #[cfg(debug_assertions)]
        if operations.len() <= 5 {
            eprintln!("[DEBUG] Operation {}: '{}' ({} bytes) with {} operands",
                op_idx, operator, op.operator.len(), op.operands.len());
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
                // Flush any pending graphics state ops
                if !graphics_state_ops.is_empty() {
                    components.push(ContentComponent::GraphicsState {
                        operators: graphics_state_ops.clone(),
                    });
                    graphics_state_ops.clear();
                }
                in_text_block = true;
                text_block_ops.clear();
                text_block_ops.push(op.clone());
                state.text_matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                state.text_pos = (0.0, 0.0);
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
                        if let Some(size) = extract_number(&op.operands, 1) {
                            state.font_size = size;
                        }
                    }
                    "Tm" => {
                        if let Some(matrix) = extract_matrix(&op.operands) {
                            state.text_matrix = matrix;
                            state.text_pos = (matrix[4], matrix[5]);
                        }
                    }
                    "Td" | "TD" => {
                        if let (Some(tx), Some(ty)) = (
                            extract_number(&op.operands, 0),
                            extract_number(&op.operands, 1),
                        ) {
                            state.text_pos = (state.text_pos.0 + tx, state.text_pos.1 + ty);
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
                                eprintln!("[DEBUG] Processing Form XObject: {}", String::from_utf8_lossy(xobj_name));

                                // Calculate bbox for Form XObject with proper transformation
                                let bbox = if let Ok(xobj_ref) = get_xobject_object_id(doc, resources_dict, xobj_name) {
                                    #[cfg(debug_assertions)]
                                    eprintln!("[DEBUG] Got XObject reference: {:?}", xobj_ref);

                                    let result = calculate_form_xobject_bbox(doc, xobj_ref, &state.ctm);
                                    #[cfg(debug_assertions)]
                                    eprintln!("[DEBUG] Form XObject bbox calculation result: {:?}", result);
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
            "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" |
            "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" => {
                graphics_state_ops.push(op.clone());
            }

            // Marked content operators
            "BMC" | "BDC" | "EMC" | "MP" | "DP" => {
                graphics_state_ops.push(op.clone());
            }

            // Text showing operators that might appear outside BT/ET (invalid but happens)
            "Tj" | "TJ" | "'" | "\"" => {
                // These are text operators that should be in BT/ET but sometimes aren't
                // For now, we MUST keep these as they contain actual content
                // TODO: Proper solution would calculate actual text bbox from font metrics

                #[cfg(not(target_arch = "wasm32"))]
                {
                    eprintln!("[WARNING] Orphaned '{}' at ({:.1}, {:.1}) - keeping in stream",
                        operator, state.text_pos.0, state.text_pos.1);
                }

                // Add to graphics state buffer to preserve the text
                // We cannot filter these without proper font metrics
                graphics_state_ops.push(op.clone());

                // Update text position for ' and " operators which include line feed
                if operator == "'" {
                    // ' operator = T* + Tj (move to next line and show text)
                    state.text_pos.1 -= state.font_size * 1.2; // Approximate line height
                } else if operator == "\"" {
                    // " operator = T* + Tw + Tc + Tj
                    state.text_pos.1 -= state.font_size * 1.2;
                }
            }

            // Text state and font operators - track and keep
            "Tf" => {
                // Font selection - update font size for text bbox estimation
                if let Some(size) = extract_number(&op.operands, 1) {
                    state.font_size = size;

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!("[DEBUG] Tf outside BT/ET: font size = {:.1}", size);
                }
                graphics_state_ops.push(op.clone());
            }

            "Ts" | "Tz" | "TL" | "Tw" | "Tc" | "Tr" => {
                graphics_state_ops.push(op.clone());
            }

            // Text positioning operators that might appear outside BT/ET
            "Tm" => {
                // Text matrix - sets absolute text position
                if let Some(matrix) = extract_matrix(&op.operands) {
                    state.text_matrix = matrix;
                    state.text_pos = (matrix[4], matrix[5]);

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!("[DEBUG] Tm outside BT/ET: pos = ({:.1}, {:.1})",
                        state.text_pos.0, state.text_pos.1);
                }
                graphics_state_ops.push(op.clone());
            }

            "Td" | "TD" => {
                // Text position - relative move
                if let (Some(tx), Some(ty)) = (
                    extract_number(&op.operands, 0),
                    extract_number(&op.operands, 1),
                ) {
                    state.text_pos = (state.text_pos.0 + tx, state.text_pos.1 + ty);

                    #[cfg(not(target_arch = "wasm32"))]
                    eprintln!("[DEBUG] {} outside BT/ET: pos = ({:.1}, {:.1})",
                        operator, state.text_pos.0, state.text_pos.1);
                }
                graphics_state_ops.push(op.clone());
            }

            "T*" => {
                // Move to start of next line
                state.text_pos.1 -= state.font_size * 1.2; // Approximate line height
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

    let min_x = points.iter().map(|(x, _)| x).fold(f64::INFINITY, |a, &b| a.min(b));
    let max_x = points.iter().map(|(x, _)| x).fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let min_y = points.iter().map(|(_, y)| y).fold(f64::INFINITY, |a, &b| a.min(b));
    let max_y = points.iter().map(|(_, y)| y).fold(f64::NEG_INFINITY, |a, &b| a.max(b));

    BoundingBox::new(min_x, min_y, max_x, max_y).ok()
}

/// Calculate bounding box for image XObject placement
/// Images are placed at (0,0)-(1,1) in user space, transformed by CTM
fn calculate_image_bbox(ctm: &[f64; 6]) -> Option<BoundingBox> {
    // Image corners in user space: (0,0), (1,0), (1,1), (0,1)
    let corners = [
        (0.0, 0.0),
        (1.0, 0.0),
        (1.0, 1.0),
        (0.0, 1.0),
    ];

    // Transform corners by CTM
    let [a, b, c, d, e, f] = ctm;
    let transformed: Vec<(f64, f64)> = corners.iter()
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
                    m[i] = val.as_f32().unwrap_or(if i == 0 || i == 3 { 1.0 } else { 0.0 }) as f64;
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
    let corners = [
        (x1, y1),
        (x2, y1),
        (x2, y2),
        (x1, y2),
    ];

    let [a, b, c, d, e, f] = combined_ctm;
    let transformed: Vec<(f64, f64)> = corners.iter()
        .map(|(x, y)| {
            (a * x + c * y + e, b * x + d * y + f)
        })
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

    let xobj_ref = xobject_dict.unwrap().get(xobj_name).ok()
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
    let subtype = xobj_stream.unwrap().dict.get(b"Subtype").ok()
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
            components.len(), crop_box.left, crop_box.bottom, crop_box.right, crop_box.top
        )));
    }

    #[cfg(debug_assertions)]
    eprintln!("[DEBUG] filter_components: {} components, crop: ({:.1}, {:.1}, {:.1}, {:.1})",
        components.len(), crop_box.left, crop_box.bottom, crop_box.right, crop_box.top);

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
                            "[DEBUG] Keeping Form XObject (no bbox calculated)"
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
                                image_bbox.left, image_bbox.bottom, image_bbox.right, image_bbox.top
                            )));
                        }
                    }
                } else {
                    // No bbox calculated - keep to be safe
                    stats.images_kept += 1;
                    output.push(operator);
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        web_sys::console::log_1(&JsValue::from_str(&format!(
            "[DEBUG] Component stats: {} text blocks, {} graphics state, {} form XObjects, {}/{} paths kept, {}/{} images kept",
            stats.text_blocks, stats.graphics_state, stats.form_xobjects,
            stats.paths_kept, stats.paths_total,
            stats.images_kept, stats.images_total
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
    !(component_bbox.right < left ||
      component_bbox.left > right ||
      component_bbox.top < bottom ||
      component_bbox.bottom > top)
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
        },
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
                op.operator, op.operands.len()
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
        eprintln!("[DEBUG] Content stream has {} operations", content.operations.len());
        if content.operations.len() == 1 {
            let op = &content.operations[0];
            eprintln!("[DEBUG] Single op: '{}', operands: {:?}", op.operator, op.operands);
            if let Some(Object::Name(name)) = op.operands.first() {
                eprintln!("[DEBUG] First operand is Name: {}", String::from_utf8_lossy(name));
            }
        } else if content.operations.len() > 0 {
            // Check if stream starts with text operators (potential issue)
            let first_op = &content.operations[0];
            if matches!(first_op.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
                eprintln!("[WARNING] Stream starts with text operator '{}' without BT!", first_op.operator);
                eprintln!("[WARNING] This is invalid PDF - text operators should be inside BT/ET blocks");
            }

            // Count BT/ET pairs
            let bt_count = content.operations.iter().filter(|op| op.operator == "BT").count();
            let et_count = content.operations.iter().filter(|op| op.operator == "ET").count();
            let text_ops_count = content.operations.iter()
                .filter(|op| matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\""))
                .count();

            eprintln!("[DEBUG] BT: {}, ET: {}, Text ops: {}", bt_count, et_count, text_ops_count);

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
                        eprintln!("[DEBUG] Component {} (Path): bbox=({:.1},{:.1},{:.1},{:.1})",
                            i, b.left, b.bottom, b.right, b.top);
                    }
                }
                ContentComponent::FormXObject { bbox, .. } => {
                    if let Some(b) = bbox {
                        eprintln!("[DEBUG] Component {} (FormXObject): bbox=({:.1},{:.1},{:.1},{:.1})",
                            i, b.left, b.bottom, b.right, b.top);
                    } else {
                        eprintln!("[DEBUG] Component {} (FormXObject): no bbox", i);
                    }
                }
                ContentComponent::TextBlock { operators } => {
                    eprintln!("[DEBUG] Component {} (TextBlock): {} ops", i, operators.len());
                }
                ContentComponent::GraphicsState { operators } => {
                    eprintln!("[DEBUG] Component {} (GraphicsState): {} ops", i, operators.len());
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
            web_sys::console::log_1(&JsValue::from_str("[DEBUG] No operations removed - keeping original content stream"));
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
    doc: &Document,
    resources: &Dictionary,
    xobj_name: &[u8],
) -> Result<ObjectId> {
    // Look up XObject in Resources
    let xobject_ref = resources.get(b"XObject")?;
    let xobject_dict = xobject_ref.as_dict()
        .map_err(|_| Error::PdfParse("XObject is not a dictionary".to_string()))?;

    let xobj_ref = xobject_dict
        .get(xobj_name)
        .ok()
        .and_then(|obj| obj.as_reference().ok())
        .ok_or_else(|| Error::PdfParse(format!(
            "XObject {} not found in Resources",
            String::from_utf8_lossy(xobj_name)
        )))?;

    Ok(xobj_ref)
}

/// Get Form XObject reference and resources for later filtering
/// Returns (ObjectId, Option<Dictionary>) if it's a Form XObject
fn get_form_xobject_ref(
    doc: &Document,
    resources: &Dictionary,
    xobj_name: &[u8],
) -> Result<(ObjectId, Option<Dictionary>)> {
    // Look up XObject in Resources
    let xobject_ref = resources.get(b"XObject")?;
    let xobject_dict = xobject_ref.as_dict()
        .map_err(|_| Error::PdfParse("XObject is not a dictionary".to_string()))?;

    let xobj_ref = xobject_dict
        .get(xobj_name)
        .ok()
        .and_then(|obj| obj.as_reference().ok())
        .ok_or_else(|| Error::PdfParse(format!(
            "XObject {} not found in Resources",
            String::from_utf8_lossy(xobj_name)
        )))?;

    // Get the XObject stream to check if it's a Form
    let xobj_stream = doc
        .get_object(xobj_ref)
        .map_err(|e| Error::PdfParse(format!("Failed to get XObject: {}", e)))?
        .as_stream()
        .map_err(|e| Error::PdfParse(format!("XObject is not a stream: {}", e)))?;

    // Check if it's a Form XObject (Subtype = Form)
    let is_form = xobj_stream.dict.get(b"Subtype")
        .ok()
        .and_then(|obj| obj.as_name().ok())
        .map(|name| name == b"Form")
        .unwrap_or(false);

    if !is_form {
        // Not a Form XObject (probably an Image), skip
        return Err(Error::PdfParse("Not a Form XObject".to_string()));
    }

    // Get Form XObject's Resources (it may have its own)
    let form_resources = xobj_stream.dict.get(b"Resources")
        .ok()
        .and_then(|obj| obj.as_dict().ok())
        .map(|d| d.clone());

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
    let (filtered_content, nested_form_xobjects) = filter_content_stream(doc, xobj_stream, xobj_resources.as_ref(), crop_box)?;

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
                    filtered.extend(path_ops_buffer.drain(..));
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
            "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" => {
                true
            }

            // XObject operator - collect Form XObjects for later filtering
            "Do" => {
                // Extract XObject name and collect it for second pass
                if let Some(Object::Name(xobj_name)) = op.operands.first() {
                    if let Some(resources_dict) = resources {
                        // Try to get the XObject reference
                        if let Ok((xobj_id, xobj_resources)) = get_form_xobject_ref(doc, resources_dict, xobj_name) {
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
                eprintln!("[DEBUG] Filtered out: {} (operands: {})", operator, op.operands.len());
            }
        }
    }

    Ok((filtered, form_xobjects))
}

/// Create PDF operations for a rectangular clipping path
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
fn is_point_near_box(point: (f64, f64), bbox: &BoundingBox, margin: f64) -> bool {
    let (x, y) = point;
    x >= bbox.left - margin
        && x <= bbox.right + margin
        && y >= bbox.bottom - margin
        && y <= bbox.top + margin
}

/// Check if a path intersects with the bounding box
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
    let min_x = path.iter().map(|(x, _)| x).fold(f64::INFINITY, |a, &b| a.min(b));
    let max_x = path.iter().map(|(x, _)| x).fold(f64::NEG_INFINITY, |a, &b| a.max(b));
    let min_y = path.iter().map(|(_, y)| y).fold(f64::INFINITY, |a, &b| a.min(b));
    let max_y = path.iter().map(|(_, y)| y).fold(f64::NEG_INFINITY, |a, &b| a.max(b));

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
        assert_eq!(extract_number(&operands, 1), Some(3.14));
        assert_eq!(extract_number(&operands, 2), None);
    }
}
