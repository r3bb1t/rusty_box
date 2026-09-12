#![cfg_attr(
    all(windows, feature = "windows-gui-subsystem"),
    windows_subsystem = "windows"
)]

#[cfg(not(target_arch = "wasm32"))]
use clap::Parser;
#[cfg(not(target_arch = "wasm32"))]
use std::process::ExitCode;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    const EMULATOR_STACK_SIZE: usize = 1500 * 1024 * 1024;

    let args = rusty_box_gui::Args::parse();

    // A command line that names no machine opens the egui shell on its VM
    // library alone. A run flag on such a command line is refused rather than
    // dropped: the engine, the processor and the log level are each VM's own
    // setting, and there is no VM here for the flag to apply to.
    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    if !args.names_a_machine()
        && args
            .display
            .is_none_or(|display| display == rusty_box_gui::DisplayBackend::Egui)
    {
        let flags = args.run_flags();
        if !flags.is_empty() {
            return print_result(Err(rusty_box_gui::RunError::RunFlagsNeedAMachine {
                flags,
            }));
        }
        return print_result(rusty_box_gui::run_shell(None));
    }

    let config = match rusty_box_gui::config::load_config(&args) {
        Ok(config) => config,
        Err(error) => return print_result(Err(error)),
    };

    #[cfg(all(feature = "gui-egui", not(target_os = "android")))]
    if config.display == rusty_box_gui::DisplayBackend::Egui {
        let launch = rusty_box_gui::LaunchVm::from_args(&args, config);
        return print_result(rusty_box_gui::run_shell(Some(launch)));
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
            match summary.instructions_executed {
                Some(count) => println!("rusty_box_gui: executed {count} instructions"),
                None => println!(
                    "rusty_box_gui: the guest ran on the hypervisor, which does not count \
                     instructions"
                ),
            }
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
    if let Err(error) = eframe::WebLogger::init(log::LevelFilter::Debug) {
        web_sys::console::warn_1(
            &format!("rusty_box_gui: the log does not reach this console: {error}").into(),
        );
    }
    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        if let Err(failure) = start_web_shell(web_options).await {
            report_web_start_failure(&failure);
        }
    });
}

/// Starts the browser shell on the page's `#the_canvas_id` canvas, or says
/// why it could not.
#[cfg(target_arch = "wasm32")]
async fn start_web_shell(web_options: eframe::WebOptions) -> Result<(), String> {
    use eframe::wasm_bindgen::JsCast as _;

    let document = web_sys::window()
        .ok_or("the page has no window")?
        .document()
        .ok_or("the window has no document")?;
    let canvas = document
        .get_element_by_id("the_canvas_id")
        .ok_or("the page has no #the_canvas_id element")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "#the_canvas_id is not a canvas")?;

    eframe::WebRunner::new()
        .start(
            canvas,
            web_options,
            Box::new(|cc| Ok(Box::new(rusty_box_gui::app::WebShellApp::new(cc)))),
        )
        .await
        .map_err(|error| format!("eframe did not start: {error:?}"))
}

/// Puts a start-up failure where the person at the page can find it. A
/// browser `main` has no caller to return it to, so it goes to the browser
/// console, and replaces the page's content when there is a document to
/// write it into.
#[cfg(target_arch = "wasm32")]
fn report_web_start_failure(failure: &str) {
    let message = format!("rusty_box_gui could not start: {failure}");
    web_sys::console::error_1(&message.as_str().into());
    let Some(body) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.body())
    else {
        return;
    };
    // The page paints a dark ground, so the notice names its own colour.
    if let Err(error) = body.set_attribute(
        "style",
        "color: #E6EDF3; font: 14px monospace; padding: 1em; white-space: pre-wrap",
    ) {
        web_sys::console::error_2(
            &"rusty_box_gui: the failure notice could not be styled:".into(),
            &error,
        );
    }
    body.set_text_content(Some(&message));
}
