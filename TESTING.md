# Testing Guide for pdfcrop CLI

This guide shows you how to test the pdfcrop command-line tool.

## Quick Start

### 1. Generate Test PDFs

You have two options for getting test PDFs:

#### Option A: Generate and Crop Test PDFs Locally

Generate test PDFs with known shapes and crop them:

```bash
cargo run --example crop_shapes_pdf
```

This will create 4 test PDFs and their cropped versions:
- `test_rectangle.pdf` + cropped version - Blue rectangle with 10pt margins
- `test_shapes.pdf` + cropped version - Multiple shapes (circle, rectangle, line) with 5pt margins
- `test_small_content.pdf` + cropped version - Small content in large page (no margins)
- `test_edge_content.pdf` + cropped version - Rectangle near edge with asymmetric margins

#### Option B: Download and Crop a Real PDF

Download a research paper from the internet and crop it:

```bash
cargo run --example crop_online_pdf
```

This will:
- Download a PDF from https://wqzhao.org/assets/zhao2024flexible.pdf
- Cache it locally (subsequent runs use the cached version)
- Crop it with ContentStream bbox detection (pure Rust, no Ghostscript required)
- Show before/after file sizes
- Handle network errors gracefully

**Accuracy Results** (zhao2024flexible.pdf, 5 pages):
```
Page | Left Error | Right Error | Top Error | Bottom Error
-----|------------|-------------|-----------|-------------
  1  |   0.00 pts |    0.49 pts |  0.04 pts |     0.31 pts
  2  |   0.20 pts |    0.00 pts |  0.50 pts |     0.31 pts
  3  |   0.00 pts |    0.29 pts |  0.46 pts |     0.31 pts
  4  |   0.22 pts |    0.02 pts |  0.50 pts |     0.31 pts
  5  |   0.20 pts |    0.16 pts |  0.42 pts |     0.31 pts
```
All pages within **0.5 points** of Ghostscript bbox device - essentially perfect!

### 2. Build the CLI

```bash
cargo build --release
```

The binary will be at `target/release/pdfcrop`

### 3. Run Basic Tests

```bash
# Basic crop (auto-detect bbox, no margins)
cargo run -- test_rectangle.pdf output.pdf

# With verbose output to see what's happening
cargo run -- --verbose test_shapes.pdf output.pdf

# Add uniform margins (10 points on all sides)
cargo run -- --margins "10" test_small_content.pdf output.pdf

# Add different margins (left, top, right, bottom)
cargo run -- --margins "5 10 15 20" test_edge_content.pdf output.pdf
```

## CLI Options Reference

### Basic Usage

```bash
pdfcrop [OPTIONS] <INPUT> [OUTPUT]
```

- `<INPUT>` - Input PDF file (use `-` for stdin)
- `[OUTPUT]` - Output PDF file (optional, defaults to `<input>-crop.pdf`)

### Options

#### `--margins "<values>"`

Add margins around the detected bounding box:

```bash
# Uniform margins (10pt on all sides)
cargo run -- --margins "10" input.pdf output.pdf

# Horizontal and vertical (left/right=5pt, top/bottom=10pt)
cargo run -- --margins "5 10" input.pdf output.pdf

# All four sides (left, top, right, bottom)
cargo run -- --margins "5 10 15 20" input.pdf output.pdf
```

#### `--bbox "<left> <bottom> <right> <top>"`

Manually specify bounding box instead of auto-detection:

```bash
# Crop to specific region
cargo run -- --bbox "50 50 300 400" input.pdf output.pdf
```

#### `--bbox-odd` and `--bbox-even`

Different bounding boxes for odd/even pages:

```bash
# Different crops for alternating pages
cargo run -- --bbox-odd "50 50 300 400" --bbox-even "60 60 310 410" input.pdf output.pdf
```

#### `--verbose` / `-v`

Show detailed processing information:

```bash
cargo run -- --verbose test_shapes.pdf output.pdf
```

Output shows:
- Number of pages being processed
- Detected bbox for each page
- Size of detected content
- Final bbox after margins applied

#### `--debug` / `-d`

Enable debug output (same as verbose currently):

```bash
cargo run -- --debug test_shapes.pdf output.pdf
```

## Test Scenarios

### Test 1: Basic Auto-Detection

```bash
# Generate test PDF
cargo run --example generate_test_pdf

# Crop with auto-detection
cargo run -- --verbose test_rectangle.pdf cropped_rect.pdf

# Expected: Should detect bbox around the blue rectangle (100,100) to (300,250)
```

### Test 2: Adding Margins

