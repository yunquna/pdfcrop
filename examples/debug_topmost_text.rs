//! Find the topmost text operation on a page
//!
//! Usage: cargo run --example debug_topmost_text <pdf_file> <page_num>

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

    println!("Finding topmost text on page {}:", page_num + 1);
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
                        String::from_utf8_lossy(&text[..text.len().min(30)]).to_string()
                    } else if let Ok(array) = obj.as_array() {
                        format!("[array len={}]", array.len())
                    } else {
                        "".to_string()
                    }
                } else {
                    "".to_string()
                };

                // Calculate top of text (baseline + ascender height)
                const ASCENDER_RATIO: f64 = 0.75;
                let ascender_height = text_font_size * ASCENDER_RATIO;
                let text_top = text_matrix.f + ascender_height;

                text_positions.push((i, text_matrix.e, text_matrix.f, text_top, text_preview));
            }
            _ => {}
        }
    }

    // Sort by top y position (highest first)
    text_positions.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());

    println!("Topmost 20 text operations:");
    println!("----------------------------");
    for (i, (op_idx, x, baseline_y, top_y, preview)) in text_positions.iter().take(20).enumerate() {
        println!("{:2}. Op {:4}: x={:8.2}, baseline_y={:8.2}, top_y={:8.2}, text: {:?}",
                 i + 1, op_idx, x, baseline_y, top_y, preview);
    }

    if let Some((_, _, _, max_y, _)) = text_positions.first() {
        println!("\nTopmost Y coordinate (top of text): {:.2}", max_y);
    }

    Ok(())
}

fn get_number(op: &lopdf::content::Operation, index: usize) -> Option<f64> {
    op.operands.get(index).and_then(|obj| {
        obj.as_f32().ok().map(|f| f as f64).or_else(|| obj.as_i64().ok().map(|i| i as f64))
    })
}
