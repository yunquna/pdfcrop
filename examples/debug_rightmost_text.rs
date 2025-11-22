//! Find the rightmost text operation on a page
//!
//! Usage: cargo run --example debug_rightmost_text <pdf_file> <page_num>

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

    println!("Finding rightmost text on page {}:", page_num + 1);
    println!("==========================================\n");

    let mut text_matrix = Matrix::identity();
    let mut text_line_matrix = Matrix::identity();
    let mut text_font_size = 12.0;

    let mut text_positions: Vec<(usize, f64, f64, f64, String)> = Vec::new();

    for (i, operation) in content.operations.iter().enumerate() {
        match operation.operator.as_ref() {
            "BT" => {
                text_matrix = Matrix::identity();
                text_line_matrix = Matrix::identity();
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
                // Text showing - record position
                let text_preview = if let Some(obj) = operation.operands.get(0) {
                    if let Ok(text) = obj.as_str() {
                        String::from_utf8_lossy(&text[..text.len().min(20)]).to_string()
                    } else if let Ok(array) = obj.as_array() {
                        format!("[array len={}]", array.len())
                    } else {
                        "".to_string()
                    }
                } else {
                    "".to_string()
                };

                // Estimate text width
                let text_width = estimate_text_width(&operation, text_font_size);
                let text_end_x = text_matrix.e + text_matrix.a * text_width;

                text_positions.push((i, text_matrix.e, text_end_x, text_matrix.f, text_preview));
            }
            _ => {}
        }
    }

    // Sort by end x position (rightmost edge)
    text_positions.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

    println!("Rightmost 20 text operations:");
    println!("----------------------------");
    for (i, (op_idx, start_x, end_x, y, preview)) in text_positions.iter().take(20).enumerate() {
        println!("{:2}. Op {:4}: x={:8.2}-{:8.2}, y={:8.2}, text: {:?}",
                 i + 1, op_idx, start_x, end_x, y, preview);
    }

    if let Some((_, _, max_x, _, _)) = text_positions.first() {
        println!("\nRightmost X coordinate: {:.2}", max_x);
    }

    Ok(())
}

fn get_number(op: &lopdf::content::Operation, index: usize) -> Option<f64> {
    op.operands.get(index).and_then(|obj| {
        obj.as_f32().ok().map(|f| f as f64).or_else(|| obj.as_i64().ok().map(|i| i as f64))
    })
}

fn estimate_text_width(op: &lopdf::content::Operation, font_size: f64) -> f64 {
    let char_count = if op.operator == "TJ" {
        // Array of strings
        if let Some(obj) = op.operands.get(0) {
            if let Ok(array) = obj.as_array() {
                let mut count = 0;
                for item in array {
                    if let Ok(bytes) = item.as_str() {
                        count += bytes.len();
                    }
                }
                count
            } else {
                0
            }
        } else {
            0
        }
    } else {
        // Single string
        if let Some(obj) = op.operands.get(0) {
            if let Ok(bytes) = obj.as_str() {
                bytes.len()
            } else {
                0
            }
        } else {
            0
        }
    };

    char_count as f64 * font_size * 0.25
}
