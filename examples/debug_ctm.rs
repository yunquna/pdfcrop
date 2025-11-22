//! Debug CTM (Current Transformation Matrix) tracking
//!
//! Usage: cargo run --example debug_ctm <pdf_file> [page_num]

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
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
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

    println!("Debug: CTM Tracking");
    println!("===================");
    println!("File: {}", pdf_file);
    println!("Page: {}\n", page_num + 1);

    let pdf_data = fs::read(pdf_file)?;
    let doc = Document::load_mem(&pdf_data)?;

    let page_id = doc.page_iter().nth(page_num).ok_or("Page not found")?;
    let content_data = doc.get_page_content(page_id)?;
    let content = Content::decode(&content_data)?;

    let mut ctm = Matrix::identity();
    let mut ctm_stack: Vec<Matrix> = Vec::new();
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    let total_ops = content.operations.len();
    println!("Graphics state and drawing operations:");
    println!("Total operations: {}", total_ops);
    println!("Showing: q/Q/cm operations and ALL drawing operations");
    println!("--------------------------------------------------");

    for (i, operation) in content.operations.iter().enumerate() {
        match operation.operator.as_ref() {
            "q" => {
                ctm_stack.push(ctm);
                // println!("{:3}. q              [save state, stack depth: {}]", i, ctm_stack.len());
            }
            "Q" => {
                if let Some(saved_ctm) = ctm_stack.pop() {
                    ctm = saved_ctm;
                    // println!("{:3}. Q              [restore state, stack depth: {}]", i, ctm_stack.len());
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
                        let new_matrix = Matrix { a, b, c, d, e, f };
                        ctm = ctm.concat(&new_matrix);
                        println!("{:3}. cm [{:.2}, {:.2}, {:.2}, {:.2}, {:.2}, {:.2}]", i, a, b, c, d, e, f);
                        println!("      -> CTM now: [{:.2}, {:.2}, {:.2}, {:.2}, {:.2}, {:.2}]",
                                 ctm.a, ctm.b, ctm.c, ctm.d, ctm.e, ctm.f);
                    }
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

                    println!("{:3}. re ({:.2}, {:.2}, {:.2}, {:.2})", i, x, y, w, h);
                    println!("      -> Transformed: ({:.2}, {:.2}) to ({:.2}, {:.2})", x1, y1, x2, y2);

                    min_x = min_x.min(x1).min(x2);
                    min_y = min_y.min(y1).min(y2);
                    max_x = max_x.max(x1).max(x2);
                    max_y = max_y.max(y1).max(y2);
                }
            }
            "m" | "l" => {
                if let (Some(x), Some(y)) = (get_number(&operation, 0), get_number(&operation, 1)) {
                    let (tx, ty) = ctm.transform_point(x, y);
                    println!("{:3}. {} ({:.2}, {:.2}) -> ({:.2}, {:.2})", i, operation.operator, x, y, tx, ty);

                    min_x = min_x.min(tx);
                    min_y = min_y.min(ty);
                    max_x = max_x.max(tx);
                    max_y = max_y.max(ty);
                }
            }
            _ => {}
        }
    }

    println!("\n Calculated BBox:");
    println!("------------------");
    if min_x != f64::INFINITY {
        println!("({:.2}, {:.2}) to ({:.2}, {:.2})", min_x, min_y, max_x, max_y);
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
