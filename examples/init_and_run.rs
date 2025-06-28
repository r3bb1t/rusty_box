use rusty_box::cpu::{builder::BxCpuBuilder, core_i7_skylake::Corei7SkylakeX};

// It doesn't run since it tries to allocate 54 mb on stack  💀💀💀

fn main() {
    tracing_subscriber::fmt()
        .without_time()
        .with_target(false)
        .init();

    let builder: BxCpuBuilder<Corei7SkylakeX> = *Box::new(BxCpuBuilder::new());
    let cpu = Box::new(
        builder
            .build()
            .expect("An error occured while building cpu"),
    );

    println!("{cpu:#?}");
}
