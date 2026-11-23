use std::process::Command;

#[test]
#[ignore = "requires the documented Debian kernel and observation initramfs"]
fn observes_two_independent_guest_programs_with_and_without_elf_reads() {
    let kernel = std::env::var_os("RB_OBSERVATION_KERNEL").expect("Debian i386 kernel path");
    let initramfs =
        std::env::var_os("RB_OBSERVATION_INITRAMFS").expect("observation initramfs path");

    for inspect in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_linux_observe"));
        command.arg(&kernel).arg(&initramfs);
        if inspect {
            command.arg("--elf");
        }
        let output = command.output().unwrap();
        let transcript = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{transcript}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(transcript.contains("Guest output sum=22"));
        assert!(transcript.contains("Guest output argv0=/bin/program_text"));
        assert_eq!(transcript.matches("Process /bin/").count(), 2);
        assert!(transcript.matches("Executable mapping ").count() >= 2);
        assert_eq!(
            transcript.matches("Shared ELF result ").count(),
            if inspect { 2 } else { 0 }
        );
        if inspect {
            assert!(transcript.contains("Mapped ELF Executable link entry="));
            let pie = transcript
                .lines()
                .find(|line| line.starts_with("Mapped ELF SharedObject "))
                .unwrap();
            assert!(!pie.ends_with("bias=0x0"), "{pie}");
        }
    }
}
