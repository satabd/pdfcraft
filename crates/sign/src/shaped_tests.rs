//! Arabic in visible signatures: the signer's name, the reason, the location and the labels are drawn
//! shaped from Type 3 fonts, one `/ActualText` span per drawn line, right-aligned when the line's
//! paragraph runs right to left. Most tests run with a stand-in for the craft-fonts Arabic face (one
//! cluster per character, half an em wide), so they run in any build; the one at the end uses the
//! real face and skips without it. The Latin-only appearances are pinned to what they were before
//! Arabic was drawn: their bytes are signed, so they must not move.

use super::*;
use crate::keys::PrivateKey;
use crate::pkcs12;
use crate::x509::Name;
use crate::{Appearance, AppearanceLabels, SignOptions, Status};
use pdfcraft_cos::Lexer;
use pdfcraft_fonts::shaped_text::{Shaper, set_test_shaper};
use pdfcraft_fonts::{GlyphError, GlyphOutline, ShapedCluster, ShapedScript, helvetica_width};

// ---- the stand-in face ----

fn fake_shape(script: ShapedScript, text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    if script != ShapedScript::Arabic {
        return Err(GlyphError::Missing);
    }
    let mut out: Vec<ShapedCluster> = text
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let advance = if matches!(u32::from(c), 0x064B..=0x065F) {
                0.0
            } else if c == '\u{0600}' {
                // Every one of these is a different shape: a hostile text can need thousands.
                0.5 + i as f64 * 1e-4
            } else {
                0.5
            };
            ShapedCluster { script, glyphs: vec![(u32::from(c), [0.0, 0.0])], advance, text: c.to_string() }
        })
        .collect();
    if rtl {
        out.reverse();
    }
    Ok(out)
}

fn fake_has(script: ShapedScript, c: char) -> bool {
    script == ShapedScript::Arabic && (c == ' ' || c.is_ascii_punctuation() || pdfcraft_fonts::shaped_text::is_arabic(c))
}

fn fake_outline(_: ShapedScript, id: u32) -> Result<GlyphOutline, GlyphError> {
    // One letter whose outline is too complex.
    if id == 0x06E5 {
        return Err(GlyphError::TooComplex);
    }
    Ok(GlyphOutline { contours: vec![vec![[0.0, 0.0], [0.5, 0.0], [0.5, 0.5], [0.0, 0.5]]], width: 0.5, bbox: [0.0, 0.0, 0.5, 0.5] })
}

const FAKE: Shaper = Shaper { shape: fake_shape, has: fake_has, outline: fake_outline, arabic_available: || true };

fn no_shape(_: ShapedScript, _: &str, _: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    Err(GlyphError::NoFont)
}

fn no_outline(_: ShapedScript, _: u32) -> Result<GlyphOutline, GlyphError> {
    Err(GlyphError::NoFont)
}

/// A build with no Arabic face.
const NO_FACE: Shaper = Shaper { shape: no_shape, has: |_, _| false, outline: no_outline, arabic_available: || false };

/// Uses a shaper on this thread until dropped.
struct Using;

impl Using {
    fn shaper(shaper: Shaper) -> Using {
        set_test_shaper(Some(shaper));
        Using
    }
}

impl Drop for Using {
    fn drop(&mut self) {
        set_test_shaper(None);
    }
}

// ---- the document and the signers ----

/// One page with text and an unsigned signature field "Approval".
fn fixture_bytes() -> Vec<u8> {
    let objs: Vec<&[u8]> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] >> >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R] >>",
        b"<< /Length 44 >>\nstream\nBT /F1 14 Tf 20 250 Td (Contract text) Tj ET\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Approval) /Rect [150 20 280 70] /P 3 0 R /F 4 >>",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn fixture() -> Document {
    Document::open(Arc::new(fixture_bytes())).unwrap()
}

/// The test identity with a Latin name.
fn latin_id() -> DigitalId {
    pkcs12::open(include_bytes!("../tests/data/ec-p256.p12"), "test").unwrap()
}

/// A self-signed identity whose common name is `name`.
fn id_named(name: &str) -> DigitalId {
    let key = PrivateKey::generate_p256().unwrap();
    let dn = Name::build(name, "", "", "", "");
    let cert = Certificate::self_signed(&dn, &key, Time::from_unix(1_790_000_000), 5, &[1, 2, 3, 4]).unwrap();
    DigitalId { key, certificate: cert, chain: Vec::new(), friendly_name: Some(name.to_string()) }
}

