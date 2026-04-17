# Monomorphized Instrumentation API

## Problem

The current instrumentation registry stores tracers as `Vec<Box<dyn Instrumentation>>`. This has three costs:

1. **Runtime dispatch** — each `fire_*` call does a vtable lookup per tracer per event. For `before_execution` (called every instruction), this adds ~5-10 ns per tracer.
2. **Heap allocation** — `Box<dyn>` requires `alloc`. The project is considering dropping `alloc` as a hard dependency.
3. **Unsafe ergonomics** — recovering the concrete tracer type from `Box<dyn>` requires `Any`-based downcasting, which leaked `unsafe` into user examples.

## Solution

Replace the `Box<dyn Instrumentation>` slot with a monomorphized generic parameter `T: Instrumentation` that propagates through `InstrumentationRegistry<T>`, `BxCpuC<'c, I, T>`, and `Emulator<'a, I, T>`, defaulting to `()`.

Closure-based hooks (`hook_add_code`, `hook_add_mem`, etc.) remain behind an `alloc` feature gate with their current `Vec<Box<dyn FnMut>>` storage. Users who don't have `alloc` use only the monomorphized trait path.

## Design

### Generic parameter propagation

```
Emulator<'a, I, T = ()>
  └── BxCpuC<'c, I, T = ()>
        └── InstrumentationRegistry<T = ()>
              ├── tracer: T                    // monomorphized, inlined
              ├── code_hooks: Vec<CodeHook>     // alloc-gated, dyn
              └── active: HookMask
```

Default `T = ()` means existing code compiles unchanged:
```rust
// Existing — no instrumentation, zero overhead
let emu = Emulator::<Corei7SkylakeX>::new(config)?;

// With a tracer — all dispatch inlined
let emu = Emulator::<Corei7SkylakeX, SyscallTracer>::new_with_instrumentation(
    config,
    SyscallTracer::new(),
)?;
```

### `Instrumentation` trait

No supertraits. No `Any`, no `Send`, no `'static`. Just the callbacks with default no-ops:

```rust
pub trait Instrumentation {
    fn before_execution(&mut self, rip: u64, instr: &Instruction) {}
    fn after_execution(&mut self, rip: u64, instr: &Instruction) {}
    // ... all 25+ callbacks with default no-ops
}

// Unit type implements all-no-ops. The compiler eliminates every call.
impl Instrumentation for () {}
```

`Emulator<'a, I, T>` auto-derives `Send` when `T: Send`. No trait-level constraint needed.

### Tuple composition

Implement `Instrumentation` for tuples up to arity 8 via macro:

```rust
macro_rules! impl_instrumentation_tuple {
    ($($T:ident),+) => {
        impl<$($T: Instrumentation),+> Instrumentation for ($($T,)+) {
            fn before_execution(&mut self, rip: u64, instr: &Instruction) {
                let ($($T,)+) = self;
                $($T.before_execution(rip, instr);)+
            }
            // ... all other methods
        }
    }
}
impl_instrumentation_tuple!(A);
impl_instrumentation_tuple!(A, B);
impl_instrumentation_tuple!(A, B, C);
// ... up to 8
```

Usage:
```rust
let emu = Emulator::<Corei7SkylakeX, (SyscallTracer, CoverageTracer)>::new_with_instrumentation(
    config,
    (SyscallTracer::new(), CoverageTracer::new()),
)?;

// Typed access — no downcast, no unsafe
let (syscall, coverage) = emu.instrumentation();
println!("syscalls seen: {}", syscall.count);
```

### Typed accessor

```rust
impl<'a, I: BxCpuIdTrait, T: Instrumentation> Emulator<'a, I, T> {
    /// Direct typed reference to the installed tracer. Zero-cost.
    pub fn instrumentation(&self) -> &T { ... }
    pub fn instrumentation_mut(&mut self) -> &mut T { ... }
}
```

No `TypeId`, no `Any`, no `unsafe`. Just a field access.

### `HookMask` gating

The bitmask fast-path remains. For the monomorphized path, the question is: when `T = ()`, should the `has_exec()` check still exist?

Answer: yes. The pattern stays:

```rust
#[cfg(feature = "instrumentation")]
if self.instrumentation.active.has_exec() {
    self.instrumentation.fire_before_execution(rip, instr);
}
```

When `T = ()`, `fire_before_execution` is a no-op and LLVM eliminates the call. The bitmask check becomes dead code and is also eliminated. Net cost: zero instructions emitted.

When `T` is a real tracer, the bitmask check survives as a single predicted branch. This is useful because closure hooks share the same bitmask — if no closures are registered AND the tracer's `before_execution` is a no-op (default), the branch is still not-taken.

### Closure hooks

