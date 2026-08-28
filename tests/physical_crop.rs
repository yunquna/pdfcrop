#[macro_use]
extern crate lopdf;

use lopdf::{Document, Object, Stream};
use pdfcrop::pdf_ops::apply_page_boxes;
use pdfcrop::{BoundingBox, PageBoxPolicy};

const PAGE_BOX_KEYS: [&[u8]; 5] = [b"MediaBox", b"CropBox", b"TrimBox", b"BleedBox", b"ArtBox"];

fn one_page_document() -> Document {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
    let media_box = || vec![0.into(), 0.into(), 612.into(), 792.into()];

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "MediaBox" => media_box(),
        "CropBox" => media_box(),
        "TrimBox" => media_box(),
        "BleedBox" => media_box(),
        "ArtBox" => media_box(),
    });

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    doc
}

fn round_trip(mut doc: Document) -> Document {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    Document::load_mem(&bytes).unwrap()
}

fn page_box(doc: &Document, key: &[u8]) -> [f64; 4] {
    let page_id = doc.page_iter().next().unwrap();
    let values = doc
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(key)
        .unwrap()
        .as_array()
        .unwrap();
    std::array::from_fn(|index| {
        values[index]
            .as_f32()
            .map(f64::from)
            .or_else(|_| values[index].as_i64().map(|value| value as f64))
            .unwrap()
    })
}

#[test]
fn physical_policy_sets_all_standard_page_boxes() {
    let mut doc = one_page_document();
    let crop = BoundingBox::new(36.0, 324.0, 324.0, 756.0).unwrap();

    apply_page_boxes(&mut doc, 0, &crop, PageBoxPolicy::Physical, false).unwrap();
    let reopened = round_trip(doc);

    for key in PAGE_BOX_KEYS {
        assert_eq!(page_box(&reopened, key), [36.0, 324.0, 324.0, 756.0]);
    }
}

#[test]
fn crop_only_policy_preserves_physical_page_boxes() {
    let mut doc = one_page_document();
    let crop = BoundingBox::new(36.0, 324.0, 324.0, 756.0).unwrap();

    apply_page_boxes(&mut doc, 0, &crop, PageBoxPolicy::CropOnly, false).unwrap();
    let reopened = round_trip(doc);

    assert_eq!(page_box(&reopened, b"CropBox"), [36.0, 324.0, 324.0, 756.0]);
    for key in [b"MediaBox".as_slice(), b"TrimBox", b"BleedBox", b"ArtBox"] {
        assert_eq!(page_box(&reopened, key), [0.0, 0.0, 612.0, 792.0]);
    }
}
