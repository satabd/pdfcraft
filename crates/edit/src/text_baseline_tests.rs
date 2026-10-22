//! Edit text writes the same bytes for a text that is not justified as it did before justified text was stretched on every
//! route: one fingerprint over the content streams of drafts in the single-style rewrite (Latin, Hebrew), the styled rewrite
//! (looks, line breaks, a composite font) and shaped Arabic, in every alignment but Justify. The value was taken from the
//! code before the change; a deliberate change to what these routes write re-pins it.

use pdfcraft_cos::{Dict, Document, Object, Stream};

use crate::added::Align;
use crate::tests::{arabic_ligature_page, hebrew_page, page_content_bytes, text_page};
use crate::text::{self, BlockStyle};
use crate::text_shaped_tests::fake;

/// 64-bit FNV-1a.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

fn styles() -> Vec<BlockStyle> {
    let aligned = |align| BlockStyle { align: Some(align), ..Default::default() };
    vec![
        BlockStyle::default(),
        aligned(Align::Left),
        aligned(Align::Center),
        aligned(Align::Right),
        BlockStyle { width: Some(90.0), ..Default::default() },
        BlockStyle { width: Some(120.0), align: Some(Align::Right), underline: Some(true), ..Default::default() },
        BlockStyle { size: Some(14.0), char_spacing: Some(0.5), line_spacing: Some(1.4), ..Default::default() },
        BlockStyle { scale: Some(90.0), color: Some([0.1, 0.2, 0.3]), offset: Some([5.0, -4.0]), ..Default::default() },
    ]
}

/// The content of `doc` after `draft` is written in each style (an error is part of the fingerprint too).
fn written(out: &mut Vec<u8>, page: &dyn Fn() -> Document, drafts: &[&str]) {
    for draft in drafts {
        for style in styles() {
            let mut doc = page();
            let result = text::rewrite_block(&mut doc, 0, 0, Some(draft), &style);
            out.extend(format!("\n--- {draft:?} {style:?} {:?}\n", result.map(|r| r.substituted.is_some()).map_err(|e| e.to_string())).bytes());
            out.extend(page_content_bytes(&doc, 0));
        }
    }
}

fn composite_page() -> Document {
    let hex = |s: &str| s.bytes().map(|b| format!("00{b:02x}")).collect::<String>();
    let src = format!(
        "BT /F1 10 Tf 12 TL 100 700 Td <{}> Tj 0 0 1 rg <{}> Tj 0 g T* <{}> Tj T* <{}> Tj ET",
        hex("One two three four five six"),
        hex(" seven"),
        hex("eight nine ten eleven twelve"),
        hex("and then the end of it")
    );
    let mut doc = text_page(&src);
    let unicode = doc.add(Object::Stream(Stream::flate(
        Dict::new(),
        b"1 begincodespacerange <0000> <ffff> endcodespacerange 1 beginbfrange <0020> <007e> <0020> endbfrange",
    )));
    let font = Dict::from_iter([
        (b"Type".to_vec(), Object::name("Font")),
        (b"Subtype".to_vec(), Object::name("Type0")),
        (b"BaseFont".to_vec(), Object::name("SyntheticCID")),
        (b"Encoding".to_vec(), Object::name("Identity-H")),
        (b"ToUnicode".to_vec(), Object::Ref(unicode)),
        (
            b"DescendantFonts".to_vec(),
            Object::Array(vec![Object::Dict(Dict::from_iter([
                (b"Subtype".to_vec(), Object::name("CIDFontType2")),
                (b"DW".to_vec(), Object::Int(500)),
            ]))]),
        ),
    ]);
    let p = pdfcraft_model::pages(&doc)[0].obj;
    let resources = Dict::from_iter([(b"Font".to_vec(), Object::Dict(Dict::from_iter([(b"F1".to_vec(), Object::Dict(font))])))]);
    doc.update_dict(p, |d| d.set(b"Resources".to_vec(), Object::Dict(resources))).unwrap();
    doc
}

/// Over the 66,385 bytes of the 80 drafts' content streams, taken from the code before justified text was stretched on every route.
const FINGERPRINT: u64 = 0x1ad6_60cd_e322_69d2;

const LATIN: &str = "One two three four five six seven eight nine ten eleven twelve and then the end of it all and more words";

#[test]
fn text_that_is_not_justified_is_written_as_it_always_was() {
    let _face = fake();
    let mut out = Vec::new();
    // The single-style rewrite: three rows of one look, a short and a long draft, and one with a line break.
    let latin =
        || text_page("BT /F1 10 Tf 12 TL 100 700 Td (One two three four five six seven) Tj T* (eight nine ten eleven twelve) Tj T* (end) Tj ET");
    written(&mut out, &latin, &["One two", LATIN, "Short first\nthen a longer second paragraph with more words to wrap around"]);
    // The styled rewrite: two looks in the paragraph, and a composite font.
    let looks = || {
        text_page(
            "BT /F1 10 Tf 12 TL 100 700 Td (One two three four five six) Tj 0 0 1 rg ( seven) Tj 0 g T* (eight nine ten eleven twelve) Tj T* (end) Tj ET",
        )
    };
    written(&mut out, &looks, &[LATIN, "Short first\nthen a longer second paragraph with more words to wrap around"]);
    written(&mut out, &composite_page, &[LATIN]);
    // Hebrew, in the single-style rewrite, growing to the left from its right edge.
    let hebrew = || hebrew_page("BT /F1 12 Tf 300 700 Td (EDCBA) Tj ET");
    written(&mut out, &hebrew, &["שלום בדיוק", &["שלום בדיוק"; 8].join(" ")]);
    // Shaped Arabic (the stand-in face), in generated glyphs, with a line break and Latin words among it.
    let arabic = || arabic_ligature_page("BT /F1 12 Tf 300 700 Td (BAB) Tj ET", false);
    let words = "مرحبا بالعالم مرحبا بالعالم مرحبا بالعالم مرحبا بالعالم";
    written(&mut out, &arabic, &[words, "مرحبا\nمرحبا بالعالم مرحبا hello world بالعالم مرحبا بالعالم"]);
    // Every draft was written (a refusal would be part of the bytes, but a baseline of refusals proves nothing).
    let text = String::from_utf8_lossy(&out);
    assert_eq!(text.matches("\n--- ").count(), 80);
    assert_eq!(text.matches(" Ok(").count(), 80);
    assert_eq!(fnv1a(&out), FINGERPRINT, "the bytes written for text that is not justified changed: {:#018x} over {} bytes", fnv1a(&out), out.len());
}
