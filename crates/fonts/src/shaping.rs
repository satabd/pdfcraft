//! Arabic, Hebrew and Thai text shaping with the matching craft-fonts face and bounded glyph outlines.

use std::sync::OnceLock;

use harfrust::{BufferClusterLevel, Direction, ShapeOptions, ShaperData, ShaperInstance, UnicodeBuffer, script};
use skrifa::instance::{Location, LocationRef, Size};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::{GlyphError, GlyphOutline};

/// One cluster of shaped text: the glyphs that show one character (a letter's body and
/// dots, say), or several characters for a ligature. Em units.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedCluster {
    /// Face that owns the glyph ids.
    pub script: ShapedScript,
    /// Glyph ids, each with its offset from the cluster's pen position (marks sit above or below).
    pub glyphs: Vec<(u32, [f64; 2])>,
    pub advance: f64,
    /// The characters it shows, in logical order.
    pub text: String,
}

/// The face owning a shaped glyph id. IDs from different faces must never share a cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapedScript {
    Arabic,
    Hebrew,
    Thai,
    /// The Arabic face at its bold instance (`wght` 700): the same glyph ids as [`Self::Arabic`], with other outlines
    /// and advances, so it is a face of its own for every cache. Only shaped when the face has a `wght` axis; without
    /// one, shaping it gives [`Self::Arabic`] clusters (see [`shape_arabic_at`]).
    ArabicBold,
}

/// The weight shaped text is drawn at. Only the Arabic face has a bold instance here (its `wght` axis at 700); the
/// other scripts are drawn at their one weight whatever this says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Weight {
    #[default]
    Regular,
    Bold,
}

impl ShapedScript {
    pub fn font(self) -> Option<&'static crate::CraftFont> {
        match self {
            Self::Arabic | Self::ArabicBold => crate::document_arabic_font(),
            Self::Hebrew => crate::document_hebrew_font(false),
            Self::Thai => crate::document_thai_font(),
        }
    }

    /// The face of this script at `weight`: Arabic's bold instance for [`Weight::Bold`], Arabic's regular one for
    /// [`Weight::Regular`]; Hebrew and Thai have one weight and stay as they are.
    pub fn weighted(self, weight: Weight) -> Self {
        match (self, weight) {
            (Self::Arabic | Self::ArabicBold, Weight::Bold) => Self::ArabicBold,
            (Self::Arabic | Self::ArabicBold, Weight::Regular) => Self::Arabic,
            (other, _) => other,
        }
    }

    /// The weight this face is drawn at.
    pub fn weight(self) -> Weight {
        if self == Self::ArabicBold { Weight::Bold } else { Weight::Regular }
    }

    /// Whether this is the Arabic face, at either weight.
    pub fn is_arabic(self) -> bool {
        matches!(self, Self::Arabic | Self::ArabicBold)
    }
}

