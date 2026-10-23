# Editing existing text and source subsets

Click a text region in **Edit a PDF → Edit text and images**. One editor contains its
source rows and mixed styles. Embedded TrueType and OpenType/CFF faces are used in memory;
source font programs, colour, size and baseline anchors stay intact. A textless preview
prevents a second copy underneath. The frame shows the live extent as text grows or shrinks.
Rows a text edit doesn't reach keep their original operators, so their kerning, `TJ` word
gaps and glyph widths stay exactly as they were. Only rows whose characters changed, or that
moved because an earlier row wrapped, are rewritten, and formatting a selection rewrites
only the rows it covers. Changing the whole paragraph's formatting, width or position rewrites
every row.

Add Text also opens existing text when it is clicked; a blank-space click starts a new item.
Both text tools retype an added text item with one click. Draft boxes grow to the page edge
before wrapping, and the saved item uses that width. A single visual source row grows across font/style boundaries;
multi-row paragraphs retain their existing wrap width. Retyping uses a preview with only
that added item's old drawing removed, so deleted draft characters do not show underneath.

A subset font contains only some of the original face's glyphs. A new character cannot be
reconstructed from a glyph the PDF does not contain. **Load font…** imports an openly licensed,
embeddable standalone `.ttf` or `.otf` face. **Installed fonts…** discovers eligible user-installed
faces on request, off the UI thread; the document-font menu selects resources already on the
page. Select **Keep source fonts** to preserve each run's original face. A matching full face
completes its subsets automatically, including new Chinese characters. When the PDF itself embeds
the complete program of a paragraph's font (not a subset) and its licence allows editable
embedding, a character the font's encoding can't show is drawn with that same program. A
character no available face has is refused with an error that names it. Choosing another face
is explicit. Fonts with restricted embedding, unsupported collections, excluded families or
no open-licence declaration are refused. Interface fallback fonts are never exported.

The full selected program, glyph mappings, widths and Unicode text are embedded in the PDF.
Saving and reopening therefore needs no locally installed font. Editing the same text again
reuses the font resource the page already has, and every resource made from one program
shares a single copy of it. Original subset bytes are
preserved. Optional matching faces from `CRAFT_FONTS_DIR` can also complete source subsets;
this works only when that build input actually contains the full face. This change does not
add a font binary or change the release font pin. Fonts shared by releases belong in
`storytold/craft-fonts` under the repository's asset rules.

Rewrapped Chinese and Japanese text, which has no spaces between words, may break between any
two characters. A row never begins with closing punctuation, a closing bracket or quote, a small
kana or the prolonged sound mark, and never ends with an opening bracket or quote or a currency
sign before an amount: the line-start and line-end rules of JIS X 4051, as in Unicode line
breaking (UAX #14). Latin words inside such text are not split. Korean and Thai still break only
at spaces.

Select text within the editor before changing size, colour, font, underline or spacing to
format that selection. Automation supports up to 1024 disjoint UTF-8 byte ranges with the
same properties; invalid or overlapping ranges fail atomically. Untouched source formatting
survives. OpenType shaping uses the selected/source face's substitution and positioning
rules, preserves marks and ligatures, and applies Unicode bidi ordering for a single face.
Logical text remains available after save/reopen through ToUnicode and ActualText.
Added Arabic text records ActualText per shaped line for copying and searching in typing order.
ActualText already supplies typing order, so the editor preserves it without applying
a second bidi reversal; adjacent visual glyph runs still recover their own reading order.
Only right-to-left ActualText is kept apart this way: left-to-right spans, such as the Latin
ligatures (`fi`, `fl`) Word and Chrome wrap in ActualText mid-word, join their line as before.

Arabic text typed into Edit text is shaped when the source font can't shape it. A complete
embedded face that covers the text still shapes it in place (above). Otherwise (a subset font,
no embedded font, a face without the letters, or a paragraph an earlier edit already replaced)
the craft-fonts Arabic face (Noto Sans Arabic) draws it: joined forms, ligatures and marks, in
UAX #9 order, as Type 3 glyphs named `PCEdAr…` (`/Name /PCArabic`) with a ToUnicode map, up to
240 glyphs per font. Every wrapped line is one `/Span` with `/ActualText` holding the line in
typing order, so copy, search and a later edit read it as typed. Latin letters and digits
between the Arabic are drawn with the paragraph's own font when it has them, else Helvetica.
The page font's reverse ToUnicode map is never used for Arabic letters: a producer maps a
letter's initial, medial and final glyphs to one code, so re-encoding would write isolated
forms. `replace_line` (the automation `line` form) shapes the same way. A build without the
Arabic face says so (`CRAFT_FONTS_DIR`) and changes nothing. Not done yet: Hebrew and Arabic in
one paragraph, formatting part of the Arabic text or choosing a document font for it
(refused), character spacing (ignored, since it would pull joined letters apart), at most 4,096
distinct letter shapes per edit, and embedding the face as a real font.

A paragraph of several looks (runs that differ in size, fill colour or font, such as a larger
first row or a bold red word) keeps them when it is retyped in Arabic and drawn in the craft-fonts
face. Each character of the draft takes the look of the stored character it came from, and an
inserted one its neighbour's (`TextBlock::edited_spans`, as the styled rewrite and the Edit text
box have it); a stretch of right-to-left script, a word, takes the look of its first letter, so a
look never changes inside a joined word (`text_looks`, the rule the editor's `shaper_styles`
draws by). Each line is drawn as runs of one look: the shaped glyphs at the run's own size and in
its fill colour, the Latin text among them in the run's own font where that has the characters
(else the standard font of its family and weight). Each word is measured in its own look when the
paragraph wraps, a one-row paragraph of several runs grows and keeps its right edge as a single
line does, and Justify widens every gap of a line by the same distance, a `TJ` adjustment in the
size of the run it follows. Every line is still one `/Span` with `/ActualText` in typing order,
and the ToUnicode maps give the letters back. A paragraph in one look is written byte for byte
as before. Still different: the Arabic face has one weight, so the Arabic letters of a bold run
are drawn regular (their size and colour kept); a line written in several looks reads back as one
run (its `/ActualText` span) in the look of its leftmost run, so editing it again starts from one
look; the rows keep the paragraph's first line spacing; past 256 looks in one paragraph the rest
take the first.

