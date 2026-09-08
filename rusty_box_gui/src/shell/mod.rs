//! The desktop shell's design vocabulary.
//!
//! `theme` fixes the palette and the type and spacing scale every pane draws
//! from, and applies them to the egui context and to a card frame. `widgets`
//! is the set of pieces a pane is assembled from on that scale: the page
//! header, the field row, the status dot and badge, the hairlines that join
//! stacked panels, and the action tiles. `destination` is where the shell is
//! pointed — which VM profile and which of its pages, held as one value so
//! the pair cannot disagree — and the actions its two chrome surfaces report.
//! `sidebar` is the desktop's navigation: the tree of VM profiles, the
//! selected one open to its pages, and the entry each row is drawn from.

pub(crate) mod destination;
pub(crate) mod sidebar;
pub(crate) mod theme;
pub(crate) mod widgets;
