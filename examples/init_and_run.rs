use rusty_box::cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX};

fn main() {
    std::thread::Builder::new()
        .stack_size(150 * 1024 * 1024) // Allocate 150 mb stack
        .name("Emulator thread".to_string())
        .spawn(inner_main)
        .expect("thread spawn error")
        .join()
        .expect("error while joining spawned thread");
}

fn inner_main() {
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .init();
    let builder: BxCpuBuilder<Corei7SkylakeX> = BxCpuBuilder::new();

    tracing::info!("Builder: {builder:#?}");
    let cpu = builder
        .build()
        .expect("An error occured while building cpu");

    tracing::info!("Created cpu");
}
