//! # pdfcrop
//!
//! A library and CLI tool for cropping PDF files with automatic bounding box detection.
//!
//! This library provides functionality to:
//! - Detect bounding boxes of PDF content through content stream parsing
//! - Crop PDF pages with custom margins
//! - Support manual bounding box override
//! - Process PDFs page by page or in batch
//!
//! ## Library Usage Example
//!
//! ```no_run
//! use pdfcrop::{crop_pdf, CropOptions, BoundingBox, Margins};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let pdf_data = std::fs::read("input.pdf")?;
//!
//! let options = CropOptions {
//!     margins: Margins::uniform(10.0),
//!     bbox_override: None,
//!     ..Default::default()
//! };
//!
//! let cropped = crop_pdf(&pdf_data, options)?;
//! std::fs::write("output.pdf", cropped)?;
//! # Ok(())
//! # }
//! ```

pub mod bbox;
pub mod crop;
pub mod error;
pub mod ghostscript;
pub mod margins;
pub mod pdf_ops;

pub use bbox::{BoundingBox, detect_bbox};
pub use crop::crop_pdf;
pub use error::{Error, Result};
pub use margins::Margins;

/// Bounding box detection method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BBoxMethod {
    /// Use Ghostscript's bbox device (most accurate, requires Ghostscript installed)
    Ghostscript,
    /// Parse PDF content stream (pure Rust, works in WASM, less accurate)
    ContentStream,
    /// Try Ghostscript first, fall back to content stream if unavailable
    Auto,
}

/// Options for PDF cropping operations
#[derive(Debug, Clone)]
pub struct CropOptions {
    /// Margins to add around the detected or specified bounding box
    pub margins: Margins,

    /// Manual bounding box override (if None, auto-detect from content)
    pub bbox_override: Option<BoundingBox>,

    /// Bounding box override for odd pages only
    pub bbox_odd: Option<BoundingBox>,

    /// Bounding box override for even pages only
    pub bbox_even: Option<BoundingBox>,

    /// Bounding box detection method
    pub bbox_method: BBoxMethod,

    /// Enable verbose output (for debugging)
    pub verbose: bool,

    /// Clip content outside the crop box by adding a clipping path to the content stream
    /// When enabled, adds clipping commands to ensure content outside bbox is not rendered
    /// Note: This increases file size as it adds code without removing content (default: false)
    /// Most PDF viewers respect CropBox without needing explicit clipping
    pub clip_content: bool,

    /// When a manual bbox is specified, automatically shrink it to the actual content bounds
    /// This detects the real content within the specified bbox and uses that instead
    /// Useful for removing remaining margins within a manually specified region (default: false)
    pub shrink_to_content: bool,
}

impl Default for CropOptions {
    fn default() -> Self {
        Self {
            margins: Margins::none(),
            bbox_override: None,
            bbox_odd: None,
            bbox_even: None,
            bbox_method: BBoxMethod::ContentStream, // Pure Rust, WASM-compatible by default
            verbose: false,
            clip_content: false,  // Default: only set CropBox (standard PDF cropping behavior)
            shrink_to_content: false,  // Default: don't auto-shrink manual bbox
        }
    }
}
