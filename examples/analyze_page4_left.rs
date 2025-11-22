//! Analyze what content is on page 4 left edge
//!
//! Usage: cargo run --example analyze_page4_left

use lopdf::{Document, content::Content};
use std::fs;

#[derive(Debug, Clone, Copy)]
struct Matrix {
    a: f64, b: f64, c: f64, d: f64, e: f64, f: f64,
}

impl Matrix {
    fn identity() -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 }
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

    fn transform_point(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdf_data = fs::read("zhao2024flexible.pdf")?;
    let doc = Document::load_mem(&pdf_data)?;

    let page_id = doc.page_iter().nth(3).ok_or("Page 4 not found")?;
    let content_data = doc.get_page_content(page_id)?;
    let content = Content::decode(&content_data)?;

    println!("Analyzing Page 4 left edge content (x < 50):");
    println!("=============================================\n");

    let mut ctm = Matrix::identity();
    let mut ctm_stack: Vec<Matrix> = Vec::new();
    let mut text_matrix = Matrix::identity();
    let mut text_line_matrix = Matrix::identity();
    let mut text_font_size = 12.0;

    for (i, operation) in content.operations.iter().enumerate() {
        match operation.operator.as_ref() {
            "q" => ctm_stack.push(ctm),
            "Q" => { if let Some(saved) = ctm_stack.pop() { ctm = saved; } }
            "cm" => {
                if operation.operands.len() >= 6 {
                    if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                        get_number(&operation, 0), get_number(&operation, 1),
                        get_number(&operation, 2), get_number(&operation, 3),
                        get_number(&operation, 4), get_number(&operation, 5),
                    ) {
                        ctm = ctm.concat(&Matrix { a, b, c, d, e, f });
                    }
                }
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
                        get_number(&operation, 0), get_number(&operation, 1),
                        get_number(&operation, 2), get_number(&operation, 3),
                        get_number(&operation, 4), get_number(&operation, 5),
                    ) {
                        text_matrix = Matrix { a, b, c, d, e, f };
                        text_line_matrix = text_matrix;
                    }
                }
            }
            "Td" | "TD" => {
                if let (Some(tx), Some(ty)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                    let translation = Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty };
                    text_line_matrix = text_line_matrix.concat(&translation);
                    text_matrix = text_line_matrix;
                }
            }
            "Tj" | "TJ" | "'" | "\"" => {
                if text_matrix.e >= 0.0 && text_matrix.e < 50.0 {
                    let text_str = if let Some(obj) = operation.operands.get(0) {
                        if let Ok(bytes) = obj.as_str() {
                            String::from_utf8_lossy(bytes).to_string()
                        } else if let Ok(array) = obj.as_array() {
                            let mut result = String::new();
                            for item in array {
                                if let Ok(bytes) = item.as_str() {
                                    result.push_str(&String::from_utf8_lossy(bytes));
                                }
                            }
                            result
                        } else {
                            "".to_string()
                        }
                    } else {
                        "".to_string()
                    };

                    println!("Op {:4}: {} at x={:6.2}, y={:6.2}, font_size={:4.1}",
                             i, operation.operator, text_matrix.e, text_matrix.f, text_font_size);
                    if !text_str.is_empty() {
                        println!("       Text: {:?}", text_str);
                    }
                    println!();
                }
            }
            "m" | "l" | "re" => {
                // Path operations
                if operation.operator == "m" || operation.operator == "l" {
                    if let (Some(x), Some(y)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                        let (tx, ty) = ctm.transform_point(x, y);
                        if tx >= 0.0 && tx < 50.0 {
                            println!("Op {:4}: {} path point at x={:6.2}, y={:6.2}", i, operation.operator, tx, ty);
                        }
                    }
                } else if operation.operator == "re" {
                    if let (Some(x), Some(y), Some(w), Some(_h)) = (
                        get_number(&operation, 0), get_number(&operation, 1),
                        get_number(&operation, 2), get_number(&operation, 3),
                    ) {
                        let (tx, _ty) = ctm.transform_point(x, y);
                        if tx >= 0.0 && tx < 50.0 {
                            println!("Op {:4}: re rectangle at x={:6.2}, width={:6.2}", i, tx, w);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn get_number(op: &lopdf::content::Operation, index: usize) -> Option<f64> {
    op.operands.get(index).and_then(|obj| {
        obj.as_f32().ok().map(|f| f as f64).or_else(|| obj.as_i64().ok().map(|i| i as f64))
    })
}
