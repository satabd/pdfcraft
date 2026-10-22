//! The About window's credits: every contributor (GitHub login always; display and real names only
//! with recorded consent) and the AI models that helped. The tables are baked into the binary by
//! build.rs from contributors/contributors.json (craftrules standards/contributors.md); nothing is
//! read at run time.

use std::cmp::Ordering;

use egui::{Align, Layout, RichText, Sense, TextWrapMode, Ui, UiBuilder, WidgetText, vec2};

use crate::rtl;
use crate::theme::Tokens;

/// The About dialog is a modal that sizes to its content; the lists scroll inside this height.
const LIST_HEIGHT: f32 = 380.0;

/// English lookup keys for the model table, also checked by catalog coverage tests.
pub const MODEL_COLUMNS: [&str; 6] = ["Company", "Model", "Version", "Commits", "% of all commits", "Lines +/−"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contributor {
    pub login: &'static str,
    pub display_name: Option<&'static str>,
    pub real_name: Option<&'static str>,
    pub prs: u64,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub binary_added: u64,
    pub binary_deleted: u64,
    /// ISO 8601 UTC; empty for someone credited only through a merged PR.
    pub first_commit: &'static str,
    pub last_commit: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub company: &'static str,
    pub model: &'static str,
    pub version: &'static str,
    pub commits: u64,
    pub lines_added: u64,
    pub lines_deleted: u64,
}

include!(concat!(env!("OUT_DIR"), "/credits.rs"));

/// Which name to show. Display and real names fall back to the `@username` when not given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameMode {
    #[default]
    Username,
    DisplayName,
    RealName,
}

impl NameMode {
    pub const ALL: [NameMode; 3] = [NameMode::Username, NameMode::DisplayName, NameMode::RealName];

    pub fn label(self) -> &'static str {
        match self {
            NameMode::Username => "Username",
            NameMode::DisplayName => "Display name",
            NameMode::RealName => "Real name",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Prs,
    Commits,
    LinesAdded,
    LinesDeleted,
    LinesDelta,
    BinaryAdded,
    BinaryDeleted,
    #[default]
    FirstCommit,
    LastCommit,
}

impl SortKey {
    pub const ALL: [SortKey; 10] = [
        SortKey::Name,
        SortKey::Prs,
        SortKey::Commits,
        SortKey::LinesAdded,
        SortKey::LinesDeleted,
        SortKey::LinesDelta,
        SortKey::BinaryAdded,
        SortKey::BinaryDeleted,
        SortKey::FirstCommit,
        SortKey::LastCommit,
    ];

    /// Menu label and table header.
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            SortKey::Name => ("Name (A–Z)", "Name"),
            SortKey::Prs => ("Merged PRs", "PRs"),
            SortKey::Commits => ("Commits", "Commits"),
            SortKey::LinesAdded => ("Lines added", "+LOC"),
            SortKey::LinesDeleted => ("Lines deleted", "−LOC"),
            SortKey::LinesDelta => ("Line delta", "ΔLOC"),
            SortKey::BinaryAdded => ("Binary assets added", "+Bin"),
            SortKey::BinaryDeleted => ("Binary assets removed", "−Bin"),
            SortKey::FirstCommit => ("First commit", "First"),
            SortKey::LastCommit => ("Last commit", "Last"),
        }
    }

    /// Names and dates read naturally oldest/A first; counts read biggest first.
    pub fn default_ascending(self) -> bool {
        matches!(self, SortKey::Name | SortKey::FirstCommit | SortKey::LastCommit)
    }
}

impl Contributor {
    pub fn name(&self, mode: NameMode) -> String {
        let chosen = match mode {
            NameMode::Username => None,
            NameMode::DisplayName => self.display_name,
            NameMode::RealName => self.real_name,
        };
        chosen.map_or_else(|| format!("@{}", self.login), str::to_string)
    }

    pub fn lines_delta(&self) -> i128 {
        i128::from(self.lines_added) - i128::from(self.lines_deleted)
    }

