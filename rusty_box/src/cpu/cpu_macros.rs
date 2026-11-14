#![allow(dead_code)]
/// EFLAGS bit accessors (Bochs cpu.h `IMPLEMENT_EFLAG_ACCESSOR` and
/// `IMPLEMENT_EFLAG_SET_ACCESSOR`).
///
/// `impl_eflag!(name, bit)` writes the bit alone. `impl_eflag!(name, bit,
/// then hook)` calls `self.hook()` after every write, for the flags whose
/// Bochs setters do (`IMPLEMENT_EFLAG_SET_ACCESSOR_AC` and `_IF`).
/// `impl_eflag!(name, bit, get_only)` leaves the writers to a hand-written
/// impl, for the flags whose Bochs setters do more than that.
#[macro_export]
macro_rules! impl_eflag {
    ($name:ident, $bit:literal,get_only) => {
        paste::paste! {
            impl<'c, T: $crate::cpu::instrumentation::Instrumentation> $crate::cpu::BxCpuC<T> {
                #[inline]
                pub(crate) fn [<get_ $name>](&self) -> u32 {
                    self.eflags.bits() & (1 << $bit)
                }

                #[inline]
                pub(crate) fn [<get_b_ $name>](&self) -> u32 {
                    (self.eflags.bits() >> $bit) & 1
                }
            }
        }
    };
    ($name:ident, $bit:literal,then $hook:ident) => {
        $crate::impl_eflag!($name, $bit, get_only);
        paste::paste! {
            impl<'c, T: $crate::cpu::instrumentation::Instrumentation> $crate::cpu::BxCpuC<T> {
                #[inline]
                pub(crate) fn [<assert_ $name>](&mut self) {
                    self.eflags = $crate::cpu::eflags::EFlags::from_bits_retain(
                        self.eflags.bits() | (1 << $bit)
                    );
                    self.$hook();
                }

                #[inline]
                pub(crate) fn [<clear_ $name>](&mut self) {
                    self.eflags = $crate::cpu::eflags::EFlags::from_bits_retain(
                        self.eflags.bits() & !(1 << $bit)
                    );
                    self.$hook();
                }

                #[inline]
                pub(crate) fn [<set_ $name>](&mut self, val: bool) {
                    let mask = 1 << $bit;
                    self.eflags = $crate::cpu::eflags::EFlags::from_bits_retain(
                        (self.eflags.bits() & !mask) | ((val as u32) << $bit)
                    );
                    self.$hook();
                }
            }
        }
    };
    ($name:ident, $bit:literal) => {
        $crate::impl_eflag!($name, $bit, get_only);
        paste::paste! {
            impl<'c, T: $crate::cpu::instrumentation::Instrumentation> $crate::cpu::BxCpuC<T> {
                #[inline]
                pub(crate) fn [<assert_ $name>](&mut self) {
                    self.eflags = $crate::cpu::eflags::EFlags::from_bits_retain(
                        self.eflags.bits() | (1 << $bit)
                    );
                }

                #[inline]
                pub(crate) fn [<clear_ $name>](&mut self) {
                    self.eflags = $crate::cpu::eflags::EFlags::from_bits_retain(
                        self.eflags.bits() & !(1 << $bit)
                    );
                }

                #[inline]
                pub(crate) fn [<set_ $name>](&mut self, val: bool) {
                    let mask = 1 << $bit;
                    self.eflags = $crate::cpu::eflags::EFlags::from_bits_retain(
                        (self.eflags.bits() & !mask) | ((val as u32) << $bit)
                    );
                }
            }
        }
    };
}
