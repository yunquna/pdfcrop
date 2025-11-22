//! Debug text matrices to find unusual transformations
//!
//! Usage: cargo run --example debug_text_matrix <pdf_file> <page_num>

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

    println!("Looking for unusual text matrices on page {}:", page_num + 1);
    println!("================================================\n");

    let mut found_unusual = false;

    for (i, operation) in content.operations.iter().enumerate() {
        if operation.operator == "Tm" {
            if operation.operands.len() >= 6 {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                    get_number(&operation, 0),
                    get_number(&operation, 1),
                    get_number(&operation, 2),
                    get_number(&operation, 3),
                    get_number(&operation, 4),
                    get_number(&operation, 5),
                ) {
                    // Look for unusual transformations
                    if a < 0.0 || d < 0.0 || b != 0.0 || c != 0.0 || a == 0.0 || d == 0.0 {
                        println!("{:4}. Tm [{:.4}, {:.4}, {:.4}, {:.4}, {:.2}, {:.2}]",
                                 i, a, b, c, d, e, f);
                        if a < 0.0 {
                            println!("      ^ Negative horizontal scale!");
                        }
                        if d < 0.0 {
                            println!("      ^ Negative vertical scale!");
                        }
                        if b != 0.0 || c != 0.0 {
                            println!("      ^ Rotation/skew present");
                        }
                        println!();
                        found_unusual = true;
                    }
                }
            }
        }
    }

    if !found_unusual {
        println!("No unusual text matrices found (all standard horizontal text)");
    }

    Ok(())
}

fn get_number(op: &lopdf::content::Operation, index: usize) -> Option<f64> {
    op.operands.get(index).and_then(|obj| {
        obj.as_f32().ok().map(|f| f as f64).or_else(|| obj.as_i64().ok().map(|i| i as f64))
    })
}
