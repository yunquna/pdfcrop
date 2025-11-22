//! Download a PDF from an online source and crop it
//!
//! This example demonstrates:
//! - Downloading a PDF from the internet
//! - Processing it with pdfcrop library
//! - Handling network errors gracefully
//!
//! Usage: cargo run --example crop_online_pdf

use pdfcrop::{crop_pdf, CropOptions, Margins};
use std::fs;

const PDF_URL: &str = "https://wqzhao.org/assets/zhao2024flexible.pdf";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("PDF Cropping from Online Source");
    println!("================================\n");

    // Check if we should download or use cached version
    let cached_file = "zhao2024flexible.pdf";
    let cached_exists = std::path::Path::new(cached_file).exists();

    let pdf_bytes = if cached_exists {
        println!("Found cached PDF: {}", cached_file);
        println!("Delete this file to force re-download\n");
        fs::read(cached_file)?
    } else {
        println!("Downloading PDF from: {}", PDF_URL);
        println!("This may take a moment...\n");

        match download_pdf(PDF_URL) {
            Ok(bytes) => {
                println!("✓ Downloaded {} bytes", bytes.len());

                // Cache the downloaded PDF
                fs::write(cached_file, &bytes)?;
                println!("✓ Cached to {}\n", cached_file);

                bytes
            }
            Err(e) => {
                eprintln!("✗ Failed to download PDF: {}", e);
                eprintln!("\nPossible reasons:");
                eprintln!("  - No internet connection");
                eprintln!("  - URL is unreachable");
                eprintln!("  - Network timeout");
                eprintln!("\nYou can try again or use a local PDF file instead.");
                return Err(e);
            }
        }
    };

    println!("Processing PDF with pdfcrop...");

    // Crop with default margins (0)
    let options = CropOptions {
        margins: Margins::none(),
        bbox_method: pdfcrop::BBoxMethod::ContentStream,
        verbose: true,
        ..Default::default()
    };

    let cropped = crop_pdf(&pdf_bytes, options)?;

    let output_file = "zhao2024flexible-cropped.pdf";
    fs::write(output_file, cropped)?;

    println!("\n✓ Successfully cropped PDF!");
    println!("  Original size: {} bytes", pdf_bytes.len());
    println!("  Output file: {}", output_file);

    // Show comparison
    let output_size = fs::metadata(output_file)?.len();
    let size_change = output_size as i64 - pdf_bytes.len() as i64;
    let size_change_pct = (size_change as f64 / pdf_bytes.len() as f64) * 100.0;

    println!("  Cropped size: {} bytes ({:+.1}%)", output_size, size_change_pct);

    println!("\nYou can now open the cropped PDF:");
    #[cfg(target_os = "macos")]
    println!("  open {}", output_file);
    #[cfg(target_os = "linux")]
    println!("  xdg-open {}", output_file);
    #[cfg(target_os = "windows")]
    println!("  start {}", output_file);

    Ok(())
}

/// Download PDF from URL with timeout and error handling
fn download_pdf(url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use reqwest::blocking::Client;
    use std::time::Duration;

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let response = client
        .get(url)
        .header("User-Agent", "pdfcrop-rust/0.1.0")
        .send()?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()).into());
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("pdf") && !content_type.is_empty() {
        eprintln!("Warning: Content-Type is '{}', expected 'application/pdf'", content_type);
    }

    let bytes = response.bytes()?.to_vec();

    // Verify it's actually a PDF (should start with "%PDF")
    if !bytes.starts_with(b"%PDF") {
        return Err("Downloaded file is not a valid PDF".into());
    }

    Ok(bytes)
}