/// 200 x 50: the name's half is 100 wide, the details' half starts at 103 and ends at 197.
const RECT: [f64; 4] = [20.0, 100.0, 220.0, 150.0];

fn opts() -> SignOptions {
    SignOptions { page: 0, rect: Some(RECT), date: "D:20261002120000+01'00'".into(), ..SignOptions::default() }
}

fn with_reason(reason: &str) -> SignOptions {
    SignOptions {
        reason: Some(reason.into()),
        location: Some("London".into()),
        appearance: Appearance { reason: true, location: true, ..Appearance::default() },
        ..opts()
    }
}

fn arabic_labels() -> AppearanceLabels {
    AppearanceLabels {
        signed_by: "وقّع رقمياً بواسطة".into(),
        distinguished_name: "الهوية: ".into(),
        reason: "السبب: ".into(),
        location: "الموقع: ".into(),
        date: "التاريخ: ".into(),
    }
}

// ---- reading an appearance back ----

/// A content stream as (operator, operands), read with the document lexer.
fn parse_content(bytes: &[u8]) -> Vec<(String, Vec<Object>)> {
    let mut lexer = Lexer::new(bytes, 0);
    let mut out = Vec::new();
    let mut operands = Vec::new();
    loop {
        lexer.skip_ws();
        let Some(b) = bytes.get(lexer.pos).copied() else { break };
        if matches!(b, b'/' | b'(' | b'<' | b'[' | b'0'..=b'9' | b'-' | b'+' | b'.') {
            operands.push(lexer.object().expect("a content operand"));
        } else {
            let op = String::from_utf8_lossy(lexer.token()).into_owned();
            assert!(!op.is_empty(), "stuck at {} in {}", lexer.pos, String::from_utf8_lossy(bytes));
            out.push((op, std::mem::take(&mut operands)));
        }
    }
    out
}

/// One drawn line: where its `Tm` put it, the typing-order text of its span (none for plain text),
/// its drawn width, the Latin strings shown in Helvetica, the fonts used and the text of the
/// generated fonts' glyphs.
#[derive(Debug, Default)]
struct Row {
    x: f64,
    y: f64,
    size: f64,
    span: Option<String>,
    width: f64,
    latin: Vec<String>,
    fonts: Vec<String>,
    glyphs: String,
}

/// The normal appearance of the signature field `name`: its stream, decoded content and the document.
fn appearance_stream(doc: &Document, field: &str) -> Stream {
    let catalog = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    let form = doc.resolve(catalog.get(b"AcroForm").unwrap()).as_dict().cloned().unwrap();
    let fields = doc.resolve(form.get(b"Fields").unwrap()).as_array().cloned().unwrap();
    let field = fields
        .iter()
        .filter_map(|f| doc.resolve(f).as_dict().cloned())
        .find(|f| matches!(f.get(b"T"), Some(Object::String(s)) if s.bytes == field.as_bytes()))
        .unwrap();
    let ap = doc.resolve(field.get(b"AP").unwrap()).as_dict().cloned().unwrap();
    let Object::Stream(s) = &*doc.resolve(ap.get(b"N").unwrap()) else { panic!("no normal appearance") };
    s.clone()
}

/// Code -> text of a generated Type 3 font's `ToUnicode` map (`<01> <0627>` lines).
fn to_unicode_of(doc: &Document, font: &Dict) -> HashMap<u8, String> {
    let mut map = HashMap::new();
    let Some(r) = font.get(b"ToUnicode") else { return map };
    let Object::Stream(s) = &*doc.resolve(r) else { return map };
    let text = String::from_utf8_lossy(&s.decoded().unwrap()).into_owned();
    for line in text.lines() {
        let hex: Vec<&str> = line.split(['<', '>']).map(str::trim).filter(|p| !p.is_empty()).collect();
        if let [code, unicode] = hex[..]
            && code.len() == 2
            && unicode.len() >= 4
            && let Ok(code) = u8::from_str_radix(code, 16)
        {
            let units: Vec<u16> = (0..unicode.len() / 4).filter_map(|i| u16::from_str_radix(unicode.get(i * 4..i * 4 + 4)?, 16).ok()).collect();
            map.insert(code, String::from_utf16_lossy(&units));
        }
    }
    map
}