```bash
# Add 10pt margins
cargo run -- --margins "10" --verbose test_rectangle.pdf cropped_margins.pdf

# Expected: Bbox should be expanded by 10pt on each side
# Original: (100, 100, 300, 250)
# With margins: (90, 90, 310, 260)
```

### Test 3: Manual Bbox Override

```bash
# Manually specify crop region
cargo run -- --bbox "50 50 200 200" --verbose test_shapes.pdf cropped_manual.pdf

# Expected: Should ignore auto-detection and use specified bbox
```

### Test 4: Multiple Shapes

```bash
# Auto-detect bbox for multiple shapes
cargo run -- --verbose test_shapes.pdf cropped_shapes.pdf

# Expected: Bbox should encompass all shapes (circle, rectangle, and line)
```

### Test 5: Stdin/Stdout Support

```bash
# Read from stdin, write to specified file
cat test_rectangle.pdf | cargo run -- - output_from_stdin.pdf
```

### Test 6: Default Output Filename

```bash
# No output specified - creates test_rectangle-crop.pdf
cargo run -- test_rectangle.pdf

# Verify output file was created
ls -la test_rectangle-crop.pdf
```

## Viewing Results

To verify the cropping worked, open the output PDF in a viewer:

```bash
# macOS
open output.pdf

# Linux
xdg-open output.pdf

# Or use any PDF viewer
```

You should see:
- White space removed around content
- Content tightly cropped (plus any margins you specified)
- Page size reduced to match the cropped area

## Comparing with Original pdfcrop

If you have the original TeX Live `pdfcrop` installed:

```bash
# Generate test PDF
cargo run --example generate_test_pdf

# Crop with original pdfcrop
pdfcrop test_rectangle.pdf original_output.pdf

# Crop with Rust pdfcrop
cargo run -- test_rectangle.pdf rust_output.pdf

# Compare file sizes and appearance
ls -lh original_output.pdf rust_output.pdf
```

## Testing with Real PDFs

To test with your own PDFs:

```bash
# Basic crop
cargo run -- your_document.pdf cropped_output.pdf

# With margins
cargo run -- --margins "10" your_document.pdf cropped_output.pdf

# Verbose to see what's detected
cargo run -- --verbose your_document.pdf cropped_output.pdf
```

## Debugging Bbox Detection

If bbox detection isn't working as expected, use the debug examples:

### Find What Content is Being Detected

```bash
# See all operations contributing to bbox
cargo run --example debug_bbox_contributors your.pdf 0

# Find leftmost/rightmost/topmost text
cargo run --example debug_leftmost_text your.pdf 0
cargo run --example debug_rightmost_text your.pdf 0
cargo run --example debug_topmost_text your.pdf 0
```

### Check for Coordinate Transformations

```bash
# See all CTM transformations
cargo run --example debug_ctm your.pdf 0

# Find rotated or scaled text
cargo run --example debug_text_matrix your.pdf 0
```

### Compare with Ghostscript

```bash
# Get Ghostscript's bbox (requires gs installed)
gs -o /dev/null -sDEVICE=bbox your.pdf 2>&1 | grep HiRes

# Get our bbox
cargo run --example crop_online_pdf  # or use --verbose flag
```

## Troubleshooting

### "No content found on page X"

This error means the bbox detector didn't find any drawing operations. This can happen if:
- The page is truly empty
- Content uses PDF operators we don't yet parse
- Content is in images (we don't detect image bboxes yet)

**Solution**: Use `--bbox` to manually specify the bounding box

### Bbox is too large/small

If auto-detection doesn't capture all content:

**Solution 1**: Add margins to expand the detected area
```bash
cargo run -- --margins "20" input.pdf output.pdf
```

**Solution 2**: Manually specify bbox with `--bbox`
```bash
cargo run -- --bbox "0 0 612 792" input.pdf output.pdf
```

### Content is clipped

If the crop is too aggressive:

**Solution**: Add larger margins
```bash
cargo run -- --margins "30" input.pdf output.pdf
```

## Performance Testing

For large PDFs:

```bash
# Time the operation
time cargo run --release -- large_document.pdf output.pdf

# Process specific pages only (future feature)
# cargo run -- --pages "1-10" large_document.pdf output.pdf
```

## Automated Testing

Run the built-in tests:

```bash
# Unit tests
cargo test

# Integration tests with generated PDFs
cargo test --test integration_tests  # (when added)
```

## Installation for System-Wide Use

Once testing is complete, install globally:

```bash
cargo install --path .

# Now use directly
pdfcrop --help
pdfcrop --margins "10" input.pdf output.pdf
```

## Getting Help

```bash
# Show help message
cargo run -- --help

# Show version
cargo run -- --version
```
