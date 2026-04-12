//! Rusty Box Web — x86 emulator running in the browser via WASM.
//!
//! Uses eframe/egui for rendering. On WASM, the emulator runs cooperatively
//! (step_batch per frame). On native, it runs the same cooperative loop
//! for testing without needing a browser.

mod app;

// ---- Native entry point ----
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    env_logger::init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([720.0, 400.0])
            .with_title("Rusty Box"),
        ..Default::default()
    };

    eframe::run_native(
        "Rusty Box",
        native_options,
        Box::new(|cc| Ok(Box::new(app::WasmEmulatorApp::new(cc)))),
    )
}

// ---- WASM entry point ----
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find #the_canvas_id canvas element")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("Element is not a canvas");

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(app::WasmEmulatorApp::new(cc)))),
            )
            .await
            .expect("Failed to start eframe");
    });
}
