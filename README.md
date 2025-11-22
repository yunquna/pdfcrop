# pdfcrop

A Rust library and CLI tool for cropping PDF files with **rendering-based automatic bounding box detection**.

Modern Rust reimplementation of the TeX Live `pdfcrop` tool, using actual PDF rendering to accurately detect content boundaries. Pure Rust, WASM-ready, and library-first design.

## Features

- **Rendering-based bbox detection** - Renders PDF pages to detect actual visible content (like Ghostscript)
- **Accurate and simple** - No heuristics, handles all PDF features automatically
- **Flexible margins** - Uniform or per-side margins (left, top, right, bottom)
- **Manual override** - Specify exact crop regions when needed
- **Library + CLI** - Use as a Rust library or standalone command-line tool
- **Pure Rust** - No external dependencies, WASM-compatible

## Quick Start

### CLI Usage

```bash
# Install
cargo install --path .

# Basic crop (auto-detect bbox)
pdfcrop input.pdf output.pdf

# With margins
pdfcrop --margins "10" input.pdf output.pdf

# Verbose mode to see detection details
pdfcrop --verbose input.pdf output.pdf

# Custom bbox
pdfcrop --bbox "50 50 500 700" input.pdf output.pdf

# Add content clipping (adds clipping path to stream - increases file size)
pdfcrop --clip input.pdf output.pdf

# Auto-shrink manual bbox to actual content (removes remaining margins)
pdfcrop --bbox "0 0 612 792" --shrink-to-content input.pdf output.pdf
```

### Library Usage

```rust
use pdfcrop::{crop_pdf, CropOptions, Margins};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdf_data = fs::read("input.pdf")?;

    let options = CropOptions {
        margins: Margins::uniform(10.0),
        verbose: true,
        ..Default::default()
    };

    let cropped = crop_pdf(&pdf_data, options)?;
    fs::write("output.pdf", cropped)?;

    Ok(())
}
```

## Examples

Try the included examples:

```bash
# Generate test PDFs with shapes and crop them
cargo run --example crop_shapes_pdf

# Download a real PDF and crop it
cargo run --example crop_online_pdf
```

See [TESTING.md](TESTING.md) for comprehensive testing guide.

## Development

```bash
# Build
cargo build --release

# Run tests
cargo test

# Run with verbose output
cargo run -- --verbose input.pdf output.pdf

# Check code
cargo clippy
cargo fmt
```

See [CLAUDE.local.md](CLAUDE.local.md) for detailed architecture and development guide.

## How It Works

pdfcrop uses **rendering-based detection** with **parallel processing**:
1. Renders PDF pages to bitmaps in parallel using [hayro](https://github.com/LaurenzV/hayro) and [rayon](https://github.com/rayon-rs/rayon)
2. Scans pixels to find the bounding box of non-white content
3. Converts pixel coordinates back to PDF points
4. Applies margins and sets the CropBox

This approach is simple, accurate, and automatically handles annotations, rotated text, transformed graphics, and all PDF features. **Multi-page PDFs are processed in parallel** for optimal performance.

### File Size Behavior

**By default**, pdfcrop only sets the PDF's CropBox:
- Standard PDF cropping method
- All viewers respect CropBox and hide content outside it
- File size may increase slightly (~2-20%) due to PDF rewriting
- Original content remains in file but is hidden

**With `--clip` flag**, adds a clipping path when manually specifying bbox:
- **Fast track:** Auto-detected bboxes skip clipping (no content outside detected bounds)
- Only applies to manual bbox specifications (`--bbox`, `--bbox-odd`, `--bbox-even`)
- Ensures content outside bbox is never rendered
- Useful for security/privacy when cropping to manual regions
- **Increases file size** slightly as it adds clipping code
- Note: `--shrink-to-content` also skips clipping (uses detected content)

### Shrink-to-Content

When you specify a manual bounding box, use `--shrink-to-content` to:
- Automatically detect the actual content within your specified region
- Remove remaining margins inside the manual bbox
- Useful when you know the general area but want precise cropping

**Example:**
```bash
# Specify full page, let it shrink to actual content
pdfcrop --bbox "0 0 612 792" --shrink-to-content input.pdf output.pdf
```

## Dependencies

- **lopdf** - PDF manipulation
- **hayro** - Pure Rust PDF rendering
- **rayon** - Parallel processing for multi-page PDFs
- **clap** - CLI parsing
- **thiserror/anyhow** - Error handling

All dependencies are pure Rust and WASM-compatible.

## Comparison with Original pdfcrop

| Feature | Original (TeX Live) | This (Rust) |
|---------|-------------------|-------------|
| Language | Perl | Rust |
| Bbox detection | Ghostscript (rendering) | hayro (rendering) |
| External dependencies | Ghostscript required | None |
| WASM support | No | Yes |
| Library API | No | Yes |
| Accuracy | Excellent | Excellent (matches Ghostscript) |

## License

MIT OR Apache-2.0
