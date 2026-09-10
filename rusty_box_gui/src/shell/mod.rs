//! The desktop shell's design vocabulary.
//!
//! `theme` fixes the palette and the type and spacing scale every pane draws
//! from, and applies them to the egui context and to a card frame. `widgets`
//! is the set of pieces a pane is assembled from on that scale: the page
//! header, the field row, the selection row the sidebar tree and the Hardware
//! device list share, the status dot and badge, the hairlines that join
//! stacked panels, and the action tiles. `destination` is where the shell is
//! pointed — which VM profile and which of its pages, held as one value so
//! the pair cannot disagree — and the actions its two chrome surfaces report.
//! `sidebar` is the desktop's navigation: the tree of VM profiles, the
//! selected one open to its pages, and the entry each row is drawn from.
//! `vm_bar` is the one bar above a page: the selected VM's name, its state
//! badge, and the verbs that change that state, with the console's own
//! controls shown only while the console page is.

pub(crate) mod destination;
pub(crate) mod sidebar;
pub(crate) mod theme;
pub(crate) mod vm_bar;
pub(crate) mod widgets;
