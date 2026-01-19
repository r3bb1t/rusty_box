mod siminterface;
pub mod egui_gui;
pub mod gui_trait;
pub mod keymap;
pub mod nogui;
pub mod term;

pub use egui_gui::EguiGui;
pub use gui_trait::{BxGui, DisplayMode, VgaTextModeInfo};
pub use keymap::{ascii_to_scancode, char_to_scancode_sequence, needs_shift};
pub use nogui::NoGui;
pub use term::TermGui;