    /// One line with everything we know, for tooltips.
    pub fn summary(&self) -> String {
        let prs = group(self.prs);
        let commits = group(self.commits);
        let added = group(self.lines_added);
        let deleted = group(self.lines_deleted);
        let delta = signed(self.lines_delta());
        let bin_added = self.binary_added.to_string();
        let bin_deleted = self.binary_deleted.to_string();
        crate::i18n::fmt(
            crate::i18n::t(
                "@{login}: {prs} PRs, {commits} commits, +{added} / −{deleted} lines (Δ {delta}), +{bin_added} / −{bin_deleted} binary assets, {first} – {last}",
            ),
            &[
                ("login", self.login),
                ("prs", &prs),
                ("commits", &commits),
                ("added", &added),
                ("deleted", &deleted),
                ("delta", &delta),
                ("bin_added", &bin_added),
                ("bin_deleted", &bin_deleted),
                ("first", day(self.first_commit)),
                ("last", day(self.last_commit)),
            ],
        )
    }
}

/// Alphabetical key: case-insensitive, ignoring the `@` of usernames.
fn name_key(name: &str) -> String {
    name.trim_start_matches('@').to_lowercase()
}

fn compare(a: &Contributor, b: &Contributor, mode: NameMode, key: SortKey) -> Ordering {
    match key {
        SortKey::Name => name_key(&a.name(mode)).cmp(&name_key(&b.name(mode))),
        SortKey::Prs => a.prs.cmp(&b.prs),
        SortKey::Commits => a.commits.cmp(&b.commits),
        SortKey::LinesAdded => a.lines_added.cmp(&b.lines_added),
        SortKey::LinesDeleted => a.lines_deleted.cmp(&b.lines_deleted),
        SortKey::LinesDelta => a.lines_delta().cmp(&b.lines_delta()),
        SortKey::BinaryAdded => a.binary_added.cmp(&b.binary_added),
        SortKey::BinaryDeleted => a.binary_deleted.cmp(&b.binary_deleted),
        // A PR-only contributor (no commit date) sorts after everyone with a date.
        SortKey::FirstCommit => dated(a.first_commit).cmp(&dated(b.first_commit)),
        SortKey::LastCommit => dated(a.last_commit).cmp(&dated(b.last_commit)),
    }
}

fn dated(d: &str) -> (bool, &str) {
    (d.is_empty(), d)
}

/// Contributors in display order. Ties fall back to the name (A–Z) so the order is stable.
pub fn sorted(list: &[Contributor], mode: NameMode, key: SortKey, ascending: bool) -> Vec<&Contributor> {
    let mut v: Vec<&Contributor> = list.iter().collect();
    v.sort_by(|a, b| {
        let o = compare(a, b, mode, key);
        let o = if ascending { o } else { o.reverse() };
        o.then_with(|| compare(a, b, mode, SortKey::Name)).then_with(|| a.login.cmp(b.login))
    });
    v
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn signed(n: i128) -> String {
    let g = group(u64::try_from(n.unsigned_abs()).unwrap_or(u64::MAX));
    if n < 0 { format!("−{g}") } else { format!("+{g}") }
}

fn day(iso: &str) -> &str {
    if iso.is_empty() { "—" } else { iso.get(..10).unwrap_or(iso) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    names: NameMode,
    key: SortKey,
    ascending: bool,
    table: bool,
}

impl Default for View {
    fn default() -> Self {
        View { names: NameMode::Username, key: SortKey::FirstCommit, ascending: true, table: false }
    }
}

/// About ▸ Contributors: a name toggle, a sort, and the list as a grab bag or a table.
pub fn contributors_ui(ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let id = egui::Id::new("credits_view");
    let mut v = ui.data_mut(|d| d.get_temp::<View>(id)).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(crate::i18n::t("Show"));
        for m in NameMode::ALL {
            if ui.selectable_label(v.names == m, crate::i18n::t(m.label())).clicked() {
                v.names = m;
            }
        }
        ui.separator();
        ui.label(crate::i18n::t("Sort"));
        egui::ComboBox::from_id_salt("credits_sort").selected_text(crate::i18n::t(v.key.label().0)).show_ui(ui, |ui| {
            for k in SortKey::ALL {
                if ui.selectable_label(v.key == k, crate::i18n::t(k.label().0)).clicked() {
                    v.key = k;
                    v.ascending = k.default_ascending();
                }
            }
        });
        if ui.button(if v.ascending { "▲" } else { "▼" }).on_hover_text(crate::i18n::t("Reverse the order")).clicked() {
            v.ascending = !v.ascending;
        }
        ui.separator();
        if ui.selectable_label(!v.table, crate::i18n::t("Grab bag")).clicked() {
            v.table = false;
        }
        if ui.selectable_label(v.table, crate::i18n::t("Table")).clicked() {
            v.table = true;
        }
    });
    let list = sorted(CONTRIBUTORS, v.names, v.key, v.ascending);
    let counts = crate::i18n::fmt(crate::i18n::t("{n} contributors · {c} commits"), &[("n", &list.len().to_string()), ("c", &group(TOTAL_COMMITS))]);
    ui.label(RichText::new(counts).small().color(t.text_muted));
    ui.separator();
    // A mirrored table is as wide as this at least, and its name column takes what the rest leaves.
    let view_width = ui.available_width();
    // A table wider than the window starts at its right edge, where a mirrored table begins.
    let area = egui::ScrollArea::both().id_salt("credits_list").max_height(LIST_HEIGHT).auto_shrink([false, true]);
    crate::widgets::scroll_area_with(ui, area.stick_to_right(rtl::mirrored()), true, |area, ui| {
        area.show(ui, |ui| {
            if list.is_empty() {
                ui.label(crate::i18n::t("No contributor data was built into this copy."));
            } else if v.table {
                table(ui, &list, &mut v, view_width);
            } else {
                ui.horizontal_wrapped(|ui| {
                    for (i, c) in list.iter().enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("·").color(t.text_faint));
                        }
                        ui.hyperlink_to(c.name(v.names), format!("https://github.com/{}", c.login)).on_hover_text(c.summary());
                    }
                });
            }
        })
    });
    ui.data_mut(|d| d.insert_temp(id, v));
}

