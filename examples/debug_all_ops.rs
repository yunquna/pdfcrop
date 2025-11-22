//! Show ALL PDF operators in a range
//!
//! Usage: cargo run --example debug_all_ops <pdf_file> <page_num> <start_op> <end_op>

use lopdf::{Document, content::Content};
use std::env;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 5 {
        eprintln!("Usage: {} <pdf_file> <page_num> <start_op> <end_op>", args[0]);
        std::process::exit(1);
    }

    let pdf_file = &args[1];
    let page_num = args[2].parse::<usize>().unwrap_or(0);
    let start_op = args[3].parse::<usize>().unwrap_or(0);
    let end_op = args[4].parse::<usize>().unwrap_or(100);

    println!("Debug: All PDF Operations");
    println!("=========================");
    println!("File: {}", pdf_file);
    println!("Page: {}", page_num + 1);
    println!("Operations: {} to {}\n", start_op, end_op);

    let pdf_data = fs::read(pdf_file)?;
    let doc = Document::load_mem(&pdf_data)?;

    let page_id = doc.page_iter().nth(page_num).ok_or("Page not found")?;
    let content_data = doc.get_page_content(page_id)?;
    let content = Content::decode(&content_data)?;

    for (i, operation) in content.operations.iter().enumerate() {
        if i >= start_op && i <= end_op {
            println!("{:4}. {:8} {:?}", i, operation.operator, operation.operands);
        }
    }

    Ok(())
}
