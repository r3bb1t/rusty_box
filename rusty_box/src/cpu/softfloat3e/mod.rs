pub(crate) mod softfloat_types;
pub(crate) mod softfloat;
pub(crate) mod specialize;
pub(crate) mod primitives;
pub(crate) mod internals;

// Phase 1: ExtFloat80 operations (matching Bochs softfloat3e/ file layout)
pub(crate) mod extf80_addsub;
pub(crate) mod extf80_mul;
pub(crate) mod extf80_div;
pub(crate) mod extf80_sqrt;
pub(crate) mod extf80_compare;
pub(crate) mod extf80_to_f32;
pub(crate) mod extf80_to_f64;
pub(crate) mod extf80_to_i16;
pub(crate) mod extf80_to_i32;
pub(crate) mod extf80_to_i64;
pub(crate) mod extf80_roundToInt;
pub(crate) mod extf80_class;
pub(crate) mod extf80_scale;
pub(crate) mod f32_to_extf80;
pub(crate) mod f64_to_extf80;
pub(crate) mod i32_to_extf80;
pub(crate) mod i64_to_extf80;
pub(crate) mod f128;
