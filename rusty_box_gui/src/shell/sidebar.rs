//! The desktop's navigation.
//!
//! `VmLibraryEntry` is what a VM profile summarises as — the strings a row
//! shows and a search matches against — and both shells hold their library as
//! a list of them, in a drawer that is open or closed as `Drawer` says.
//! `Sidebar` is the desktop's only navigation: every profile as a
//! `selection_row`, the selected one open to its pages, scrolling under a
//! header and a search field that stay put. It draws from borrowed data and
//! reports the click; the app decides what the click means.

#[cfg(not(target_arch = "wasm32"))]
use crate::shell::destination::{Destination, ShellPage, SidebarAction};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::theme::{
    ACCENT_AMBER, BG_PANEL, SPACE_GROUP, SPACE_ITEM, STROKE_HAIRLINE, TEXT_BODY, TEXT_CAPTION,
    TEXT_MUTED,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::shell::widgets::{selection_row, RowMark, ShellStateBadge, CHILD_INDENT, ROOT_INDENT};
#[cfg(not(target_arch = "wasm32"))]
use egui::{RichText, Stroke};

/// Whether a VM in the list is a file in the library, a library file that
/// is behind the VM because its last write failed, or only in memory. The
/// browser shell has no library, so its one entry carries no source.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntrySource {
    /// A library file; the row is the VM's name.
    Saved,
    /// Only in memory — the command line's temporary VM; the row says so.
    Unsaved,
    /// A library file the last write to failed; the row says so.
    WriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VmLibraryEntry {
    pub(crate) name: String,
    pub(crate) boot: String,
    pub(crate) memory: String,
    pub(crate) disk: String,
    pub(crate) cdrom: String,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) source: EntrySource,
}

impl VmLibraryEntry {
    pub(crate) fn new(
        name: impl Into<String>,
        boot: impl Into<String>,
        memory: impl Into<String>,
        disk: impl Into<String>,
        cdrom: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            boot: boot.into(),
            memory: memory.into(),
            disk: disk.into(),
            cdrom: cdrom.into(),
            #[cfg(not(target_arch = "wasm32"))]
            source: EntrySource::Saved,
        }
    }

    /// The same entry, marked as a VM that is not in the library.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn unsaved(self) -> Self {
        Self {
            source: EntrySource::Unsaved,
            ..self
        }
    }

    /// The same entry, marked as a library VM whose last write failed.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn write_failed(self) -> Self {
        Self {
            source: EntrySource::WriteFailed,
            ..self
        }
    }

    /// The text of the entry's row.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn row_label(&self) -> String {
        match self.source {
            EntrySource::Saved => self.name.clone(),
            EntrySource::Unsaved => format!("{} (unsaved)", self.name),
            EntrySource::WriteFailed => format!("{} (save failed)", self.name),
        }
    }

    pub(crate) fn matches_filter(&self, filter: &str) -> bool {
        filter.is_empty()
            || self.name.to_ascii_lowercase().contains(filter)
            || self.boot.to_ascii_lowercase().contains(filter)
            || self.disk.to_ascii_lowercase().contains(filter)
            || self.cdrom.to_ascii_lowercase().contains(filter)
    }
}

/// Whether the VM library's drawer is open beside the page or folded away.
/// Both shells draw it every frame: a closed drawer leaves a grab handle at
/// the left edge, from which it is dragged back open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drawer {
    Open,
    Closed,
}

impl Drawer {
    /// The drawer in its other state: what the VM bar's `☰` and the browser
    /// toolbar's Library toggle make of it.
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Open => Self::Closed,
            Self::Closed => Self::Open,
        }
    }
}

/// The width the sidebar opens at, and the narrowest a drag may make it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SIDEBAR_DEFAULT_WIDTH: f32 = 200.0;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 170.0;

/// The desktop's only navigation, kept in a drawer: every VM profile, with
/// the selected one expanded into its pages, and under them the library files
/// that do not load, each with its Delete. It borrows what it draws and the
/// drawer's state; `show` reports what was clicked, and the caller owns the
/// consequences.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Sidebar<'a> {
    /// Every VM in the library, in the library's order.
    pub(crate) entries: &'a [VmLibraryEntry],
    /// The indices into `entries` the search leaves, in the order drawn.
    pub(crate) visible: &'a [usize],
    /// The VM and page shown: that VM's row is expanded, that page's marked.
    pub(crate) destination: Destination,
    /// The search field's text.
    pub(crate) filter: &'a mut String,
    /// The shown VM's state, whose colour is the dot on that VM's row.
    pub(crate) badge: ShellStateBadge,
    /// The library files that do not load, listed under "Could not load".
    pub(crate) broken: &'a [crate::library::BrokenVmFile],
    /// Open or closed; a drag across the drawer's edge changes it.
    pub(crate) drawer: &'a mut Drawer,
}