fn rows_of(doc: &Document, stream: &Stream) -> Vec<Row> {
    let res = doc.resolve(stream.dict.get(b"Resources").unwrap());
    let fonts = doc.resolve(res.as_dict().unwrap().get(b"Font").unwrap()).as_dict().cloned().unwrap();
    let mut rows: Vec<Row> = Vec::new();
    let (mut font, mut size) = (String::new(), 0.0);
    for (op, operands) in parse_content(&stream.decoded().unwrap()) {
        match op.as_str() {
            "Tm" => {
                let n = |i: usize| operands.get(i).and_then(Object::as_f64).unwrap();
                rows.push(Row { x: n(4), y: n(5), ..Row::default() });
            }
            "Tf" => {
                font = String::from_utf8_lossy(operands[0].as_name().unwrap()).into_owned();
                size = operands[1].as_f64().unwrap();
            }
            "BDC" => {
                let props = operands[1].as_dict().expect("inline properties");
                let bytes = &props.get(b"ActualText").unwrap().as_string().unwrap().bytes;
                assert_eq!(&bytes[..2], [0xFE, 0xFF]);
                let units: Vec<u16> = bytes[2..].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                rows.last_mut().expect("a Tm before the span").span = Some(String::from_utf16(&units).unwrap());
            }
            "Tj" => {
                let row = rows.last_mut().expect("a Tm before the text");
                let bytes = &operands[0].as_string().unwrap().bytes;
                let font_obj = doc.resolve(fonts.get(font.as_bytes()).unwrap_or_else(|| panic!("{font} is a resource")));
                let fd = font_obj.as_dict().unwrap();
                row.fonts.push(font.clone());
                row.size = size;
                if fd.name(b"Subtype") == Some(b"Type3") {
                    let first = fd.get(b"FirstChar").and_then(Object::as_int).unwrap();
                    let widths = fd.get(b"Widths").and_then(Object::as_array).unwrap();
                    let to_unicode = to_unicode_of(doc, fd);
                    for b in bytes {
                        let w = widths.get(usize::from(*b) - first as usize).and_then(Object::as_f64).unwrap();
                        row.width += w / 1000.0 * size;
                        row.glyphs.push_str(to_unicode.get(b).map_or("", String::as_str));
                    }
                } else {
                    let text: String = bytes.iter().map(|b| char::from(*b)).collect();
                    row.width += helvetica_width(&text, size);
                    row.latin.push(text);
                }
            }
            _ => {}
        }
    }
    rows
}

fn rows_in(doc: &Document, field: &str) -> Vec<Row> {
    rows_of(doc, &appearance_stream(doc, field))
}

fn generated_fonts(doc: &Document, stream: &Stream) -> Vec<String> {
    let res = doc.resolve(stream.dict.get(b"Resources").unwrap());
    let fonts = doc.resolve(res.as_dict().unwrap().get(b"Font").unwrap()).as_dict().cloned().unwrap();
    fonts.iter().map(|(k, _)| String::from_utf8_lossy(k).into_owned()).filter(|k| k.starts_with("PCSgAr")).collect()
}

fn signed(id: &DigitalId, options: &SignOptions) -> Vec<u8> {
    sign(&fixture(), id, options).unwrap()
}

fn reopen(bytes: &[u8]) -> Document {
    Document::open(Arc::new(bytes.to_vec())).unwrap()
}

// ---- the Latin-only appearances do not move ----

/// SHA-256 of an appearance's decoded content and its resources, as hex.
fn fingerprint(doc: &Document, stream: &Stream) -> String {
    let mut bytes = stream.decoded().unwrap();
    let mut res = Vec::new();
    let resources = doc.resolve(stream.dict.get(b"Resources").unwrap());
    pdfcraft_cos::serialize(&resources, &mut res);
    bytes.extend_from_slice(&res);
    DigestAlg::Sha256.digest(&[&bytes]).iter().map(|b| format!("{b:02x}")).collect()
}

fn built(name: &str, options: &SignOptions) -> (Document, Stream) {
    let mut doc = fixture();
    let id = latin_id();
    let stream = appearance(&mut doc, RECT, name, &id.certificate, options).unwrap();
    (doc, stream)
}