Unchanged. Feature-gated behind `alloc`:

```rust
#[cfg(feature = "alloc")]
pub fn hook_add_code<R, F>(&mut self, range: R, cb: F) -> HookHandle
where
    R: RangeBounds<u64>,
    F: FnMut(u64, &Instruction) + Send + 'static,
{ ... }
```

The `fire_*` methods dispatch both the monomorphized `T` and the closure vecs:

```rust
#[inline]
pub fn fire_before_execution(&mut self, rip: u64, instr: &Instruction) {
    self.tracer.before_execution(rip, instr);
    #[cfg(feature = "alloc")]
    for h in &mut self.code_hooks {
        if h.range.contains(rip) { (h.cb)(rip, instr); }
    }
}
```

### `BochsEntry` and `Vec<Box<dyn>>` removal

The `BochsEntry` struct, `add_bochs`, `remove_bochs`, `clear_bochs` methods, and the `Box<dyn Instrumentation>` import path are all deleted. The `set_instrumentation` / `clear_instrumentation` methods on `Emulator` are replaced by construction-time `new_with_instrumentation` and the field accessor.

### Construction API

```rust
impl<'a, I: BxCpuIdTrait> Emulator<'a, I, ()> {
    /// Existing constructor. T defaults to (), zero instrumentation.
    pub fn new(config: EmulatorConfig) -> Result<Box<Self>>;
}

impl<'a, I: BxCpuIdTrait, T: Instrumentation> Emulator<'a, I, T> {
    /// Constructor with a monomorphized tracer.
    pub fn new_with_instrumentation(config: EmulatorConfig, tracer: T) -> Result<Box<Self>>;

    /// Constructor with tracer + CPU mode setup (skip BIOS).
    pub fn new_with_mode_and_instrumentation(
        config: EmulatorConfig,
        mode: CpuSetupMode,
        tracer: T,
    ) -> Result<Box<Self>>;
}
```

No runtime `set_instrumentation` — the tracer type is fixed at construction. If users need to swap tracers dynamically, they use the `alloc`-gated closure hooks or reconstruct the emulator.

### Impact on `impl` blocks

`BxCpuC<'c, I>` becomes `BxCpuC<'c, I, T = ()>`. Every `impl` block gains `T: Instrumentation` (or `T` unconstrained where the block doesn't touch instrumentation). This affects ~78 `impl` blocks across ~40 files.

The change is mechanical: replace `impl<I: BxCpuIdTrait> BxCpuC<'_, I>` with `impl<I: BxCpuIdTrait, T: Instrumentation> BxCpuC<'_, I, T>`. An `ast_edit` pass handles it.

Files that don't reference `self.instrumentation` at all could use `impl<I: BxCpuIdTrait, T> BxCpuC<'_, I, T>` (no trait bound) to avoid pulling in the trait import. This is an optional refinement.

### `no_alloc` summary

| Component | `alloc` required | `core`-only |
|-----------|-----------------|-------------|
| Monomorphized tracer `T` | | field on struct |
| `Instrumentation` trait | | all default no-ops |
| Tuple composition `(A, B)` | | macro-generated |
| `instrumentation()` accessor | | field borrow |
| `HookMask` bitmask | | bitflags on `u32` |
| Closure hooks `hook_add_*` | `Vec<Box<dyn FnMut>>` | |
| `HookHandle` / `hook_del` | with `Vec` above | |

## Migration

### What changes for existing users

- `Emulator::<Corei7SkylakeX>::new(config)` — unchanged (default `T = ()`).
- `emu.set_instrumentation(Box::new(MyTracer))` — removed. Replace with `new_with_instrumentation(config, MyTracer::new())`.
- `emu.clear_instrumentation()` — removed. Tracer lives for the emulator's lifetime.
- `emu.instrumentation_mut::<MyTracer>()` → `emu.instrumentation_mut()` — returns `&mut T` directly, no `TypeId` dance.
- Hook closures (`hook_add_code`, etc.) — unchanged, now `alloc`-gated.

### What changes in examples

- `alpine_strace`: construct `Emulator<_, _, StraceTracer>`, access pending syscalls via `emu.instrumentation_mut().pending.take()`.
- `shellcode_trace`: construct `Emulator<_, _, TraceInstr>`, no `peek_pending_syscall` ceremony.
- `alpine_direct`: uses closures (`hook_add_code`) — unchanged.

## Non-goals

- **Runtime tracer swapping** — out of scope. Fixed at construction.
- **Plugin/DLL-loaded tracers** — would need `dyn`. Not planned.
- **Tuple arity > 8** — diminishing returns. 8 covers all practical cases.
- **Dropping `alloc` from closure hooks now** — future work. Spec captures the path but implementation is deferred.
