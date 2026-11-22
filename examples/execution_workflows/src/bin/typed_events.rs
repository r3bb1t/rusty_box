use rusty_box::events::{App, ExecutionRequest, publish_events};

mod process {
    use rusty_box::events::{App, AppHandlers};

    pub struct Created(pub u32);
    pub struct Exited(pub u32);

    pub trait ProcessState {
        fn active_processes(&mut self) -> &mut Vec<u32>;
    }

    pub fn register<S: ProcessState, X, H: AppHandlers<S, X>>(
        app: App<S, X, H>,
    ) -> App<S, X, impl AppHandlers<S, X>> {
        app.on(|event: &Created, state: &mut S| state.active_processes().push(event.0))
            .on(|event: &Exited, state: &mut S| {
                state.active_processes().retain(|pid| *pid != event.0);
            })
    }
}

struct DeviceRegistered(&'static str);

struct AppState {
    processes: Vec<u32>,
    created: u64,
}

impl process::ProcessState for AppState {
    fn active_processes(&mut self) -> &mut Vec<u32> {
        &mut self.processes
    }
}

fn main() {
    let mut devices = Vec::new();
    let handlers = process::register(App::<AppState>::new())
        .on(|event: &process::Created, state: &mut AppState| {
            assert_eq!(state.processes.last(), Some(&event.0));
            state.created += 1;
        })
        .on(|event: &DeviceRegistered| devices.push(event.0));

    let mut app = handlers.with_state(AppState {
        processes: Vec::new(),
        created: 0,
    });
    assert_eq!(
        publish_events(
            [process::Created(10), process::Created(11)],
            &mut app,
            &mut ()
        ),
        Ok(ExecutionRequest::None)
    );
    assert_eq!(
        app.dispatch(&DeviceRegistered("mailbox")),
        Ok(ExecutionRequest::None)
    );
    assert_eq!(
        app.dispatch(&process::Exited(10)),
        Ok(ExecutionRequest::None)
    );

    let state = app.into_state();
    assert_eq!((state.processes, state.created), (vec![11], 2));
    assert_eq!(devices, ["mailbox"]);
}
