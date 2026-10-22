# pdfcraft-export

Export a PDF to Word (.docx), PowerPoint (.pptx), HTML and RTF from its paragraphs, tables and
images. Layer L4; depends on `flate2` (the package writer) and `pdfcraft-fonts` (the paragraph
direction only). The engine reduces each page to `Page` (text blocks with their boxes, sizes and
colours, and images); the writers turn that into a flowing document, or into positioned shapes
for pages with rotated or overlaid text.

The writers are XML, HTML and RTF text writers: no font is drawn or embedded, so Arabic and Hebrew
are written as they are (UTF-8, `\uN` escapes in RTF) and nothing is refused or written as `?`.
A paragraph, heading or table cell whose first strong character runs right to left (UAX #9) is
marked as such where the format has a way to: `<w:bidi/>` and, in its runs, `<w:rtl/>` with the
complex-script size and weight in Word; `<a:pPr algn="r" rtl="1"/>` in PowerPoint; `dir="rtl"` in
HTML; `\rtlpar\qr` with `\afs`, `\ab` and `\ai` in RTF. A whole table reads right to left when more
of its cells open with a right-to-left letter than with a left-to-right one (a tie, or no letters,
stays left to right; `table_is_rtl`): `<w:bidiVisual/>` in `w:tblPr` (Word mirrors the columns, the
cells stay in logical order), `rtl="1"` on `<a:tblPr>`, `dir="rtl"` on `<table>`, `\rtlrow` in `\trowd`.
Not done: text a PDF stores in visual order is exported as extracted.
