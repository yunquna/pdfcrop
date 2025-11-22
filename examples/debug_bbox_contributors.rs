//! Find all operations that contribute to the bounding box
//!
//! Usage: cargo run --example debug_bbox_contributors <pdf_file> <page_num>

use lopdf::{Document, content::Content};
use std::env;
use std::fs;

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
    fn identity() -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 }
    }

    fn transform_point(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: {} <pdf_file> <page_num>", args[0]);
        std::process::exit(1);
    }

    let pdf_file = &args[1];
    let page_num = args[2].parse::<usize>().unwrap_or(0);

    let pdf_data = fs::read(pdf_file)?;
    let doc = Document::load_mem(&pdf_data)?;

    let page_id = doc.page_iter().nth(page_num).ok_or("Page not found")?;
    let content_data = doc.get_page_content(page_id)?;
    let content = Content::decode(&content_data)?;

    println!("Finding bbox contributors on page {}:", page_num + 1);
    println!("============================================\n");

    let mut ctm = Matrix::identity();
    let mut ctm_stack: Vec<Matrix> = Vec::new();

    let mut text_matrix = Matrix::identity();
    let mut text_font_size = 12.0;
    let mut text_line_matrix = Matrix::identity();

    let mut path_points: Vec<(f64, f64)> = Vec::new();
    let mut contributions: Vec<(usize, String, f64, f64)> = Vec::new();

    for (i, operation) in content.operations.iter().enumerate() {
        match operation.operator.as_ref() {
            "q" => {
                ctm_stack.push(ctm);
            }
            "Q" => {
                if let Some(saved_ctm) = ctm_stack.pop() {
                    ctm = saved_ctm;
                }
            }
            "cm" => {
                if operation.operands.len() >= 6 {
                    if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                        get_number(&operation, 0),
                        get_number(&operation, 1),
                        get_number(&operation, 2),
                        get_number(&operation, 3),
                        get_number(&operation, 4),
                        get_number(&operation, 5),
                    ) {
                        let transform = Matrix { a, b, c, d, e, f };
                        ctm = ctm.concat(&transform);
                    }
                }
            }
            "m" => {
                if let (Some(x), Some(y)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                    let (tx, ty) = ctm.transform_point(x, y);
                    path_points.push((tx, ty));
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                    let (tx, ty) = ctm.transform_point(x, y);
                    path_points.push((tx, ty));
                }
            }
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (
                    get_number(&operation, 0),
                    get_number(&operation, 1),
                    get_number(&operation, 2),
                    get_number(&operation, 3),
                ) {
                    let (x1, y1) = ctm.transform_point(x, y);
                    let (x2, y2) = ctm.transform_point(x + w, y + h);
                    path_points.push((x1, y1));
                    path_points.push((x2, y2));
                }
            }
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                // Path painting - record all path points
                for &(x, y) in &path_points {
                    contributions.push((i, format!("Path {}", operation.operator), x, y));
                }
                path_points.clear();
            }
            "n" => {
                path_points.clear();
            }
            "BT" => {
                text_matrix = ctm;
                text_line_matrix = ctm;
            }
            "Tf" => {
                if let Some(size) = get_number(&operation, 1) {
                    text_font_size = size.abs();
                }
            }
            "Tm" => {
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
                    }
                }
            }
            "Td" | "TD" => {
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
                }
            }
            "Tj" | "TJ" | "'" | "\"" => {
                let text_preview = if let Some(obj) = operation.operands.get(0) {
                    if let Ok(text) = obj.as_str() {
                        String::from_utf8_lossy(&text[..text.len().min(10)]).to_string()
                    } else if let Ok(array) = obj.as_array() {
                        format!("[arr:{}]", array.len())
                    } else {
                        "".to_string()
                    }
                } else {
                    "".to_string()
                };

                contributions.push((i, format!("Text {}: '{}'", operation.operator, text_preview), text_matrix.e, text_matrix.f));
            }
            _ => {}
        }
    }

    // Sort by x position
    contributions.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());

    println!("Leftmost 20 bbox contributors:");
    println!("-------------------------------");
    for (idx, (op_idx, desc, x, y)) in contributions.iter().take(20).enumerate() {
        println!("{:2}. Op {:4}: x={:8.2}, y={:8.2}, type: {}", idx + 1, op_idx, x, y, desc);
    }

    if let Some((_, _, min_x, _)) = contributions.first() {
        println!("\nLeftmost X coordinate: {:.2}", min_x);
    }

    Ok(())
}

fn get_number(op: &lopdf::content::Operation, index: usize) -> Option<f64> {
    op.operands.get(index).and_then(|obj| {
        obj.as_f32().ok().map(|f| f as f64).or_else(|| obj.as_i64().ok().map(|i| i as f64))
    })
}
