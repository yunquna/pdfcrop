# Content Filtering TODO

## Overview
The `--clip` flag is intended to remove PDF content outside the crop box for privacy/security purposes. Currently, it works well for properly-formed PDFs but has limitations with malformed PDFs that have text operators outside BT/ET blocks.

## Current Status

### Working ✅
1. **Malformed PDF Support**: Text operators (TJ, Tj) outside BT/ET blocks are preserved
2. **No Visual Shifts**: Original content streams are kept when no filtering occurs
3. **Path/Image Filtering**: Graphics paths and images outside crop box are correctly removed
4. **Proper PDFs**: Well-formed PDFs with BT/ET blocks work correctly

### Not Working ❌
1. **Orphaned Text Filtering**: Cannot spatially filter text operators outside BT/ET blocks
2. **Incomplete Position Tracking**: Text positions don't account for page-level transformations

## Remaining Work

### 1. Implement Proper Text Bbox Calculation
**Problem**: Orphaned text operators (outside BT/ET) cannot be filtered because we can't calculate their actual bounding boxes.

**Current Code Location**: `src/content_filter.rs` lines 414-438

**What's Needed**:
- Parse text content from Tj/TJ operands
- Load font metrics from PDF resources
- Calculate actual text width using character advances
- Apply full transformation stack (CTM + text matrix)

**Suggested Approach**:
```rust
// In parse_into_components(), for orphaned text operators:
1. Extract text string from operands
2. Get current font from resources using font name
3. For each character:
   - Get glyph width from font
   - Sum up advances
4. Calculate bbox: (x, y, x+width, y+height)
5. Apply CTM transformation to get page coordinates
```

### 2. Use Hayro for Accurate Text Measurement
**Alternative Approach**: Use hayro's rendering to determine actual text bounds

**Implementation**:
```rust
// In content_filter_render.rs
pub fn calculate_text_bbox(
    pdf_bytes: &[u8],
    page_num: usize,
    text_op: &Operation,
    state: &GraphicsState,
) -> Result<BoundingBox> {
    // 1. Create minimal PDF with just this text operation
    // 2. Render with hayro at high resolution
    // 3. Find non-white pixels to get actual bounds
    // 4. Return calculated bbox
}
```

### 3. Fix Coordinate System Transformation
**Problem**: Text positions show as (0, -10, -20) instead of actual page coordinates

**Investigation Needed**:
- Check if there's a page-level transformation matrix
- Verify CTM is being properly tracked through graphics state saves/restores
- Ensure text matrix and CTM are being combined correctly

**Debug Points**:
- `src/content_filter.rs` line 475-480 (Tm handling)
- `src/content_filter.rs` line 35-50 (GraphicsState transform methods)

## Test Procedure

### Test File
`A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf` - Has malformed content streams with text operators outside BT/ET blocks

### Test Commands

#### 1. Basic Functionality Test
```bash
# Without clip (baseline)
./target/release/pdfcrop --bbox "494.26 465.45 590.31 546.72" --shrink-to-content A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf /tmp/no_clip.pdf

# With clip (should be visually identical, but with content removed)
./target/release/pdfcrop --bbox "494.26 465.45 590.31 546.72" --clip --shrink-to-content A_Neural_Receiver_for_5G_NR_Multi-User_MIMO.pdf /tmp/with_clip.pdf

# Compare visually
open /tmp/no_clip.pdf /tmp/with_clip.pdf
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
# [WARNING] Orphaned 'TJ' at (0.0, 0.0)
# [WARNING] Orphaned 'TJ' at (0.0, -10.0)
# These positions are wrong - should be in page coordinates
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
1. **Conservative Filtering**: Orphaned text is always kept to avoid data loss
2. **Position Calculation**: Text positions don't reflect actual page coordinates
3. **Font Metrics**: No font width calculation, so bbox estimates are rough

## Related Files
- `src/content_filter.rs` - Main filtering logic
- `src/content_filter_render.rs` - Rendering-based filtering (started but incomplete)
- `src/pdf_ops.rs` - Calls content filtering
- `src/crop.rs` - Crop box calculation

## References
- PDF Reference 1.7 - Section 5.3 (Text Objects)
- PDF Reference 1.7 - Section 8.4 (Font Metrics)
- hayro documentation - Text rendering