/// The text of the columns after the name in a contributor's table row.
fn figures(c: &Contributor) -> [String; 9] {
    [
        group(c.prs),
        group(c.commits),
        group(c.lines_added),
        group(c.lines_deleted),
        signed(c.lines_delta()),
        group(c.binary_added),
        group(c.binary_deleted),
        day(c.first_commit).to_string(),
        day(c.last_commit).to_string(),
    ]
}

/// A sort header: its text with the arrow of the current sort.
fn header_text(k: SortKey, v: &View) -> String {
    let arrow = if v.key == k { if v.ascending { " ▲" } else { " ▼" } } else { "" };
    format!("{}{arrow}", crate::i18n::t(k.label().1))
}

/// A header was clicked: sort by it, or reverse the sort when it already is the key.
fn sort_by(v: &mut View, k: SortKey) {
    if v.key == k {
        v.ascending = !v.ascending;
    } else {
        v.key = k;
        v.ascending = k.default_ascending();
    }
}

fn table(ui: &mut egui::Ui, list: &[&Contributor], v: &mut View, view_width: f32) {
    if rtl::mirrored() {
        let heads: Vec<Head> = SortKey::ALL
            .iter()
            .map(|k| Head { text: header_text(*k, v), tip: Some(crate::i18n::t(k.label().0).to_string()), button: true })
            .collect();
        let rows: Vec<Vec<Cell>> = list
            .iter()
            .map(|c| {
                let mut row = vec![Cell::Link { text: c.name(v.names), url: format!("https://github.com/{}", c.login), tip: c.summary() }];
                row.extend(figures(c).into_iter().map(Cell::Text));
                row
            })
            .collect();
        if let Some(column) = mirrored_table(ui, "credits_table", &heads, &rows, view_width)
            && let Some(k) = SortKey::ALL.get(column)
        {
            sort_by(v, *k);
        }
        return;
    }
    egui::Grid::new("credits_table").striped(true).num_columns(SortKey::ALL.len()).show(ui, |ui| {
        for k in SortKey::ALL {
            if ui.button(RichText::new(header_text(k, v)).strong()).on_hover_text(crate::i18n::t(k.label().0)).clicked() {
                sort_by(v, k);
            }
        }
        ui.end_row();
        for c in list {
            ui.hyperlink_to(c.name(v.names), format!("https://github.com/{}", c.login)).on_hover_text(c.summary());
            for text in figures(c) {
                ui.label(text);
            }
            ui.end_row();
        }
    });
}

