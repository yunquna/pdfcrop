//! Debug text operations to see what text we're detecting
//!
//! Usage: cargo run --example debug_text <pdf_file> <page_num>

use lopdf::{Document, content::Content};
use std::env;
use std::fs;

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

    println!("Text operations on page {}:", page_num + 1);
    println!("================================\n");

    let mut text_count = 0;
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;

    let mut current_x = 0.0;
    let mut current_y = 0.0;

    for (i, operation) in content.operations.iter().enumerate() {
        match operation.operator.as_ref() {
            "BT" => {
                println!("{:4}. BT (begin text)", i);
            }
            "ET" => {
                println!("{:4}. ET (end text)", i);
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
                        println!("{:4}. Tm [{:.2}, {:.2}, {:.2}, {:.2}, {:.2}, {:.2}]", i, a, b, c, d, e, f);
                        current_x = e;
                        current_y = f;
                        min_x = min_x.min(e);
                        max_x = max_x.max(e);
                    }
                }
            }
            "Td" | "TD" => {
                if let (Some(tx), Some(ty)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                    println!("{:4}. {} tx={:.2}, ty={:.2}", i, operation.operator, tx, ty);
                    current_x += tx;
                    current_y += ty;
                    min_x = min_x.min(current_x);
                    max_x = max_x.max(current_x);
                }
            }
            "Tf" => {
                if let Some(size) = get_number(&operation, 1) {
                    println!("{:4}. Tf font_size={:.2}", i, size);
                }
            }
            "Tj" | "'" => {
                if let Some(obj) = operation.operands.get(0) {
                    if let Ok(text) = obj.as_str() {
                        let preview = String::from_utf8_lossy(&text[..text.len().min(40)]);
                        println!("{:4}. {} text: len={} bytes, preview: {:?}",
                                 i, operation.operator, text.len(), preview);
                        text_count += 1;
                    }
                }
            }
            "TJ" => {
                if let Some(obj) = operation.operands.get(0) {
                    if let Ok(array) = obj.as_array() {
                        let mut total_chars = 0;
                        for item in array {
                            if let Ok(text) = item.as_str() {
                                total_chars += text.len();
                            }
                        }
                        println!("{:4}. TJ array: {} total bytes", i, total_chars);
                        text_count += 1;
                    }
                }
            }
            _ => {}
        }
    }

    println!("\n Summary:");
    println!("Total text operations: {}", text_count);
    if min_x != f64::INFINITY {
        println!("Text X range (Tm): {:.2} to {:.2}", min_x, max_x);
        println!("Width: {:.2} pts", max_x - min_x);
    }

    Ok(())
}

fn get_number(op: &lopdf::content::Operation, index: usize) -> Option<f64> {
    op.operands.get(index).and_then(|obj| {
        obj.as_f32().ok().map(|f| f as f64).or_else(|| obj.as_i64().ok().map(|i| i as f64))
    })
}