#[cfg(not(target_arch = "wasm32"))]
impl Sidebar<'_> {
    /// Draws the drawer and reports what was clicked in it. Called every
    /// frame, open or closed: a closed drawer leaves egui's grab handle at
    /// the left edge, from which it is dragged back open, and an open one
    /// folds away when its edge is dragged in past `SIDEBAR_MIN_WIDTH`. The
    /// header row and the search field stay put; the rows under them scroll,
    /// by the wheel, by the scroll bar, or, once egui has seen a touch screen,
    /// by a finger dragged across them.
    pub(crate) fn show(self, ui: &mut egui::Ui) -> Option<SidebarAction> {
        let Sidebar {
            entries,
            visible,
            destination,
            filter,
            badge,
            broken,
            drawer,
        } = self;
        let mut open = *drawer == Drawer::Open;
        let shown = egui::Panel::left("vm_sidebar")
            .resizable(true)
            .default_size(SIDEBAR_DEFAULT_WIDTH)
            .min_size(SIDEBAR_MIN_WIDTH)
            .frame(
                egui::Frame::new()
                    .fill(BG_PANEL)
                    .stroke(Stroke::new(1.0_f32, STROKE_HAIRLINE))
                    .inner_margin(egui::Margin::symmetric(8, 10)),
            )
            .show_collapsible(ui, &mut open, |ui| {
                let mut action = None;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("My computer")
                            .size(TEXT_CAPTION)
                            .color(TEXT_MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("+")
                            .on_hover_text("New VM (a copy of the selected one)")
                            .clicked()
                        {
                            action = Some(SidebarAction::NewVm);
                        }
                    });
                });
                ui.add_space(SPACE_ITEM);
                ui.add(
                    egui::TextEdit::singleline(filter)
                        .desired_width(f32::INFINITY)
                        .hint_text("Search VMs"),
                );
                ui.add_space(SPACE_ITEM);

                if entries.is_empty() {
                    ui.label(
                        RichText::new("No VMs")
                            .size(TEXT_BODY)
                            .color(TEXT_MUTED),
                    );
                    return action;
                }

                egui::ScrollArea::vertical()
                    .id_salt("vm_list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for &index in visible {
                            let is_selected_vm = destination.vm() == index;
                            let dot = is_selected_vm.then_some(badge.color);
                            let vm_mark = if is_selected_vm {
                                RowMark::Expanded
                            } else {
                                RowMark::Plain
                            };
                            if selection_row(
                                ui,
                                &entries[index].row_label(),
                                ROOT_INDENT,
                                vm_mark,
                                dot,
                            )
                            .clicked()
                            {
                                action = Some(SidebarAction::Select(destination.select_vm(index)));
                            }
                            if !is_selected_vm {
                                continue;
                            }
                            for page in ShellPage::ALL {
                                let page_mark = if destination.page() == page {
                                    RowMark::Destination
                                } else {
                                    RowMark::Plain
                                };
                                if selection_row(ui, page.label(), CHILD_INDENT, page_mark, None)
                                    .clicked()
                                {
                                    action =
                                        Some(SidebarAction::Select(destination.select_page(page)));
                                }
                            }
                        }

                        if !broken.is_empty() {
                            ui.add_space(SPACE_GROUP);
                            ui.label(
                                RichText::new("Could not load")
                                    .size(TEXT_CAPTION)
                                    .color(TEXT_MUTED),
                            );
                            for (index, file) in broken.iter().enumerate() {
                                let name = file.path.file_name().map_or_else(String::new, |name| {
                                    name.to_string_lossy().into_owned()
                                });
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(name).color(ACCENT_AMBER))
                                        .on_hover_text(&file.error);
                                    if ui.small_button("Delete").clicked() {
                                        action = Some(SidebarAction::DeleteBroken(index));
                                    }
                                });
                            }
                        }
                    });
                action
            });
        *drawer = if open { Drawer::Open } else { Drawer::Closed };
        shown.and_then(|panel| panel.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::theme::TEXT_MUTED;
    use egui_kittest::{kittest::Queryable, Harness};

    const WINDOW: egui::Vec2 = egui::vec2(640.0, 400.0);

    /// The steps `Harness::run` may take before the window stops asking for
    /// frames. egui spreads a wheel turn's scroll over the frames after it
    /// (`WheelState::after_events`), so a turn that crosses the whole list
    /// takes `run` six steps to settle; the harness's own limit is four.
    const SETTLE_STEPS: u64 = 8;

    struct Library {
        entries: Vec<VmLibraryEntry>,
        visible: Vec<usize>,
        destination: Destination,
        filter: String,
        drawer: Drawer,
        picked: Option<SidebarAction>,
        /// The x of the drawer's fixed edge, where a closed drawer's grab
        /// handle lies: the left of the area the harness draws the app in,
        /// which kittest insets from the window's edge.
        left_edge: f32,
    }

    fn library_of(count: usize) -> Library {
        Library {
            entries: (0..count)
                .map(|n| {
                    VmLibraryEntry::new(format!("VM {n:02}"), "cdrom", "256 MB", "None", "boot.iso")
                })
                .collect(),
            visible: (0..count).collect(),
            destination: Destination::new(0, ShellPage::Home),
            filter: String::new(),
            drawer: Drawer::Open,
            picked: None,
            left_edge: 0.0,
        }
    }

    fn harness(library: Library) -> Harness<'static, Library> {
        Harness::builder()
            .with_size(WINDOW)
            .with_max_steps(SETTLE_STEPS)
            .build_ui_state(
                |ui, library: &mut Library| {
                    library.left_edge = ui.available_rect_before_wrap().left();
                    let action = Sidebar {
                        entries: &library.entries,
                        visible: &library.visible,
                        destination: library.destination,
                        filter: &mut library.filter,
                        badge: ShellStateBadge {
                            label: "Stopped",
                            color: TEXT_MUTED,
                        },
                        broken: &[],
                        drawer: &mut library.drawer,
                    }
                    .show(ui);
                    if action.is_some() {
                        library.picked = action;
                    }
                },
                library,
            )
    }

    /// Whether the row labelled `label` lies inside the window.
    fn on_screen(harness: &Harness<'_, Library>, label: &str) -> bool {
        let rect = harness.get_by_label(label).rect();
        rect.top() >= 0.0 && rect.bottom() <= WINDOW.y
    }

    /// A point over the list, below the header and the search field.
    const OVER_THE_LIST: egui::Pos2 = egui::pos2(90.0, 300.0);

    #[test]
    fn a_long_library_scrolls_with_the_wheel_and_its_last_vm_can_be_picked() {
        let mut harness = harness(library_of(60));
        harness.run();
        assert!(
            !on_screen(&harness, "VM 59"),
            "the last row starts below the fold"
        );

        harness.hover_at(OVER_THE_LIST);
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -10_000.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        assert!(on_screen(&harness, "VM 59"), "the wheel scrolled the list");

        harness.get_by_label("VM 59").click();
        harness.run();
        assert!(matches!(
            harness.state().picked,
            Some(SidebarAction::Select(destination)) if destination.vm() == 59
        ));
    }

    /// A finger dragged up the list scrolls it, as egui-winit reports a touch:
    /// a `Touch` event and the pointer events it simulates.
    #[test]
    fn a_long_library_scrolls_with_a_finger() {
        let mut harness = harness(library_of(60));
        harness.run();
        for _ in 0..8 {
            if on_screen(&harness, "VM 59") {
                break;
            }
            swipe(
                &mut harness,
                egui::pos2(90.0, 380.0),
                egui::pos2(90.0, 120.0),
            );
        }
        assert!(on_screen(&harness, "VM 59"));
    }

    fn swipe(harness: &mut Harness<'_, Library>, from: egui::Pos2, to: egui::Pos2) {
        let touch = |phase, pos| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase,
            pos,
            force: None,
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        harness.event(touch(egui::TouchPhase::Start, from));
        harness.event(egui::Event::PointerMoved(from));
        harness.event(button(from, true));
        harness.step();
        for n in 1..=10u8 {
            let pos = from + (to - from) * (f32::from(n) / 10.0);
            harness.event(touch(egui::TouchPhase::Move, pos));
            harness.event(egui::Event::PointerMoved(pos));
            harness.step();
        }
        harness.event(touch(egui::TouchPhase::End, to));
        harness.event(button(to, false));
        harness.run();
    }

    #[test]
    fn a_closed_drawer_shows_no_rows_and_is_dragged_open_from_its_edge() {
        let mut library = library_of(3);
        library.drawer = Drawer::Closed;
        let mut harness = harness(library);
        harness.run();
        assert!(
            harness.query_by_label("VM 00").is_none(),
            "a closed drawer draws no rows"
        );

        // The press lands on the handle, a point in from the drawer's edge.
        // The drag then reaches the point it is released at before the
        // release, as a real one does: egui reads a handle's drag only while
        // the button is down.
        let edge = harness.state().left_edge;
        harness.drag_at(egui::pos2(edge + 1.0, 200.0));
        harness.step();
        harness.hover_at(egui::pos2(120.0, 200.0));
        harness.step();
        harness.hover_at(egui::pos2(260.0, 200.0));
        harness.step();
        harness.drop_at(egui::pos2(260.0, 200.0));
        harness.run();
        assert_eq!(harness.state().drawer, Drawer::Open);
        assert!(harness.query_by_label("VM 00").is_some());
    }

    #[test]
    fn toggling_a_drawer_flips_it() {
        assert_eq!(Drawer::Open.toggled(), Drawer::Closed);
        assert_eq!(Drawer::Closed.toggled(), Drawer::Open);
    }
}