/// The text of a model's table row.
fn model_cells(m: &Model, assisted: u64) -> [String; 6] {
    [
        m.company.to_string(),
        m.model.to_string(),
        m.version.to_string(),
        group(m.commits),
        format!("{:.0}%", 100.0 * m.commits as f64 / TOTAL_COMMITS.max(assisted) as f64),
        format!("+{} / −{}", group(m.lines_added), group(m.lines_deleted)),
    ]
}

/// About ▸ Models: AI models credited in Co-Authored-By trailers.
pub fn models_ui(ui: &mut egui::Ui) {
    if MODELS.is_empty() {
        ui.label(crate::i18n::t("No model credits were built into this copy."));
        return;
    }
    let assisted: u64 = MODELS.iter().map(|m| m.commits).max().unwrap_or(0).max(1);
    let view_width = ui.available_width();
    let area = egui::ScrollArea::both().id_salt("credits_models").max_height(LIST_HEIGHT).auto_shrink([false, true]);
    crate::widgets::scroll_area_with(ui, area.stick_to_right(rtl::mirrored()), true, |area, ui| {
        area.show(ui, |ui| {
            if rtl::mirrored() {
                let heads: Vec<Head> = MODEL_COLUMNS.iter().map(|h| Head { text: crate::i18n::t(h).to_string(), tip: None, button: false }).collect();
                let rows: Vec<Vec<Cell>> = MODELS.iter().map(|m| model_cells(m, assisted).into_iter().map(Cell::Text).collect()).collect();
                mirrored_table(ui, "credits_models", &heads, &rows, view_width);
                return;
            }
            egui::Grid::new("credits_models").striped(true).num_columns(6).show(ui, |ui| {
                for h in MODEL_COLUMNS {
                    ui.label(RichText::new(crate::i18n::t(h)).strong());
                }
                ui.end_row();
                for m in MODELS {
                    for text in model_cells(m, assisted) {
                        ui.label(text);
                    }
                    ui.end_row();
                }
            });
        })
    });
}

/// A header of a [`mirrored_table`].
struct Head {
    text: String,
    /// Shown on hover.
    tip: Option<String>,
    /// A sort button; otherwise a label.
    button: bool,
}

/// A cell of a [`mirrored_table`].
enum Cell {
    Text(String),
    Link { text: String, url: String, tip: String },
}

