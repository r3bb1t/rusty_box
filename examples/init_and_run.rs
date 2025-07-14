use rusty_box::cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX};

fn main() {
    const THREAD_STACK_SIZE: usize = if cfg!(debug_assertions) {
        350 * 1024 * 1024
    } else {
        150 * 1024 * 1024
    };
    std::thread::Builder::new()
        .stack_size(THREAD_STACK_SIZE)
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
    let mut cpu = builder
        .build()
        .expect("An error occured while building cpu");

    tracing::info!("Created cpu");

    tracing::info!(
        "Before setting registers: Rax: {} , Rip: {}",
        cpu.rax(),
        cpu.rip()
    );

    cpu.set_rax(777);
    cpu.set_rip(222);
    tracing::info!("After: Rax: {} , Rip: {}", cpu.rax(), cpu.rip());
}
