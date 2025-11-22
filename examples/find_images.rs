//! Find image operations in a PDF page
//!
//! Usage: cargo run --example find_images <pdf_file> <page_num>

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

    println!("Finding image operations on page {}:", page_num + 1);
    println!("==========================================\n");

    let mut found_images = false;

    for (i, operation) in content.operations.iter().enumerate() {
        match operation.operator.as_ref() {
            "Do" => {
                // Invoke XObject (image, form, etc.)
                if let Some(obj) = operation.operands.get(0) {
                    println!("{:4}. Do {:?} (XObject reference)", i, obj);
                    found_images = true;
                }
            }
            "BI" => {
                // Begin inline image
                println!("{:4}. BI (begin inline image)", i);
                found_images = true;
            }
            "ID" => {
                // Image data
                println!("{:4}. ID (image data)", i);
            }
            "EI" => {
                // End inline image
                println!("{:4}. EI (end inline image)", i);
            }
            _ => {}
        }
    }

    if !found_images {
        println!("No image operations found");
    }

    Ok(())
}
