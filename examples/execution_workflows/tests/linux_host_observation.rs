use std::process::Command;

fn run_observer(flags: &[&str]) -> String {
    let kernel = std::env::var_os("RB_OBSERVATION_KERNEL").expect("Debian i386 kernel path");
    let initramfs =
        std::env::var_os("RB_HOST_OBSERVATION_INITRAMFS").expect("host-only initramfs path");
    let output = Command::new(env!("CARGO_BIN_EXE_linux_host_observe"))
        .args([kernel, initramfs])
        .args(flags)
        .output()
        .unwrap();
    let transcript = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{transcript}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(transcript.matches("PendingUserReturn").count(), 2);
    assert!(transcript.contains("Host process program_sum pid="));
    assert!(transcript.contains("Host process program_text pid="));
    assert!(transcript.contains("Guest output sum=22"));
    assert!(transcript.contains("Guest output argv0=/bin/program_text"));
    assert!(!transcript.contains("optional ELF"));
    transcript.into_owned()
}

#[test]
#[ignore = "requires the pinned Debian kernel and the host-only initramfs"]
fn samples_two_ordinary_programs_at_the_kernel_boundary() {
    let transcript = run_observer(&[]);
    assert_eq!(transcript.matches("Host image pid=").count(), 2);
    assert!(transcript.matches("Host executable VMA ").count() >= 2);
    assert!(transcript.contains("Prepared 2 executable images"));
}

#[test]
#[ignore = "requires the pinned Debian kernel and the host-only initramfs"]
fn process_only_handlers_skip_executable_image_preparation() {
    let transcript = run_observer(&["--process-only"]);
    assert!(!transcript.contains("Host image pid="));
    assert!(!transcript.contains("Host executable VMA "));
    assert!(transcript.contains("Prepared 0 executable images"));
}
