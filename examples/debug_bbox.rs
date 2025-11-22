//! Debug bbox detection by showing all operations found
//!
//! Usage: cargo run --example debug_bbox <pdf_file> [page_num]

use lopdf::{Document, content::Content};
use std::env;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <pdf_file> [page_num]", args[0]);
        std::process::exit(1);
    }

    let pdf_file = &args[1];
    let page_num = if args.len() > 2 {
        args[2].parse::<usize>().unwrap_or(0)
    } else {
        0
    };

    println!("Debug: PDF Content Stream Analysis");
    println!("===================================");
    println!("File: {}", pdf_file);
    println!("Page: {}\n", page_num + 1);

    let pdf_data = fs::read(pdf_file)?;
    let doc = Document::load_mem(&pdf_data)?;

    // Get the page
    let page_id = doc
        .page_iter()
        .nth(page_num)
        .ok_or("Page not found")?;

    // Get content stream
    let content_data = doc.get_page_content(page_id)?;
    let content = Content::decode(&content_data)?;

    println!("Total operations: {}\n", content.operations.len());

    // Track bbox as we parse
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    let mut op_count = std::collections::HashMap::new();

    println!("First 50 operations with coordinates:");
    println!("-------------------------------------");

    for (i, operation) in content.operations.iter().enumerate().take(50) {
        *op_count.entry(operation.operator.as_str()).or_insert(0) += 1;

        match operation.operator.as_ref() {
            "m" | "l" => {
                if let (Some(x), Some(y)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                    println!("{:3}. {} {:8.2} {:8.2}", i, operation.operator, x, y);
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (
                    get_number(&operation, 0),
                    get_number(&operation, 1),
                    get_number(&operation, 2),
                    get_number(&operation, 3),
                ) {
                    println!("{:3}. {} {:8.2} {:8.2} {:8.2} {:8.2}", i, operation.operator, x, y, w, h);
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x + w);
                    max_y = max_y.max(y + h);
                }
            }
            "Tm" => {
                if operation.operands.len() >= 6 {
                    if let (Some(x), Some(y)) = (get_number(&operation, 4), get_number(&operation, 5)) {
                        println!("{:3}. {} ... {:8.2} {:8.2}", i, operation.operator, x, y);
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                }
            }
            "cm" => {
                // Coordinate transformation matrix
                if operation.operands.len() >= 6 {
                    println!("{:3}. cm {:?}", i, &operation.operands);
                }
            }
            _ => {}
        }
    }

    println!("\nOperator frequency:");
    println!("------------------");
    let mut ops: Vec<_> = op_count.iter().collect();
    ops.sort_by_key(|&(_, count)| std::cmp::Reverse(*count));
    for (op, count) in ops.iter().take(20) {
        println!("{:8} : {}", op, count);
    }

    println!("\nCalculated BBox from first 50 ops:");
    println!("----------------------------------");
    if min_x != f64::INFINITY {
        println!("Min: ({:.2}, {:.2})", min_x, min_y);
        println!("Max: ({:.2}, {:.2})", max_x, max_y);
        println!("Size: {:.2} x {:.2}", max_x - min_x, max_y - min_y);
    } else {
        println!("No coordinates found");
    }

    Ok(())
}

fn get_number(op: &lopdf::content::Operation, index: usize) -> Option<f64> {
    op.operands.get(index).and_then(|obj| {
        obj.as_f32().ok().map(|f| f as f64).or_else(|| obj.as_i64().ok().map(|i| i as f64))
    })
}
