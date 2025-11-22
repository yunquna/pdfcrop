# pdfcrop

A Rust library and CLI tool for cropping PDF files with automatic bounding box detection.

This is a modern Rust implementation inspired by the original `pdfcrop` tool from TeX Live, designed to be both a reusable library and a command-line tool. It features pure Rust implementation with WASM compatibility in mind.

## Features

- **Automatic bounding box detection** - Parses PDF content streams to find actual content boundaries
- **Manual bbox override** - Specify exact crop regions when needed
- **Flexible margins** - Add uniform or per-side margins (left, top, right, bottom)
- **Page-specific options** - Different bounding boxes for odd/even pages
- **Library + CLI** - Use as a Rust library or standalone command-line tool
- **WASM-ready** - Pure Rust design enables future web applications

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

## Dependencies

- **lopdf** - PDF parsing and manipulation
- **clap** - CLI argument parsing
- **thiserror** - Error handling
- **anyhow** - CLI error context

All dependencies are pure Rust and WASM-compatible.

## License

MIT OR Apache-2.0

## Comparison with Original pdfcrop

| Feature | Original (TeX Live) | This (Rust) |
|---------|-------------------|-------------|
| Language | Perl | Rust |
| Bbox detection | Ghostscript | Content stream parsing |
| Dependencies | Ghostscript, TeX | None (pure Rust) |
| WASM support | No | Yes (designed for it) |
| Library API | No | Yes |
| Performance | Fast | Fast |
| Margin support | ✓ | ✓ |
| Bbox override | ✓ | ✓ |
| Odd/even pages | ✓ | ✓ |

## Contributing

Contributions welcome! This project is in active development.

Areas for improvement:
- More PDF operators in content stream parser
- Image bbox detection
- Hi-res bbox support
- PDF version preservation
- Streaming support for large files
