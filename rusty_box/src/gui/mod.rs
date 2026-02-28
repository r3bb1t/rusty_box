pub mod gui_trait;
pub mod keymap;
pub mod nogui;
pub mod shared_display;
mod siminterface;
pub mod vga_font;

pub use gui_trait::{BxGui, DisplayMode, VgaTextModeInfo};
pub use keymap::{ascii_to_scancode, char_to_scancode_sequence, needs_shift};
pub use nogui::NoGui;

#[cfg(all(feature = "gui-egui", feature = "std"))]
pub mod egui_gui;
#[cfg(all(feature = "gui-egui", feature = "std"))]
pub use egui_gui::{BridgeGui, EguiGui};

#[cfg(all(feature = "gui-egui", feature = "std"))]
pub mod eframe_app;
#[cfg(all(feature = "gui-egui", feature = "std"))]
pub use eframe_app::RustyBoxApp;

#[cfg(feature = "std")]
pub mod term;
#[cfg(feature = "std")]
pub use term::TermGui;
