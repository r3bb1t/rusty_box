# Linux/i386 user-mode execution

`rusty_box_linux_user` runs a static ELF32/i386 executable with the x86 interpreter. The adapter supplies an initial process stack, page tables and the `exit`, `read`, and `write` calls through `int $0x80`. It uses the CPU-only owner and requires no firmware, Linux kernel or PC devices. The library is `no_std` with `alloc`.

```rust
use rusty_box_linux_user::{BufferedIo, Image, Launch, Stop};

let image = Image::parse(&elf_bytes)?;
let mut process = image.launch(
    Launch { args: &[b"filter"], env: &[b"LANG=C"] },
    BufferedIo::new(b"hello from rusty_box\n"),
)?;
let run = process.run(50_000)?;
if let Stop::Exited(exit) = run.stop {
    assert_eq!(exit.status, 0);
    assert_eq!(process.into_io().stdout(), b"HELLO FROM RUSTY_BOX\n");
}
```

Implement `Io` to provide your own standard streams. Short transfers and errors go back to the program through EAX. A `Stop::Budget` result preserves the process for another `run`. Every other stop or error ends it. Reuse the parsed `Image` to construct another process with fresh state and different input. `ProgramStarted` and `ProgramExited` are ordinary event types that the embedding application can publish to `App::route()` handlers. [Architecture and execution contract](../docs/vnext/user-mode-foundation.md)

## Build and run the two guest executables

From the repository root, use GCC with i386 code generation and a GNU-compatible linker that supports `elf_i386`. The test fixtures link without a C library.

```sh
export RB_USER_FIXTURES="$(mktemp -d)"
sh rusty_box_linux_user/tests/guest/build.sh "$RB_USER_FIXTURES"
printf 'hello from rusty_box\n' > "$RB_USER_FIXTURES/input"
cargo run --locked --release -p rusty_box_linux_user --example run -- \
  "$RB_USER_FIXTURES/compute" "$RB_USER_FIXTURES/filter" "$RB_USER_FIXTURES/input"
```

`compute` checks its argument and environment pointers, reads the auxiliary vector, verifies zero-initialized BSS and checks a computed sum of 34. `filter` reads the supplied input in short chunks, uppercases ASCII letters and writes the result. Both exit with status 0. The Rust example supplies the streams and recovers its non-`Clone` application state after publishing both lifecycle event types.

```sh
cargo test --locked --release -p rusty_box_linux_user
cargo test --locked --release -p rusty_box_linux_user --test programs -- --ignored
cargo test --locked --release -p rusty_box_linux_user --lib measure_user_mode -- --ignored --nocapture
```

The second command exercises the independently compiled programs with partial reads and writes, repeated process construction, and different caller inputs. The third measures the user-mode stages separately and runs an identical guest loop through the CPU-only and user-mode APIs. Both commands use `RB_USER_FIXTURES` from the build above.
