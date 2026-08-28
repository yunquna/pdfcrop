use pdfcrop::{normalize_to_target, BoundingBox, TargetAlignment, TargetPage};

#[test]
fn centers_detected_content_inside_4x6() {
    let content = BoundingBox::new(33.0, 356.0, 315.0, 761.0).unwrap();
    let page = BoundingBox::new(0.0, 0.0, 612.0, 792.0).unwrap();
    let target = TargetPage::new(288.0, 432.0, TargetAlignment::ContentCenter).unwrap();

    let actual = normalize_to_target(content, page, target).unwrap();

    assert_eq!(actual.width(), 288.0);
    assert_eq!(actual.height(), 432.0);
    assert!(actual.left <= content.left);
    assert!(actual.right >= content.right);
    assert!(actual.bottom <= content.bottom);
    assert!(actual.top >= content.top);
}

#[test]
fn rejects_content_larger_than_target() {
    let content = BoundingBox::new(0.0, 0.0, 400.0, 500.0).unwrap();
    let page = BoundingBox::new(0.0, 0.0, 612.0, 792.0).unwrap();
    let target = TargetPage::new(288.0, 432.0, TargetAlignment::ContentCenter).unwrap();

    let error = normalize_to_target(content, page, target).unwrap_err();
    assert!(error.to_string().contains("content exceeds target page"));
}