A justified paragraph stretches every row the wrap breaks to the paragraph's width by widening the
spaces between its words, on every route. The last row of a paragraph, a row before a line break
(`\n`: each line of the draft is a paragraph of its own) and a row of one word are not stretched,
nor is a row that begins or ends with a space (those spaces are not between words). Word spacing
(`Tw`) widens only single-byte code 32, so where the font shows a space with another code (a
composite font, the generated Type 3 fallback, a subset with no space glyph) each space is followed
by a `TJ` adjustment instead; the styled rewrite (several looks, or a draft with a line break) sets
`Tw` for a simple font and uses the adjustments for the others, glyph by glyph where it shapes in the
page's own face. Shaped Arabic in generated Type 3 glyphs is always stretched by `TJ` adjustments
(its codes are not 32), the planner (`pdfcraft_fonts::shaped_text::ShapedPlan::line_ops_stretched`)
placing one after every space cluster and after every space of the Latin text between the Arabic,
which is shown word by word. A right-to-left row keeps its right edge: a stretched one ends at the
paragraph's right edge, one that is not stretched is right-aligned. `/ActualText` and ToUnicode
state the text as typed, whatever the spacing. `text_edit_layout` (`row_rects`, automation
`row_boxes`) reports the stretched rows, which the Edit text box shows. Not done: justifying
between the characters of Chinese or Japanese text (a row with no ASCII space has nothing to
widen), and a row that begins or ends with a space.

For headless editing, use `text_font_files` → `text_font_import` → `text_fonts` and pass a
resource to `text_edit.font_resource`. `text_edit.ranges` holds selected range formatting.
`text_edit_layout` accepts resource/ranges too and measures a draft without applying it.
All file inputs respect CLI root confinement; importing a font and applying text are undoable.

On a scanned page, recognised text (Scan & OCR) is an invisible layer over the picture. Edit
text groups it into phrases: words joined by a word space, with a table cell, a tab stop and each
scanned line kept apart. Applying an edit draws the new text visibly in the scan's ink colour (or
the one chosen) and first covers the old words with the paper colour sampled around them. Each
word keeps its position and width, and a longer phrase grows rightwards rather than wrapping. The
cover is one flat colour, and the text uses the layer's Helvetica, not the scanned typeface.
This applies to any invisible text (rendering mode 3), not just a PdfCraft OCR layer: editing
invisible text, such as another OCR tool's layer, makes the rewritten paragraph visible over a
painted cover in the sampled paper colour. Edit text also groups invisible text differently: its lines never
merge into multi-line paragraphs, and words split at gaps of 0.6 em or more.

Explicit marked paragraph/heading scopes prevent unrelated tagged regions from merging.
Compatible geometry groups untagged content, including runs split across content streams
or equivalent graphics transforms. Geometry cannot infer author intent in every untagged
PDF. Semantic structure-tree preservation, legacy Type 1/Type 3 inline font preview,
font collections/variable-font instances, mixed-font/style contextual runs and complete rotated
layout fidelity still need broader coverage. Do not interpret the passing real-file checks
as a guarantee for every PDF.

Private acceptance tests are opt-in and keep inputs unmodified. They exercise actual Chinese
keyboard insertion/deletion on all four pages, source-face/baseline checks and identical
in-memory renders before and after saving/reopening. Synthetic tests cover full TrueType
mapping, OpenType/CFF, Arabic marks/ligatures/mixed bidi, repeated edits, disjoint styles,
invalid ranges, font selection and root confinement. CLI acceptance repeats insertion/deletion
and two saves/reopens on four pages; independent PDFium checks extraction and rendering.
No private document, font or screenshot
is included in the repository.
