//! The Debugger page's drawing pieces: listing rows of monospace columns
//! painted into one fixed-height strip (so a code listing, a memory dump and
//! a process table share one rhythm, one set of row tones and one column
//! grid), the column header above them, tree guides, status chips and a
//! segmented switch.
//!
//! A row is one accessibility node named by its cells' text, so a screen
//! reader (and a kittest query) reads a row as the line it shows.

use egui::text::LayoutJob;
use egui::{Color32, FontId, Rect, RichText, Stroke, WidgetInfo, WidgetType};

use crate::shell::theme::{
    ACCENT_CYAN, BG_WELL, CONTROL_FILL_REST, STROKE_HAIRLINE, TEXT_CAPTION, TEXT_MUTED,
    TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TITLE,
};

/// Every listing row's height, points.
pub(crate) const ROW_HEIGHT: f32 = 20.0;
/// The gap between two columns, in character widths.
const GAP_CHARS: f32 = 2.0;
/// The room a `Fill` column keeps however narrow the pane, in characters.
const FILL_MIN_CHARS: f32 = 12.0;
/// The horizontal padding inside a row, points.
const ROW_PAD: f32 = 6.0;
/// One level of tree indent, points.
pub(crate) const TREE_INDENT: f32 = 16.0;

/// A stripe a shade above the card, for alternate rows.
const STRIPE: Color32 = Color32::from_rgb(0x1A, 0x25, 0x30);
/// A row under the pointer.
const HOVER: Color32 = Color32::from_rgb(0x21, 0x2E, 0x3B);
/// The selected row: the shell's selection teal.
const SELECTED: Color32 = Color32::from_rgb(0x1E, 0x4A, 0x4E);
/// Tree guide lines.
const GUIDE: Color32 = Color32::from_rgb(0x4E, 0x60, 0x74);

pub(crate) fn mono_font() -> FontId {
    FontId::monospace(TEXT_SECONDARY)
}

/// The width of one monospace character in listings.
pub(crate) fn char_width(ui: &egui::Ui) -> f32 {
    ui.fonts_mut(|fonts| fonts.glyph_width(&mono_font(), '0'))
}

/// How a row is shaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Plain,
    /// Every other row of a long table.
    Stripe,
    /// Where the processor stands: the instruction at RIP, the slot at RSP.
    Current,
    /// The row the page acts on (the picked process).
    Selected,
}

impl Tone {
    /// `Plain` and `Stripe` by turns.
    pub(crate) fn striped(index: usize) -> Self {
        if index % 2 == 1 {
            Self::Stripe
        } else {
            Self::Plain
        }
    }
}

/// What a row does under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowSense {
    /// Nothing: it is read.
    Look,
    /// It can be clicked; `selected` says whether it is the picked one.
    Pick { selected: bool },
}

