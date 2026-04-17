#![allow(dead_code)]
#[macro_export]
macro_rules! impl_eflag {
    ($name:ident, $bit:literal) => {
        paste::paste! {
            impl<'c, I: $crate::cpu::cpuid::BxCpuIdTrait, T: $crate::cpu::instrumentation::Instrumentation> $crate::cpu::BxCpuC<'c, I, T> {
                #[inline]
                pub(crate) fn [<get_ $name>](&self) -> u32 {
                    self.eflags.bits() & (1 << $bit)
                }

                #[inline]
                pub(crate) fn [<get_b_ $name>](&self) -> u32 {
                    (self.eflags.bits() >> $bit) & 1
                }

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