fn face(script: ShapedScript) -> Option<&'static (FontRef<'static>, ShaperData)> {
    static FACE: OnceLock<Option<(FontRef<'static>, ShaperData)>> = OnceLock::new();
    static HEBREW: OnceLock<Option<(FontRef<'static>, ShaperData)>> = OnceLock::new();
    static THAI: OnceLock<Option<(FontRef<'static>, ShaperData)>> = OnceLock::new();
    let cache = match script {
        ShapedScript::Arabic | ShapedScript::ArabicBold => &FACE,
        ShapedScript::Hebrew => &HEBREW,
        ShapedScript::Thai => &THAI,
    };
    cache
        .get_or_init(|| {
            let font = FontRef::new(script.font()?.bytes).ok()?;
            let data = ShaperData::new(&font);
            Some((font, data))
        })
        .as_ref()
}

/// A face's bold instance: where harfrust shapes it and where skrifa reads its advances and outlines.
pub(crate) struct BoldInstance {
    shaper: ShaperInstance,
    location: Location,
}

/// `font`'s bold instance, its `wght` axis at 700 (clamped to the axis): `None` when it has no such axis, or one that
/// goes no heavier than its default, and is then drawn at its default (regular) instance.
pub(crate) fn bold_instance_of(font: &FontRef) -> Option<BoldInstance> {
    let wght = skrifa::Tag::new(b"wght");
    let axis = font.axes().iter().find(|axis| axis.tag() == wght)?;
    if axis.max_value() <= axis.default_value() {
        return None;
    }
    let setting = [("wght", 700.0f32)];
    let location = font.axes().location(setting);
    // (A location at the default on every axis is the regular instance.)
    if location.coords().iter().all(|c| *c == skrifa::instance::NormalizedCoord::default()) {
        return None;
    }
    Some(BoldInstance { shaper: ShaperInstance::from_variations(font, setting), location })
}

/// The Arabic document face's bold instance (see [`bold_instance_of`]); `None` without the face or the axis.
fn arabic_bold() -> Option<&'static BoldInstance> {
    static BOLD: OnceLock<Option<BoldInstance>> = OnceLock::new();
    BOLD.get_or_init(|| face(ShapedScript::Arabic).and_then(|(font, _)| bold_instance_of(font))).as_ref()
}

/// Whether the Arabic document face has a bold instance to draw [`Weight::Bold`] at (false without the face, or with a
/// face that has no `wght` axis: bold Arabic is then drawn regular).
pub fn arabic_bold_available() -> bool {
    arabic_bold().is_some()
}

/// Whether the Arabic document face has a glyph for `c` (false without the face).
pub fn arabic_has(c: char) -> bool {
    face(ShapedScript::Arabic).is_some_and(|(font, _)| font.charmap().map(c).is_some())
}

/// Shape one directional run of `text` with the Arabic document face. The clusters come in
/// drawing order, left to right. [`GlyphError::NoFont`] without the face;
/// [`GlyphError::Missing`] when it lacks a character.
pub fn shape_arabic(text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    shape_script(text, rtl, ShapedScript::Arabic)
}

/// [`shape_arabic`] at `weight`: bold text is shaped at the face's bold instance, and its clusters are
/// [`ShapedScript::ArabicBold`]'s. A face without a bold instance shapes it regular, and says so: the clusters are then
/// [`ShapedScript::Arabic`]'s, the same as regular text's.
pub fn shape_arabic_at(text: &str, rtl: bool, weight: Weight) -> Result<Vec<ShapedCluster>, GlyphError> {
    shape_script(text, rtl, ShapedScript::Arabic.weighted(weight))
}

/// Whether the Hebrew document face has a glyph for `c` (false without the face).
pub fn hebrew_has(c: char) -> bool {
    face(ShapedScript::Hebrew).is_some_and(|(font, _)| font.charmap().map(c).is_some())
}

/// Shape one directional run of `text` with the Hebrew document face, clusters in drawing order.
/// Points (niqqud) and cantillation marks stay in their letter's cluster, positioned by the face.
pub fn shape_hebrew(text: &str, rtl: bool) -> Result<Vec<ShapedCluster>, GlyphError> {
    shape_script(text, rtl, ShapedScript::Hebrew)
}

pub fn thai_has(c: char) -> bool {
    face(ShapedScript::Thai).is_some_and(|(font, _)| font.charmap().map(c).is_some())
}

/// Shape Thai with grapheme clusters so Sara Am decomposition and stacked marks keep their
/// original Unicode text exactly once, including when wrapped or copied from the saved PDF.
pub fn shape_thai(text: &str) -> Result<Vec<ShapedCluster>, GlyphError> {
    shape_script(text, false, ShapedScript::Thai)
}

fn shape_script(text: &str, rtl: bool, kind: ShapedScript) -> Result<Vec<ShapedCluster>, GlyphError> {
    let (font, data) = face(kind).ok_or(GlyphError::NoFont)?;
    let bold = if kind == ShapedScript::ArabicBold { arabic_bold() } else { None };
    // Without a bold instance, bold Arabic is the regular face's.
    let kind = if kind == ShapedScript::ArabicBold && bold.is_none() { ShapedScript::Arabic } else { kind };
    let shaper = data.shaper(font).instance(bold.map(|b| &b.shaper)).build();
    let scale = 1.0 / f64::from(shaper.units_per_em().max(1));
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(if rtl { Direction::RightToLeft } else { Direction::LeftToRight });
    buffer.set_script(match kind {
        ShapedScript::Arabic | ShapedScript::ArabicBold => script::ARABIC,
        ShapedScript::Hebrew => script::HEBREW,
        ShapedScript::Thai => script::THAI,
    });
    // Preserve the historical Arabic per-character mapping; Hebrew and Thai marks stay with their base.
    buffer.set_cluster_level(match kind {
        ShapedScript::Arabic | ShapedScript::ArabicBold => BufferClusterLevel::Characters,
        ShapedScript::Hebrew | ShapedScript::Thai => BufferClusterLevel::MonotoneGraphemes,
    });
    let shaped = shaper.shape(buffer, ShapeOptions::new());
    // Clusters are byte offsets into `text`; a cluster shows the characters up to the next one.
    let mut starts: Vec<usize> = shaped.glyph_infos().iter().map(|g| g.cluster as usize).collect();
    starts.sort_unstable();
    starts.dedup();
    let mut out: Vec<(usize, ShapedCluster)> = Vec::new();
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        if info.glyph_id == 0 {
            return Err(GlyphError::Missing);
        }
        let start = info.cluster as usize;
        // A cluster's glyphs are adjacent in the shaped run.
        let cluster = match out.last_mut() {
            Some((s, c)) if *s == start => c,
            _ => {
                let end = starts.get(starts.partition_point(|s| *s <= start)).copied().unwrap_or(text.len());
                let text = text.get(start..end).unwrap_or_default().to_string();
                out.push((start, ShapedCluster { script: kind, glyphs: Vec::new(), advance: 0.0, text }));
                // Just pushed.
                let Some((_, c)) = out.last_mut() else { continue };
                c
            }
        };
        let at = [cluster.advance + f64::from(pos.x_offset) * scale, f64::from(pos.y_offset) * scale];
        cluster.glyphs.push((info.glyph_id, at));
        cluster.advance += f64::from(pos.x_advance) * scale;
    }
    Ok(out.into_iter().map(|(_, c)| c).collect())
}

/// Glyph `id` of the Arabic document face, bounded like [`crate::japanese_glyph`]. Glyphs without
/// outlines (spaces) come back empty.
pub fn arabic_glyph(id: u32) -> Result<GlyphOutline, GlyphError> {
    shaped_glyph(ShapedScript::Arabic, id)
}

/// A bounded outline from the same face that produced the shaped glyph id, at the same instance
/// ([`ShapedScript::ArabicBold`]'s at the bold one, when the face has it).
pub fn shaped_glyph(script: ShapedScript, id: u32) -> Result<GlyphOutline, GlyphError> {
    let (font, _) = face(script).ok_or(GlyphError::NoFont)?;
    if font.maxp().map_or(true, |m| id >= u32::from(m.num_glyphs())) {
        return Err(GlyphError::Missing);
    }
    let gid = GlyphId::new(id);
    let bold = if script == ShapedScript::ArabicBold { arabic_bold() } else { None };
    let loc = bold.map_or_else(LocationRef::default, |b| LocationRef::from(&b.location));
    let scale = 1.0 / font.metrics(Size::unscaled(), loc).units_per_em.max(1) as f64;
    let width = font.glyph_metrics(Size::unscaled(), loc).advance_width(gid).unwrap_or(0.0) as f64 * scale;
    // Marks have no advance; the widest ligatures (U+FDFD) span a few em.
    if !width.is_finite() || !(0.0..=8.0).contains(&width) {
        return Err(GlyphError::Missing);
    }
    if font.outline_glyphs().get(gid).is_none() {
        return Ok(GlyphOutline { contours: Vec::new(), width, bbox: [0.0, 0.0, width, 0.0] });
    }
    crate::script::bounded_outline_at(font, gid, width, loc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face_or_skip() -> bool {
        if crate::document_arabic_font().is_none() {
            eprintln!("skipping the Arabic shaping checks: built without a craft-fonts Arab face (set CRAFT_FONTS_DIR)");
            assert_eq!(shape_arabic("ب", true), Err(GlyphError::NoFont));
            assert_eq!(arabic_glyph(1).map(|_| ()), Err(GlyphError::NoFont));
            assert!(!arabic_has('ب'));
            return false;
        }
        true
    }

    #[test]
    fn letters_take_their_joining_forms() {
        if !face_or_skip() {
            return;
        }
        let ids =
            |s: &str| shape_arabic(s, true).unwrap().iter().flat_map(|c| c.glyphs.iter().map(|g| g.0)).collect::<std::collections::BTreeSet<_>>();
        let (isolated, joined) = (ids("ب"), ids("ببب"));
        // Initial, medial and final forms are glyphs the isolated letter doesn't use.
        assert!(joined.difference(&isolated).count() >= 2, "{isolated:?} {joined:?}");
        // One cluster per letter, however many glyphs the face draws it with (Noto Sans Arabic:
        // body and dots).
        let c = shape_arabic("ببب", true).unwrap();
        assert!(c.len() == 3 && c.iter().all(|c| c.text == "ب" && c.advance > 0.0 && !c.glyphs.is_empty()), "{c:?}");
    }

    #[test]
    fn right_to_left_runs_are_drawn_from_their_last_letter() {
        if !face_or_skip() {
            return;
        }
        let g = shape_arabic("سلام", true).unwrap();
        let drawn: String = g.iter().map(|g| g.text.as_str()).collect();
        assert_eq!(drawn.chars().rev().collect::<String>(), "سلام");
        assert!(shape_arabic("سلام", false).unwrap().iter().map(|g| g.text.as_str()).collect::<String>() == "سلام");
    }

    #[test]
    fn marks_keep_their_own_text_and_no_advance() {
        if !face_or_skip() {
            return;
        }
        let c = shape_arabic("بَ", true).unwrap();
        assert_eq!(c.len(), 2, "{c:?}");
        let mark = c.iter().find(|c| c.text == "\u{064E}").unwrap();
        assert_eq!(mark.advance, 0.0);
        let outline = arabic_glyph(mark.glyphs[0].0).unwrap();
        assert!(!outline.contours.is_empty() && outline.bbox[1] > 0.0, "the fatha sits above the baseline");
    }

    #[test]
    fn spaces_and_missing_characters() {
        if !face_or_skip() {
            return;
        }
        let space = shape_arabic(" ", true).unwrap();
        assert!(space[0].advance > 0.0);
        assert!(arabic_glyph(space[0].glyphs[0].0).unwrap().contours.is_empty());
        assert!(arabic_has('ب') && !arabic_has('日'));
        assert_eq!(shape_arabic("日", true), Err(GlyphError::Missing));
        assert_eq!(arabic_glyph(u32::MAX).map(|_| ()), Err(GlyphError::Missing));
        assert!(shape_arabic("", true).unwrap().is_empty());
    }
}

#[cfg(test)]
mod thai_tests {
    use super::*;

    #[test]
    fn thai_clusters_preserve_sara_am_and_stacked_marks() {
        if crate::document_thai_font().is_none() {
            assert_eq!(shape_thai("น้ำ"), Err(GlyphError::NoFont));
            assert!(!thai_has('ก'));
            return;
        }
        for text in ["น้ำ", "กิ่", "ผู้ใช้", "ปู่", "เก้า", "สวัสดีภาษาไทย", "๑๒๓", ""]
        {
            let clusters = shape_thai(text).unwrap();
            assert_eq!(clusters.iter().map(|c| c.text.as_str()).collect::<String>(), text);
            for c in clusters {
                assert_eq!(c.script, ShapedScript::Thai);
                assert!(c.advance.is_finite() && c.advance >= 0.0);
                for (id, pos) in c.glyphs {
                    assert!(pos.iter().all(|v| v.is_finite()));
                    shaped_glyph(c.script, id).unwrap();
                }
            }
        }
        let stacked = shape_thai("กิ่").unwrap();
        assert_eq!(stacked.len(), 1, "base, vowel and tone must wrap together");
        assert!(stacked[0].glyphs.len() >= 3);
        assert!(stacked[0].glyphs.iter().any(|(_, p)| p[0] != 0.0 || p[1] != 0.0));
        assert_eq!(shape_thai("日"), Err(GlyphError::Missing));
        assert_eq!(shaped_glyph(ShapedScript::Thai, u32::MAX).map(|_| ()), Err(GlyphError::Missing));
    }
}

#[cfg(test)]
mod hebrew_tests {
    use super::*;

    #[test]
    fn hebrew_runs_are_drawn_from_their_last_letter_with_points_on_their_letter() {
        if crate::document_hebrew_font(false).is_none() {
            eprintln!("skipping the Hebrew shaping checks: built without a craft-fonts Hebr face (set CRAFT_FONTS_DIR)");
            assert_eq!(shape_hebrew("ש", true), Err(GlyphError::NoFont));
            assert!(!hebrew_has('ש'));
            return;
        }
        let g = shape_hebrew("שלום", true).unwrap();
        assert!(g.iter().all(|c| c.script == ShapedScript::Hebrew && c.advance > 0.0 && !c.glyphs.is_empty()), "{g:?}");
        let drawn: String = g.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(drawn.chars().rev().collect::<String>(), "שלום");
        // A point (qamats) is one cluster with its letter, so copy and search get it once.
        let pointed = shape_hebrew("שָׁ", true).unwrap();
        assert_eq!(pointed.len(), 1, "{pointed:?}");
        assert_eq!(pointed[0].text, "שָׁ");
        assert!(pointed[0].glyphs.len() >= 2);
        for c in shape_hebrew("שָׁלוֹם 123", true).unwrap() {
            for (id, pos) in c.glyphs {
                assert!(pos.iter().all(|v| v.is_finite()));
                shaped_glyph(c.script, id).unwrap();
            }
        }
        assert!(hebrew_has('א') && hebrew_has('\u{05B8}') && !hebrew_has('日'));
        assert_eq!(shape_hebrew("日", true), Err(GlyphError::Missing));
        assert!(shape_hebrew("", true).unwrap().is_empty());
    }
}

#[cfg(test)]
mod weight_tests {
    use super::*;

    /// Dancing Script, the typed-signature face: variable, with a `wght` axis from 400 to 700.
    static DANCING: &[u8] = include_bytes!("../../../assets/fonts/DancingScript.ttf");

    #[test]
    fn a_face_without_a_wght_axis_has_no_bold_instance_and_one_with_it_has() {
        // Inter, a static face, and DejaVu Sans (static too, where the system has it): no `wght` axis, so bold text in
        // such a face is shaped and drawn at its one (regular) instance.
        let inter = FontRef::new(crate::INTER_REGULAR).expect("Inter parses");
        assert!(bold_instance_of(&inter).is_none());
        match std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf") {
            Ok(bytes) => assert!(bold_instance_of(&FontRef::new(&bytes).expect("DejaVu parses")).is_none()),
            Err(_) => eprintln!("no DejaVu Sans on this system: the static-face check ran on Inter alone"),
        }
        // A face with the axis: its bold location is away from the default, and its outlines there are other ones.
        let dancing = FontRef::new(DANCING).expect("Dancing Script parses");
        let bold = bold_instance_of(&dancing).expect("a wght axis");
        assert!(bold.location.coords().iter().any(|c| *c != skrifa::instance::NormalizedCoord::default()));
        assert!(!bold.shaper.coords().is_empty());
        let gid = dancing.charmap().map('a').expect("an a");
        let regular = crate::script::bounded_outline(&dancing, gid, 0.5).expect("an outline");
        let heavier = crate::script::bounded_outline_at(&dancing, gid, 0.5, LocationRef::from(&bold.location)).expect("an outline");
        assert_ne!(regular.contours, heavier.contours);
    }

    #[test]
    fn bold_arabic_is_shaped_wider_from_the_bold_instance() {
        if crate::document_arabic_font().is_none() {
            eprintln!("skipping the bold Arabic checks: built without a craft-fonts Arab face (set CRAFT_FONTS_DIR)");
            assert_eq!(shape_arabic_at("ب", true, Weight::Bold), Err(GlyphError::NoFont));
            assert!(!arabic_bold_available());
            return;
        }
        // Noto Sans Arabic is variable, `wght` 100 to 900.
        assert!(arabic_bold_available());
        let word = "بالعالم";
        let regular = shape_arabic_at(word, true, Weight::Regular).expect("shapes");
        let bold = shape_arabic_at(word, true, Weight::Bold).expect("shapes");
        // Regular is what it always was.
        assert_eq!(regular, shape_arabic(word, true).expect("shapes"));
        assert!(regular.iter().all(|c| c.script == ShapedScript::Arabic) && bold.iter().all(|c| c.script == ShapedScript::ArabicBold));
        let text = |clusters: &[ShapedCluster]| clusters.iter().map(|c| c.text.clone()).collect::<String>();
        assert_eq!(text(&bold), text(&regular));
        let width = |clusters: &[ShapedCluster]| clusters.iter().map(|c| c.advance).sum::<f64>();
        assert!(width(&bold) > width(&regular) * 1.02, "bold {} against regular {}", width(&bold), width(&regular));
        // Each glyph is drawn from the bold outlines, its advance read at the same instance.
        let mut differ = 0;
        for c in &bold {
            for (id, _) in &c.glyphs {
                let (heavy, light) =
                    (shaped_glyph(ShapedScript::ArabicBold, *id).expect("bold"), shaped_glyph(ShapedScript::Arabic, *id).expect("regular"));
                if heavy.contours != light.contours {
                    differ += 1;
                }
            }
        }
        assert!(differ > 0, "{bold:?}");
        assert_eq!(shaped_glyph(ShapedScript::ArabicBold, u32::MAX).map(|_| ()), Err(GlyphError::Missing));
    }
}