#[derive(Clone, Debug)]
pub(crate) enum CellText {
    Plain(String, Color32),
    /// Text coloured piece by piece (a memory row's changed bytes).
    Job(LayoutJob),
    /// A horizontal bar filled by a fraction in `0..=1`, in that colour: how
    /// much of a region is resident.
    Gauge(f32, Color32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Width {
    Chars(f32),
    /// The room the fixed columns leave.
    Fill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub(crate) struct Cell {
    pub(crate) text: CellText,
    pub(crate) width: Width,
    pub(crate) align: Align,
}

impl Cell {
    pub(crate) fn left(text: impl Into<String>, color: Color32, chars: usize) -> Self {
        Self {
            text: CellText::Plain(text.into(), color),
            width: Width::Chars(chars as f32),
            align: Align::Left,
        }
    }

    pub(crate) fn right(text: impl Into<String>, color: Color32, chars: usize) -> Self {
        Self {
            text: CellText::Plain(text.into(), color),
            width: Width::Chars(chars as f32),
            align: Align::Right,
        }
    }

    pub(crate) fn fill(text: impl Into<String>, color: Color32) -> Self {
        Self {
            text: CellText::Plain(text.into(), color),
            width: Width::Fill,
            align: Align::Left,
        }
    }

    pub(crate) fn job(job: LayoutJob, chars: usize) -> Self {
        Self {
            text: CellText::Job(job),
            width: Width::Chars(chars as f32),
            align: Align::Left,
        }
    }

    /// A residency bar `chars` wide, in the rest-control fill.
    pub(crate) fn gauge(fraction: f32, chars: usize) -> Self {
        Self::gauge_colored(fraction, chars, CONTROL_FILL_REST)
    }

    /// A residency bar `chars` wide, in `color`.
    pub(crate) fn gauge_colored(fraction: f32, chars: usize, color: Color32) -> Self {
        Self {
            text: CellText::Gauge(fraction.clamp(0.0, 1.0), color),
            width: Width::Chars(chars as f32),
            align: Align::Left,
        }
    }

    fn plain_text(&self) -> &str {
        match &self.text {
            CellText::Plain(text, _) => text,
            CellText::Job(job) => &job.text,
            CellText::Gauge(..) => "",
        }
    }
}

/// Where each column of a row of `cells` starts and how wide it is, after a
/// `lead` (tree guides, a marker) of that many points, in a row `width` wide.
fn column_rects(
    row: Rect,
    lead: f32,
    cells: &[Cell],
    char_width: f32,
) -> Vec<Rect> {
    let gap = GAP_CHARS * char_width;
    let fixed: f32 = cells
        .iter()
        .map(|cell| match cell.width {
            Width::Chars(chars) => chars * char_width,
            Width::Fill => 0.0,
        })
        .sum();
    let fills = cells.iter().filter(|cell| cell.width == Width::Fill).count();
    let gaps = gap * cells.len().saturating_sub(1) as f32;
    let left_over = row.width() - 2.0 * ROW_PAD - lead - fixed - gaps;
    let fill = if fills == 0 {
        0.0
    } else {
        (left_over / fills as f32).max(FILL_MIN_CHARS * char_width)
    };
    let mut x = row.left() + ROW_PAD + lead;
    cells
        .iter()
        .map(|cell| {
            let width = match cell.width {
                Width::Chars(chars) => chars * char_width,
                Width::Fill => fill,
            };
            let rect = Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(x + width, row.bottom()));
            x += width + gap;
            rect
        })
        .collect()
}

/// The narrowest a row of `cells` can be.
fn min_width(lead: f32, cells: &[Cell], char_width: f32) -> f32 {
    let columns: f32 = cells
        .iter()
        .map(|cell| match cell.width {
            Width::Chars(chars) => chars,
            Width::Fill => FILL_MIN_CHARS,
        })
        .sum();
    let gaps = GAP_CHARS * cells.len().saturating_sub(1) as f32;
    2.0 * ROW_PAD + lead + (columns + gaps) * char_width
}

/// One listing row: `cells` in their columns after `lead` points, shaded by
/// `tone`. The row spans the room it is given, and no less than its columns
/// need, so a narrow pane scrolls instead of squeezing hex together.
pub(crate) fn list_row(
    ui: &mut egui::Ui,
    lead: f32,
    cells: &[Cell],
    tone: Tone,
    sense: RowSense,
) -> egui::Response {
    let char_width = char_width(ui);
    // Whole points: a fraction over the pane's width would raise a scroll bar.
    let width = ui
        .available_width()
        .floor()
        .max(min_width(lead, cells, char_width).ceil());
    let egui_sense = match sense {
        RowSense::Look => egui::Sense::hover(),
        RowSense::Pick { .. } => egui::Sense::click(),
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui_sense);
    let label = cells
        .iter()
        .map(Cell::plain_text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("  ");
    let enabled = ui.is_enabled();
    response.widget_info(|| match sense {
        RowSense::Look => WidgetInfo::labeled(WidgetType::Label, enabled, &label),
        RowSense::Pick { selected } => {
            WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &label)
        }
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let hovered = matches!(sense, RowSense::Pick { .. }) && response.hovered();
    let fill = match tone {
        Tone::Selected => Some(SELECTED),
        Tone::Current => Some(ACCENT_CYAN.gamma_multiply(0.14)),
        Tone::Plain | Tone::Stripe if hovered => Some(HOVER),
        Tone::Stripe => Some(STRIPE),
        Tone::Plain => None,
    };
    if let Some(fill) = fill {
        painter.rect_filled(rect, 4.0, fill);
    }
    if matches!(tone, Tone::Selected | Tone::Current) {
        painter.rect_filled(
            Rect::from_min_size(rect.left_top(), egui::vec2(2.0, rect.height())),
            1.0,
            ACCENT_CYAN,
        );
    }
    for (cell, column) in cells.iter().zip(column_rects(rect, lead, cells, char_width)) {
        if let CellText::Gauge(fraction, color) = cell.text {
            let track = Rect::from_center_size(column.center(), egui::vec2(column.width(), 5.0));
            painter.rect_filled(track, 2.0, BG_WELL);
            let mut filled = track;
            filled.set_width(track.width() * fraction);
            painter.rect_filled(filled, 2.0, color);
            continue;
        }
        let galley = match &cell.text {
            CellText::Plain(text, color) => {
                painter.layout_no_wrap(text.clone(), mono_font(), *color)
            }
            CellText::Job(job) => ui.fonts_mut(|fonts| fonts.layout_job(job.clone())),
            CellText::Gauge(..) => unreachable!("gauge painted above"),
        };
        let x = match cell.align {
            Align::Left => column.left(),
            Align::Right => column.right() - galley.size().x,
        };
        let y = column.center().y - galley.size().y / 2.0;
        painter
            .with_clip_rect(column.intersect(painter.clip_rect()))
            .galley(egui::pos2(x, y), galley, Color32::PLACEHOLDER);
    }
    response
}

/// The column titles over a table whose rows are `cells` wide.
pub(crate) fn header_row(ui: &mut egui::Ui, lead: f32, cells: &[Cell]) {
    let char_width = char_width(ui);
    let width = ui
        .available_width()
        .floor()
        .max(min_width(lead, cells, char_width).ceil());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
    let painter = ui.painter();
    for (cell, column) in cells.iter().zip(column_rects(rect, lead, cells, char_width)) {
        let galley = painter.layout_no_wrap(
            cell.plain_text().to_owned(),
            FontId::proportional(TEXT_CAPTION),
            TEXT_MUTED,
        );
        let x = match cell.align {
            Align::Left => column.left(),
            Align::Right => column.right() - galley.size().x,
        };
        painter.galley(
            egui::pos2(x, column.center().y - galley.size().y / 2.0),
            galley,
            Color32::PLACEHOLDER,
        );
    }
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0_f32, STROKE_HAIRLINE),
    );
}

/// A tree row's guide lines in its lead: a vertical line on for each
/// ancestor level that has a later sibling (`guides`), and the elbow into
/// this row at `depth`, open below unless it is its parent's `last` child.
pub(crate) fn tree_guides(ui: &egui::Ui, row: Rect, depth: usize, guides: &[bool], last: bool) {
    if depth == 0 {
        return;
    }
    let painter = ui.painter();
    let stroke = Stroke::new(1.0_f32, GUIDE);
    let column_x = |level: usize| row.left() + ROW_PAD + level as f32 * TREE_INDENT + TREE_INDENT / 2.0;
    for (level, &runs_on) in guides.iter().enumerate() {
        if runs_on {
            painter.vline(column_x(level), row.y_range(), stroke);
        }
    }
    let x = column_x(depth - 1);
    let middle = row.center().y;
    let bottom = if last { middle } else { row.bottom() };
    painter.vline(x, egui::Rangef::new(row.top(), bottom), stroke);
    painter.hline(egui::Rangef::new(x, x + TREE_INDENT / 2.0 + 2.0), middle, stroke);
}

/// A small filled dot in a row's lead, ending `lead` points into the row.
pub(crate) fn lead_dot(ui: &egui::Ui, row: Rect, lead: f32, color: Color32) {
    let center = egui::pos2(row.left() + ROW_PAD + lead - 7.0, row.center().y);
    ui.painter().circle_filled(center, 3.5, color);
}

/// A small right-pointing marker in a row's lead: where the processor is.
pub(crate) fn lead_arrow(ui: &egui::Ui, row: Rect, color: Color32) {
    let left = row.left() + ROW_PAD;
    let middle = row.center().y;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(left, middle - 4.5),
            egui::pos2(left + 7.0, middle),
            egui::pos2(left, middle + 4.5),
        ],
        color,
        Stroke::NONE,
    ));
}

