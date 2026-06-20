#[cfg(not(target_arch = "wasm32"))]
use clap::Parser;
#[cfg(not(target_arch = "wasm32"))]
use std::process::ExitCode;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    const EMULATOR_STACK_SIZE: usize = 1500 * 1024 * 1024;

    let args = rusty_box_gui::Args::parse();
    let config = match rusty_box_gui::config::load_config(&args) {
        Ok(config) => config,
        Err(error) => return print_result(Err(error)),
    };

    #[cfg(feature = "gui-egui")]
    if config.display == rusty_box_gui::DisplayBackend::Egui {
        return print_result(rusty_box_gui::run_resolved(config));
    }

    let thread = match std::thread::Builder::new()
        .name("rusty_box_gui".to_owned())
        .stack_size(EMULATOR_STACK_SIZE)
        .spawn(move || rusty_box_gui::run_resolved(config))
    {
        Ok(thread) => thread,
        Err(error) => {
            eprintln!("rusty_box_gui: failed to start emulator thread: {error}");
            return ExitCode::FAILURE;
        }
    };

    match thread.join() {
        Ok(result) => print_result(result),
        Err(_) => {
            eprintln!("rusty_box_gui: emulator thread panicked");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn print_result(result: Result<rusty_box_gui::RunSummary, rusty_box_gui::RunError>) -> ExitCode {
    match result {
        Ok(summary) => {
            println!(
                "rusty_box_gui: executed {} instructions",
                summary.instructions_executed
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rusty_box_gui: {error}");
            ExitCode::FAILURE
        }
    }
}

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
                Box::new(|cc| Ok(Box::new(rusty_box_gui::app::WebShellApp::new(cc)))),
            )
            .await
            .expect("Failed to start eframe");
    });
}
