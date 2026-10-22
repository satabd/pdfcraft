//! The page beneath an inline text editor, with the source glyphs removed on a COS clone.

use std::sync::Arc;

use pdfcraft_render::{PageRenderer, RenderConfig, RenderPool, RenderRequest, RenderedPage};

use crate::{Document, guard};

pub struct TextEditPreview {
    page: usize,
    bytes: Arc<Vec<u8>>,
    config: RenderConfig,
}

impl TextEditPreview {
    pub fn renderer(&self) -> RenderPool {
        RenderPool::new(self.bytes.clone(), 1, self.config.clone())
    }

    pub fn render_background(&self, scale: f32) -> Result<RenderedPage, String> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err("preview scale must be finite and positive".into());
        }
        guard(|| {
            let mut renderer = PageRenderer::new(self.bytes.clone(), self.config.clone());
            let out = renderer.render(RenderRequest { page: self.page, scale, ..Default::default() });
            if let Some(e) = &out.error {
                return Err(e.clone());
            }
            Ok(out)
        })?
    }
}

/// Draft measuring for one paragraph, on a copy of the document (see [`Document::text_draft_probe`]).
#[derive(Clone)]
pub struct TextDraftProbe {
    cos: pdfcraft_cos::Document,
    page: usize,
    block: usize,
}

impl TextDraftProbe {
    /// The rows the rewrite that applies `text` in `style` lays the paragraph out in (what Apply will write), or `None` if
    /// it refuses the draft.
    pub fn rows(&self, text: &str, style: &pdfcraft_edit::BlockStyle) -> Option<Vec<String>> {
        guard(|| pdfcraft_edit::text::text_edit_layout(&self.cos, self.page, self.block, text, style).ok().map(|layout| layout.rows)).ok().flatten()
    }
}

impl Document {
    /// Background for retyping an added text item, without changing its saved content.
    pub fn added_text_preview(&self, page: usize, index: usize) -> Result<TextEditPreview, String> {
        guard(|| {
            if !self.added().iter().filter(|a| a.page == page).nth(index).is_some_and(|a| matches!(a.content, crate::AddedContent::Text(_))) {
                return Err("the selected added item is not text".into());
            }
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            let mut cos = editor.cos.clone();
            pdfcraft_edit::delete_content(&mut cos, page, index).map_err(|e| e.to_string())?;
            let bytes = Arc::new(pdfcraft_cos::write_incremental(&cos, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?);
            Ok(TextEditPreview { page, bytes, config: self.config.clone() })
        })?
    }

    /// Measure a draft without changing bytes, dirty state, or undo history.
    pub fn text_edit_layout(
        &self,
        page: usize,
        block: usize,
        text: &str,
        style: &pdfcraft_edit::BlockStyle,
    ) -> Result<pdfcraft_edit::text::TextEditLayout, String> {
        guard(|| {
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            pdfcraft_edit::text::text_edit_layout(&editor.cos, page, block, text, style).map_err(|e| e.to_string())
        })?
    }

    /// An owned, read-only copy of the document for measuring drafts of paragraph `block` of `page` where a borrow of the
    /// document cannot go (a closure an editor keeps for the frame). Nothing in it changes the document.
    pub fn text_draft_probe(&self, page: usize, block: usize) -> Option<TextDraftProbe> {
        self.editor.as_ref().map(|editor| TextDraftProbe { cos: editor.cos.clone(), page, block })
    }

    /// Embedded source faces for a text editor, bounded and validated as sfnt programs.
    /// This is read-only; no source font is installed or exported.
    pub fn text_editor_fonts(&self, page: usize) -> Vec<pdfcraft_edit::text::TextEditorFont> {
        self.editor.as_ref().and_then(|e| pdfcraft_edit::text::text_editor_fonts(&e.cos, page).ok()).unwrap_or_default()
    }

    /// Preparing the background never changes working bytes, dirty state or undo history.
    pub fn text_edit_preview(&self, page: usize, block: usize) -> Result<TextEditPreview, String> {
        guard(|| {
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            let mut cos = editor.cos.clone();
            pdfcraft_edit::clear_block(&mut cos, page, block).map_err(|e| e.to_string())?;
            let bytes = Arc::new(pdfcraft_cos::write_incremental(&cos, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?);
            Ok(TextEditPreview { page, bytes, config: self.config.clone() })
        })?
    }

    /// The page beneath an added item while it is dragged: `index` counts the page's added
    /// items, as `Edit::DeleteContent` does. Read-only, like [`Self::text_edit_preview`].
    pub fn added_content_preview(&self, page: usize, index: usize) -> Result<TextEditPreview, String> {
        guard(|| {
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            let mut cos = editor.cos.clone();
            pdfcraft_edit::delete_content(&mut cos, page, index).map_err(|e| e.to_string())?;
            let bytes = Arc::new(pdfcraft_cos::write_incremental(&cos, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?);
            Ok(TextEditPreview { page, bytes, config: self.config.clone() })
        })?
    }
}
