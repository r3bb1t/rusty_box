use rusty_box::{
    cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX},
    memory::{BxMemC, BxMemoryStubC},
};

fn main() {
    const THREAD_STACK_SIZE: usize = if cfg!(debug_assertions) {
        1500 * 1024 * 1024
    } else {
        500 * 1024 * 1024
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
    cpu.set_rip(0);

    tracing::info!("After: Rax: {} , Rip: {}", cpu.rax(), cpu.rip());


    let guest_mb = 32;
    let host_mb = 32;
    let mem_stub =
        BxMemoryStubC::create_and_init(guest_mb * 1024 * 1024, host_mb * 1024 * 1024, 128 * 1024)
            .unwrap();
    let mut mem = BxMemC::new(mem_stub, false);
    cpu.cpu_loop(&mut mem, &[]).unwrap()
}