fn latin_scenarios() -> Vec<(&'static str, String, SignOptions)> {
    let strokes = crate::Graphic::Strokes(vec![vec![[0.0, 0.0], [0.3, 0.2], [0.6, 0.0], [1.0, 0.2]], vec![[0.5, 0.1]]]);
    let german = AppearanceLabels {
        signed_by: "Digital signiert von".into(),
        distinguished_name: "DN: ".into(),
        reason: "Grund: ".into(),
        location: "Ort: ".into(),
        date: "Datum: ".into(),
    };
    let every = Appearance { reason: true, location: true, distinguished_name: true, ..Appearance::default() };
    let long = "I approve this document after reading every one of its clauses twice and talking them over with counsel";
    vec![
        ("default", "Ada Lovelace".into(), opts()),
        ("reason and location", "Ada Lovelace".into(), with_reason("I approve this document")),
        ("everything", "Ada Lovelace".into(), SignOptions { appearance: every, ..with_reason("Approved") }),
        ("no labels", "Ada Lovelace".into(), SignOptions { appearance: Appearance { labels: false, ..Appearance::default() }, ..opts() }),
        ("a graphic", "Ada Lovelace".into(), SignOptions { graphic: Some(strokes), ..with_reason("Approved") }),
        ("german labels", "Jos\u{e9} M\u{fc}ller".into(), SignOptions { appearance_labels: german, ..with_reason("Genehmigt") }),
        ("a long reason", "Ada Lovelace".into(), with_reason(long)),
        ("a long name", "Maximiliana Wilhelmina von Hohenzollern-Sigmaringen".into(), opts()),
        ("parentheses", "Ada (the first) \\ Lovelace".into(), with_reason("a (b) c")),
        ("the date only", "Ada Lovelace".into(), SignOptions { appearance: Appearance { name: false, ..Appearance::default() }, ..opts() }),
    ]
}

/// Taken from the code before Arabic was drawn.
const PINS: [(&str, &str); 10] = [
    ("default", "dcf2b288c6b727f161b4892a4d2d0b2332c57cdbf0dab47fbf8c5e7c54386bc5"),
    ("reason and location", "f3d8d448c9f1cb70db0f7537342417006d7fc1cdb42bc4fb1bf36e3f15d73719"),
    ("everything", "162d2ac6a623901e77304e3261a3a88ba6f0b09bb7e95d9b59009695bb7085ed"),
    ("no labels", "4a4e64e8c83185e029b72176668123837d088b2324b71f561ffa763f948a591f"),
    ("a graphic", "08c1eedd31ff4444c6647851d118d5684ef6818ef3457d48bee1f2568f98717a"),
    ("german labels", "163f2855aa94c261770fd2845fd707a0dd35a4fb9fcdd1eb23cb08a03a17753e"),
    ("a long reason", "06b455e7731e98842bfbeaea12656add2e2863ecd9f20e9b1a1d2e1396afa4f4"),
    ("a long name", "128a60dd088099e48c71e4a4b561de4d62a4d8e05453088523b3584a41196206"),
    ("parentheses", "a584df19470fc994757c8206e8e3198b7c60eeb75a9f22c9d8d210c01af7ca27"),
    ("the date only", "694ae7b92b2fd988877621c33f2759d98486d4e920b14603eaada2c2952e4309"),
];

