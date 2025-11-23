# Content Filtering TODO

## Overview
The `--clip` flag is intended to remove PDF content outside the crop box for privacy/security purposes. Currently, it works well for properly-formed PDFs but has limitations with malformed PDFs that have text operators outside BT/ET blocks.

## Current Status

### Working ✅
1. **Malformed PDF Support**: Text operators (TJ, Tj) outside BT/ET blocks are preserved
2. **No Visual Shifts**: Original content streams are kept when no filtering occurs
3. **Path/Image Filtering**: Graphics paths and images outside crop box are correctly removed
4. **Proper PDFs**: Well-formed PDFs with BT/ET blocks work correctly
5. **Orphaned Text Filtering**: Text operators outside BT/ET blocks are now measured using real font metrics and filtered when they fall entirely outside the crop box

### Remaining Gaps ⚠️
1. **Incomplete Font Coverage**: If a page references fonts without usable metrics (rare), orphaned text falls back to "keep" mode for safety
2. **Advanced Transformations**: Rendering-based validation (hayro) would still be the gold standard for exotic text effects, warping, or vertical writing modes

## Remaining Work

### 1. Rendering-Based Validation (Optional but Ideal)
The new bounding-box approach relies on PDF font metrics and transformation tracking. For bulletproof results on every PDF ever produced, integrating hayro-based validation is still valuable:
1. Render orphaned components in isolation
2. Inspect rendered pixels to confirm visibility inside the crop
3. Use that as the ultimate decision-maker (fonts + transformations + clipping handled automatically)

**Status**: Previously scaffolded in `src/content_filter_render.rs`, but removed now that it was unused. If we want a “paranoid mode,” reintroduce a minimal render-backed helper instead of keeping dead code.

### 2. Font Metrics Fallbacks
Most modern PDFs embed fonts with valid `/Widths` or `/W` arrays, which the new `FontCache` consumes. Remaining polish tasks:
- Support Identity-V or exotic CMap encodings
- Detect Type3 fonts and fall back to hayro rendering
- Cache metrics across pages & form XObjects (currently page-level caches)

### 3. Comprehensive Telemetry
The new filtering path reports stats for orphaned text (kept vs removed). Surfacing that information via `--verbose` / WASM logs will help users confirm when text was actually removed or retained for safety.

## Test Procedure

### Test File
`A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf` - Has malformed content streams with text operators outside BT/ET blocks

### Test Commands

#### 1. Basic Functionality Test
```bash
# With clip (should be visually identical, but with content removed)
./target/release/pdfcrop --bbox "494.26 465.45 590.31 546.72" --clip --shrink-to-content A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf /tmp/with_clip.pdf

# Compare visually
open A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf /tmp/with_clip.pdf
```

#### 2. Debug Content Filtering
```bash
# Check what's being filtered
./target/release/pdfcrop --bbox "494.26 465.45 590.31 546.72" --clip --shrink-to-content A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf /tmp/test.pdf 2>&1 | grep "removed)"

# Expected output:
# - Page 1: Currently shows "0 removed" (should show some removed)
# - Later pages: Shows "72 removed", "148 removed" (working correctly)
```

#### 3. Check Orphaned Text Detection
```bash
# See orphaned text operators being detected
./target/release/pdfcrop --bbox "494.26 465.45 590.31 546.72" --clip --shrink-to-content A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf /tmp/test.pdf 2>&1 | grep "WARNING.*Orphaned"

# Shows positions like:
# [WARNING] Orphaned 'TJ' encountered without active font - keeping in stream
# These only appear if the PDF omits usable font metrics (rare). In that case, content is safely kept.
```

#### 4. Verify No Visual Shift
```bash
# Generate multiple times and compare file sizes
for i in 1 2 3; do
  ./target/release/pdfcrop --bbox "494.26 465.45 590.31 546.72" --clip A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf /tmp/test$i.pdf 2>/dev/null
done
ls -l /tmp/test*.pdf
# Sizes should be identical if deterministic
```

### Success Criteria
1. ✅ No visual differences between --clip and no-clip versions
2. ✅ No crashes or errors on malformed PDFs
3. ❌ Content outside crop box should be removed (currently not working for orphaned text)
4. ✅ File size should be reduced when content is filtered

### Known Issues
1. **Conservative Fallback**: Fonts without `Widths`/`W` arrays are retained (no best-effort removal)
2. **Rendering Mode**: hayro-based validation still TODO for 100% fidelity
3. **Vertical Text**: Identity-V and text rotations beyond standard baselines fall back to bounding-box approximations. Rendering validation will close this gap.

## Related Files
- `src/content_filter.rs` - Main filtering logic
- `src/pdf_ops.rs` - Calls content filtering
- `src/crop.rs` - Crop box calculation

## References
- PDF Reference 1.7 - Section 5.3 (Text Objects)
- PDF Reference 1.7 - Section 8.4 (Font Metrics)
- hayro documentation - Text rendering