/// A table whose columns run from the right when the interface is mirrored: the first column is at
/// the right edge and the cells are right-aligned in their columns. `egui::Grid` places a cell at
/// the left of its column, so each cell is an exactly sized slot with a right-to-left layout in
/// it, and the columns are added in [`rtl::run_order`]. The widths come from the text itself, and
/// the first column takes whatever `view_width` has left so the striped rows span the view. Returns
/// the column of the header button that was clicked.
fn mirrored_table(ui: &mut Ui, id: &str, heads: &[Head], rows: &[Vec<Cell>], view_width: f32) -> Option<usize> {
    let measure = |text: &str, strong: bool| {
        let text = RichText::new(text);
        let text = if strong { text.strong() } else { text };
        WidgetText::from(text).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body).size()
    };
    let padding = ui.spacing().button_padding;
    let spacing = ui.spacing().item_spacing;
    let minimum = ui.spacing().interact_size.y;
    let mut widths = vec![0.0f32; heads.len()];
    let mut head_height = minimum;
    for (width, head) in widths.iter_mut().zip(heads) {
        let size = measure(&head.text, true);
        let (extra_x, extra_y) = if head.button { (2.0 * padding.x, 2.0 * padding.y) } else { (0.0, 0.0) };
        *width = size.x + extra_x;
        head_height = head_height.max(size.y + extra_y);
    }
    let mut row_height = minimum;
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            let (Cell::Text(text) | Cell::Link { text, .. }) = cell;
            let size = measure(text, false);
            *width = width.max(size.x);
            row_height = row_height.max(size.y);
        }
    }
    let between = spacing.x * heads.len().saturating_sub(1) as f32;
    let natural = widths.iter().sum::<f32>() + between;
    if let Some(first) = widths.first_mut()
        && view_width.is_finite()
        && view_width > natural
    {
        *first += view_width - natural;
    }
    // A slot of the table: `width` wide, with its content against the right.
    let slot = |ui: &mut Ui, width: f32, height: f32, add: &mut dyn FnMut(&mut Ui)| {
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let mut cell = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)));
        add(&mut cell);
    };
    let mut clicked = None;
    egui::Grid::new(id).striped(true).num_columns(heads.len()).show(ui, |ui| {
        for column in rtl::run_order(heads.len()) {
            let (Some(head), Some(width)) = (heads.get(column), widths.get(column)) else { continue };
            slot(ui, *width, head_height, &mut |ui| {
                let text = RichText::new(&head.text).strong();
                if head.button {
                    let response = ui.button(text);
                    let response = match &head.tip {
                        Some(tip) => response.on_hover_text(tip),
                        None => response,
                    };
                    if response.clicked() {
                        clicked = Some(column);
                    }
                } else {
                    ui.label(text);
                }
            });
        }
        ui.end_row();
        for row in rows {
            for column in rtl::run_order(heads.len()) {
                let (Some(cell), Some(width)) = (row.get(column), widths.get(column)) else { continue };
                slot(ui, *width, row_height, &mut |ui| match cell {
                    Cell::Text(text) => _ = ui.label(text),
                    Cell::Link { text, url, tip } => _ = ui.hyperlink_to(text, url).on_hover_text(tip),
                });
            }
            ui.end_row();
        }
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(
        login: &'static str,
        display: Option<&'static str>,
        real: Option<&'static str>,
        added: u64,
        deleted: u64,
        first: &'static str,
    ) -> Contributor {
        Contributor {
            login,
            display_name: display,
            real_name: real,
            prs: 0,
            commits: 1,
            lines_added: added,
            lines_deleted: deleted,
            binary_added: 0,
            binary_deleted: 0,
            first_commit: first,
            last_commit: first,
        }
    }

    fn people() -> [Contributor; 4] {
        [
            c("zed", None, None, 5, 50, "2026-01-02T00:00:00Z"),
            c("Bob", Some("Ann"), None, 10, 0, "2026-01-01T00:00:00Z"),
            c("alice", None, None, 7, 1, ""),
            c("carol", None, Some("Dana"), 1, 0, "2026-01-03T00:00:00Z"),
        ]
    }

    fn logins(v: &[&Contributor]) -> Vec<&'static str> {
        v.iter().map(|c| c.login).collect()
    }

    #[test]
    fn names_fall_back_to_the_username() {
        let p = c("bob", Some("Bobby"), None, 0, 0, "");
        assert_eq!(p.name(NameMode::Username), "@bob");
        assert_eq!(p.name(NameMode::DisplayName), "Bobby");
        assert_eq!(p.name(NameMode::RealName), "@bob");
    }

    #[test]
    fn alphabetical_ignores_at_and_case() {
        let all = people();
        let v = sorted(&all, NameMode::Username, SortKey::Name, true);
        assert_eq!(logins(&v), ["alice", "Bob", "carol", "zed"]);
        let v = sorted(&all, NameMode::DisplayName, SortKey::Name, true);
        assert_eq!(logins(&v), ["alice", "Bob", "carol", "zed"]); // "Ann" sorts with the @usernames
        let v = sorted(&all, NameMode::RealName, SortKey::Name, false);
        assert_eq!(logins(&v), ["zed", "carol", "Bob", "alice"]); // "Dana" between bob and zed
    }

    #[test]
    fn delta_and_dates() {
        let all = people();
        let v = sorted(&all, NameMode::Username, SortKey::LinesDelta, false);
        assert_eq!(logins(&v), ["Bob", "alice", "carol", "zed"]);
        let v = sorted(&all, NameMode::Username, SortKey::FirstCommit, true);
        assert_eq!(logins(&v), ["Bob", "zed", "carol", "alice"]); // undated last
    }

    #[test]
    fn formatting() {
        assert_eq!(group(1234567), "1,234,567");
        assert_eq!(group(12), "12");
        assert_eq!(signed(-1000), "−1,000");
        assert_eq!(day("2026-10-07T18:45:23Z"), "2026-10-07");
        assert_eq!(day(""), "—");
    }
}
