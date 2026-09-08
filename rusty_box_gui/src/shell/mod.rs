//! The desktop shell's design vocabulary.
//!
//! `theme` fixes the palette and the type and spacing scale every pane draws
//! from, and applies them to the egui context and to a card frame. `widgets`
//! is the set of pieces a pane is assembled from on that scale: the page
//! header, the field row, the status dot and badge, the hairlines that join
//! stacked panels, and the action tiles.

pub(crate) mod theme;
pub(crate) mod widgets;