/// A disclosure chevron in a row's lead: pointing right when closed, down
/// when open. Painted, not a glyph, so it needs no font coverage.
pub(crate) fn lead_chevron(ui: &egui::Ui, row: Rect, open: bool, color: Color32) {
    let left = row.left() + ROW_PAD;
    let middle = row.center().y;
    let points = if open {
        vec![
            egui::pos2(left, middle - 2.5),
            egui::pos2(left + 9.0, middle - 2.5),
            egui::pos2(left + 4.5, middle + 3.5),
        ]
    } else {
        vec![
            egui::pos2(left + 1.0, middle - 4.5),
            egui::pos2(left + 7.0, middle),
            egui::pos2(left + 1.0, middle + 4.5),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A status chip: `text` in `color` on a tint of it.
pub(crate) fn chip(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.55)))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(TEXT_CAPTION).color(color));
        })
        .response
}

/// A segmented switch over `options`: the picked one filled. Returns whether
/// the pick changed.
pub(crate) fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(BG_WELL)
        .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
        .corner_radius(8)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = egui::vec2(10.0, 3.0);
            ui.horizontal(|ui| {
                for &(value, label) in options {
                    let selected = *current == value;
                    let button = egui::Button::selectable(selected, label).corner_radius(6);
                    if ui.add(button).clicked() && !selected {
                        *current = value;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// A card's title, with a count of what it lists when it lists something.
pub(crate) fn card_title(ui: &mut egui::Ui, title: &str, count: Option<usize>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().color(TEXT_PRIMARY));
        if let Some(count) = count {
            egui::Frame::new()
                .fill(BG_WELL)
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(6, 0))
                .show(ui, |ui| {
                    ui.label(RichText::new(count.to_string()).size(TEXT_CAPTION).color(TEXT_MUTED));
                });
        }
    });
}

/// A message where a pane has nothing to show: a title and why, centred.
pub(crate) fn empty_state(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.add_space(ui.available_height() * 0.2);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).size(TEXT_TITLE).color(TEXT_PRIMARY));
        ui.add_space(4.0);
        ui.label(RichText::new(body).color(TEXT_MUTED));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fraction(cell: &Cell) -> Option<f32> {
        match cell.text {
            CellText::Gauge(fraction, _) => Some(fraction),
            CellText::Plain(..) | CellText::Job(_) => None,
        }
    }

    #[test]
    fn a_gauge_cell_clamps_its_fraction() {
        assert_eq!(fraction(&Cell::gauge(1.5, 6)), Some(1.0));
        assert_eq!(fraction(&Cell::gauge(-0.2, 6)), Some(0.0));
        assert_eq!(fraction(&Cell::gauge(0.5, 6)), Some(0.5));
        assert_eq!(fraction(&Cell::left("x", TEXT_PRIMARY, 1)), None);
    }
}