#[test]
fn latin_appearances_are_byte_for_byte_what_they_were() {
    let mut mismatches = Vec::new();
    for (what, name, options) in latin_scenarios() {
        // Whatever shaper is in use: Latin text never reaches it.
        for shaper in [None, Some(FAKE), Some(NO_FACE)] {
            set_test_shaper(shaper);
            let (doc, stream) = built(&name, &options);
            let got = fingerprint(&doc, &stream);
            let want = PINS.iter().find(|(w, _)| *w == what).map(|(_, f)| *f).unwrap();
            if got != want {
                mismatches.push(format!("(\"{what}\", \"{got}\"),"));
            }
            assert!(generated_fonts(&doc, &stream).is_empty(), "{what}: no Arabic font for Latin text");
        }
    }
    set_test_shaper(None);
    mismatches.dedup();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

// ---- Arabic is drawn ----

const NAME: &str = "أحمد محمد";
const REASON: &str = "موافقة";

#[test]
fn an_arabic_name_and_reason_are_drawn_shaped_in_spans_and_right_aligned() {
    let _face = Using::shaper(FAKE);
    let id = id_named(NAME);
    // English labels: the reason's line starts with Latin, so it runs left to right.
    let bytes = signed(&id, &with_reason(REASON));
    let doc = reopen(&bytes);
    let stream = appearance_stream(&doc, "Signature1");
    let content = String::from_utf8_lossy(&stream.decoded().unwrap()).into_owned();
    assert!(!content.contains('?'), "no placeholder: {content}");
    let rows = rows_of(&doc, &stream);
    let spans: Vec<&str> = rows.iter().filter_map(|r| r.span.as_deref()).collect();
    assert_eq!(spans, [NAME, NAME, "Reason: موافقة"], "{rows:?}");
    // The large name is a right-to-left line in the left half (3 .. 97): it ends at the right edge.
    let large = rows.iter().find(|r| r.span.as_deref() == Some(NAME)).unwrap();
    assert!((large.x + large.width - 97.0).abs() < 0.05, "{large:?}");
    assert!(large.size > 12.0, "the name is large: {large:?}");
    // The same name among the details (103 .. 197), smaller, and also against the right edge.
    let small = rows.iter().filter(|r| r.span.as_deref() == Some(NAME)).nth(1).unwrap();
    assert!((small.x + small.width - 197.0).abs() < 0.05 && small.size < large.size, "{small:?}");
    // The reason follows an English label: left to right, from the left edge of the details.
    let reason = rows.iter().find(|r| r.span.as_deref() == Some("Reason: موافقة")).unwrap();
    assert!((reason.x - 103.0).abs() < 0.01, "{reason:?}");
    assert!(reason.latin.iter().any(|s| s.starts_with("Reason")), "{reason:?}");
    assert_eq!(reason.glyphs.chars().filter(|c| REASON.contains(*c)).count(), REASON.chars().count(), "{reason:?}");
    // The Latin lines are plain Helvetica text.
    let plain: Vec<&Row> = rows.iter().filter(|r| r.span.is_none()).collect();
    assert!(plain.iter().any(|r| r.latin.iter().any(|s| s.starts_with("Digitally"))), "{plain:?}");
    assert!(plain.iter().any(|r| r.latin.iter().any(|s| s.starts_with("Location"))), "{plain:?}");
    // The generated fonts are in the appearance's resources.
    assert!(!generated_fonts(&doc, &stream).is_empty());
}

#[test]
fn arabic_labels_make_every_line_right_to_left_and_right_aligned() {
    let _face = Using::shaper(FAKE);
    let id = id_named(NAME);
    let options = SignOptions { appearance_labels: arabic_labels(), ..with_reason(REASON) };
    let bytes = signed(&id, &options);
    let doc = reopen(&bytes);
    let rows = rows_in(&doc, "Signature1");
    let spans: Vec<&str> = rows.iter().filter_map(|r| r.span.as_deref()).collect();
    // (The date wraps after its time.)
    for want in ["وقّع رقمياً بواسطة", "السبب: موافقة", "الموقع: London", "التاريخ: 2026.10.02"] {
        assert!(spans.iter().any(|s| s.starts_with(want)), "{want}: {spans:?}");
    }
    // Lines of the details end at the details' right edge (197), however long they are.
    let details: Vec<&Row> = rows.iter().filter(|r| r.span.is_some() && r.x > 100.0).collect();
    assert!(details.len() >= 4, "{rows:?}");
    for r in &details {
        assert!((r.x + r.width - 197.0).abs() < 0.05, "{r:?}");
    }
    // Top to bottom, inside the 50-point box.
    assert!(details.windows(2).all(|w| w[0].y > w[1].y) && details.iter().all(|r| r.y > 0.0 && r.y < 50.0), "{details:?}");
    // Nothing is a question mark.
    assert!(!String::from_utf8_lossy(&appearance_stream(&doc, "Signature1").decoded().unwrap()).contains('?'));
}

#[test]
fn latin_and_digits_next_to_arabic_stay_in_place() {
    let _face = Using::shaper(FAKE);
    let typed = "Dr. أحمد 2026";
    let id = id_named(typed);
    let bytes = signed(&id, &opts());
    let doc = reopen(&bytes);
    let rows = rows_in(&doc, "Signature1");
    let row = rows.iter().find(|r| r.span.as_deref() == Some(typed)).unwrap_or_else(|| panic!("{rows:?}"));
    // A left-to-right line: from the left edge, "Dr." first, the digits a Latin run, the name shaped.
    assert!(row.latin.first().is_some_and(|s| s.starts_with("Dr.")), "{row:?}");
    assert!(row.latin.iter().any(|s| s.contains("2026")), "{row:?}");
    assert!(row.glyphs.contains("أحمد") || row.glyphs.chars().any(is_arabic_letter), "{row:?}");
    assert!((row.x - 3.0).abs() < 0.01 || (row.x - 103.0).abs() < 0.01, "left aligned: {row:?}");
    assert!(!String::from_utf8_lossy(&appearance_stream(&doc, "Signature1").decoded().unwrap()).contains('?'));
    // The same in the reason, beside an Arabic label (a right-to-left line with Latin in it).
    let options = SignOptions { appearance_labels: arabic_labels(), ..with_reason("رقم 42 (Report)") };
    let bytes = signed(&latin_id(), &options);
    let rows = rows_in(&reopen(&bytes), "Signature1");
    let row = rows.iter().find(|r| r.span.as_deref() == Some("السبب: رقم 42 (Report)")).unwrap_or_else(|| panic!("{rows:?}"));
    assert!(row.latin.iter().any(|s| s == "42") && row.latin.iter().any(|s| s.contains("Report")), "{row:?}");
    assert!((row.x + row.width - 197.0).abs() < 0.05, "{row:?}");
}

fn is_arabic_letter(c: char) -> bool {
    pdfcraft_fonts::shaped_text::is_arabic_script(c)
}

#[test]
fn a_long_arabic_reason_wraps_on_shaped_widths_into_lines_that_fit() {
    let _face = Using::shaper(FAKE);
    let reason = "موافقة على هذا المستند بعد قراءة جميع بنوده بعناية";
    let id = latin_id();
    let options = SignOptions { appearance_labels: arabic_labels(), ..with_reason(reason) };
    let bytes = signed(&id, &options);
    let rows = rows_in(&reopen(&bytes), "Signature1");
    let pieces: Vec<&Row> =
        rows.iter().filter(|r| r.span.as_deref().is_some_and(|s| s.contains("موافقة") || s.contains("بعناية") || s.contains("قراءة"))).collect();
    assert!(pieces.len() >= 2, "the reason wraps: {rows:?}");
    for r in rows.iter().filter(|r| r.x > 100.0) {
        assert!(r.x >= 103.0 - 0.01 && r.x + r.width <= 197.01, "inside the details' column: {r:?}");
    }
    // Every word is in some line, in typing order.
    let joined: String = pieces.iter().filter_map(|r| r.span.as_deref()).collect::<Vec<_>>().join(" ");
    for word in reason.split(' ') {
        assert!(joined.contains(word), "{word}: {joined}");
    }
}

#[test]
fn drawn_signatures_are_deterministic() {
    let _face = Using::shaper(FAKE);
    let id = id_named(NAME);
    let options = SignOptions { appearance_labels: arabic_labels(), ..with_reason(REASON) };
    assert_eq!(signed(&id, &options), signed(&id, &options), "the signed bytes depend on nothing random");
    let (a, b) = (built_arabic(&options), built_arabic(&options));
    assert_eq!(a, b);
}

fn built_arabic(options: &SignOptions) -> String {
    let mut doc = fixture();
    let id = latin_id();
    let stream = appearance(&mut doc, RECT, NAME, &id.certificate, options).unwrap();
    fingerprint(&doc, &stream)
}

// ---- they sign and validate ----

#[test]
fn a_signature_with_an_arabic_appearance_validates_from_the_saved_file() {
    let _face = Using::shaper(FAKE);
    let id = id_named(NAME);
    let options = SignOptions { appearance_labels: arabic_labels(), ..with_reason(REASON) };
    let bytes = signed(&id, &options);
    let doc = reopen(&bytes);
    let trust = TrustStore { certs: vec![id.certificate.clone()] };
    let sigs = list(&doc, &bytes, &trust);
    let s = sigs.iter().find(|s| s.signed).expect("a signed field");
    assert_eq!(s.status, Status::Valid, "{:?}", s.details);
    assert_eq!(s.modification, Modification::None);
    assert!(s.visible);
    assert_eq!(s.signer.as_deref(), Some(NAME));
    assert_eq!(s.reason.as_deref(), Some(REASON));
    // Changing a signed byte breaks it.
    let mut tampered = bytes.clone();
    let at = tampered.windows(13).position(|w| w == b"Contract text").unwrap();
    tampered[at] = b'K';
    let s = list(&reopen(&tampered), &tampered, &trust).into_iter().find(|s| s.signed).unwrap();
    assert_eq!(s.status, Status::Invalid);
}

// ---- what can't be drawn is refused ----

#[test]
fn without_the_arabic_face_the_old_refusal_stays() {
    let _face = Using::shaper(NO_FACE);
    let id = latin_id();
    let err = sign(&fixture(), &id, &with_reason(REASON)).unwrap_err();
    let m = err.to_string();
    assert!(matches!(err, SignError::Unsupported(_)), "{err:?}");
    assert!(m.contains("\"م\" (U+0645)") && m.contains("the reason") && m.contains("sign without a visible appearance"), "{m}");
    // The signer's name, too (the name is from the certificate).
    let err = sign(&fixture(), &id_named(NAME), &opts()).unwrap_err().to_string();
    assert!(err.contains("the signer's name") && err.contains("U+0623"), "{err}");
    // And a label.
    let labels = AppearanceLabels { reason: "السبب: ".into(), ..AppearanceLabels::default() };
    assert_eq!(labels.first_undrawable(), Some(("reason", 'ا')));
    let err = sign(&fixture(), &id, &SignOptions { appearance_labels: labels, ..with_reason("ok") }).unwrap_err().to_string();
    assert!(err.contains("the reason label") && err.contains("Western European"), "{err}");
    // Invisible, or left out of the appearance: it signs, and keeps the text.
    let invisible = SignOptions { rect: None, ..with_reason(REASON) };
    let bytes = sign(&fixture(), &id, &invisible).unwrap();
    let s = list(&reopen(&bytes), &bytes, &TrustStore::default()).into_iter().find(|s| s.signed).unwrap();
    assert_eq!(s.reason.as_deref(), Some(REASON));
    sign(&fixture(), &id, &SignOptions { reason: Some(REASON.into()), ..opts() }).unwrap();
}

#[test]
fn the_labels_check_agrees_with_the_face() {
    {
        let _face = Using::shaper(FAKE);
        assert_eq!(arabic_labels().first_undrawable(), None);
        assert_eq!(AppearanceLabels::default().first_undrawable(), None);
        // Another script, or a character the face lacks, is still refused.
        let chinese = AppearanceLabels { date: "日期: ".into(), ..arabic_labels() };
        assert_eq!(chinese.first_undrawable(), Some(("date", '日')));
        let mixed = AppearanceLabels { date: "ب日".into(), ..AppearanceLabels::default() };
        assert_eq!(mixed.first_undrawable(), Some(("date", '日')));
    }
    let _face = Using::shaper(NO_FACE);
    assert_eq!(arabic_labels().first_undrawable(), Some(("signed_by", 'و')));
}

#[test]
fn text_neither_font_can_draw_is_refused_before_anything_is_added() {
    let _face = Using::shaper(FAKE);
    let id = latin_id();
    for reason in ["ب\u{06E5}ب", "ب日本", "ب\0ا", "ب\u{05D0}"] {
        let mut doc = fixture();
        let before = doc.object_numbers();
        let err = appearance(&mut doc, RECT, "Ada", &id.certificate, &with_reason(reason));
        assert!(matches!(err, Err(SignError::Unsupported(_))), "{reason:?}: {err:?}");
        assert_eq!(doc.object_numbers(), before, "{reason:?}: no font is left behind");
        assert!(sign(&fixture(), &id, &with_reason(reason)).is_err(), "{reason:?}");
    }
}

#[test]
fn hostile_text_is_an_error_or_drawn_but_never_a_panic() {
    let _face = Using::shaper(FAKE);
    let id = latin_id();
    let long: String = "مرحبا ".repeat(1700);
    assert!(long.chars().count() > 10_000);
    let many_shapes: String = "\u{0600}".repeat(5000);
    let cases: Vec<(&str, String)> = vec![
        ("lone marks", "\u{064E}\u{064E}\u{0651}".into()),
        ("controls only", "\u{202E}\u{200F}\u{061C}\u{2067}".into()),
        ("controls and letters", "\u{202E}ب\u{200F}\u{061C}ا\u{2067}".into()),
        ("NUL and tab", "ب\0ا\tب".into()),
        ("a replacement character", "ب\u{FFFD}\u{10FFFF}".into()),
        ("10,000 characters", long),
        ("an outline that is too complex", "ب\u{06E5}ب".into()),
        ("more than 4,096 shapes", many_shapes),
        ("Arabic next to CJK", "ب日本".into()),
        ("Arabic next to Hebrew", "ب\u{05D0}".into()),
        ("line breaks", "ب\nا\r\nب".into()),
        ("only spaces", "   ".into()),
    ];
    for (what, text) in cases {
        for rect in [RECT, [0.0, 0.0, 30.0, 12.0], [0.0, 0.0, 2.0, 2.0], [0.0, 0.0, 1000.0, 400.0]] {
            let options = SignOptions { appearance_labels: arabic_labels(), ..with_reason(&text) };
            for name in [text.as_str(), "Ada"] {
                let mut doc = fixture();
                let before = doc.object_numbers();
                match appearance(&mut doc, rect, name, &id.certificate, &options) {
                    Ok(stream) => assert!(stream.decoded().is_ok(), "{what}"),
                    Err(e) => {
                        assert!(matches!(e, SignError::Unsupported(_)), "{what}: {e:?}");
                        assert_eq!(doc.object_numbers(), before, "{what}: an error adds nothing");
                    }
                }
            }
        }
        // Through signing: the same, and the document handed in is never changed.
        let doc = fixture();
        let before = doc.object_numbers();
        let options = with_reason(&text);
        if let Ok(bytes) = sign(&doc, &id, &options) {
            assert!(bytes.starts_with(&fixture_bytes()), "{what}: an incremental update");
        }
        assert_eq!(doc.object_numbers(), before, "{what}");
    }
    for rect in [[f64::NAN, 0.0, 10.0, 10.0], [0.0, 0.0, f64::INFINITY, 50.0], [0.0, f64::NEG_INFINITY, 100.0, f64::INFINITY]] {
        let mut doc = fixture();
        let _ = appearance(&mut doc, rect, NAME, &id.certificate, &SignOptions { appearance_labels: arabic_labels(), ..with_reason(REASON) });
    }
}

// ---- the real face ----

#[test]
fn the_real_arabic_face_draws_a_visible_signature() {
    if pdfcraft_fonts::document_arabic_font().is_none() {
        eprintln!("set CRAFT_FONTS_DIR to exercise Arabic in visible signatures");
        return;
    }
    set_test_shaper(None);
    let id = id_named("د. أحمد محمد 2026");
    let options = SignOptions { appearance_labels: arabic_labels(), ..with_reason("موافقة على العقد (نسخة 2)") };
    let bytes = signed(&id, &options);
    let doc = reopen(&bytes);
    let stream = appearance_stream(&doc, "Signature1");
    let rows = rows_of(&doc, &stream);
    assert!(rows.iter().any(|r| r.span.as_deref() == Some("د. أحمد محمد 2026")), "{rows:?}");
    assert!(rows.iter().any(|r| r.span.as_deref() == Some("السبب: موافقة على العقد (نسخة 2)")), "{rows:?}");
    for r in rows.iter().filter(|r| r.span.is_some() && r.x > 100.0) {
        assert!((r.x + r.width - 197.0).abs() < 0.5, "{r:?}");
    }
    assert!(!generated_fonts(&doc, &stream).is_empty());
    assert!(!String::from_utf8_lossy(&stream.decoded().unwrap()).contains('?'));
    let trust = TrustStore { certs: vec![id.certificate.clone()] };
    let s = list(&doc, &bytes, &trust).into_iter().find(|s| s.signed).unwrap();
    assert_eq!((s.status, s.modification), (Status::Valid, Modification::None), "{:?}", s.details);
    // Hostile input with the real face: no panic, whatever is drawn.
    for text in ["\u{064E}\u{064E}", "\u{202E}\u{200F}\u{061C}", "ب\0ا", "\u{FDFD}", &"ب".repeat(20_000)] {
        let mut doc = fixture();
        let _ = appearance(&mut doc, RECT, text, &latin_id().certificate, &with_reason(text));
    }
}
