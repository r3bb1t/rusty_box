#![allow(dead_code)]
//! Polynomial evaluation using Float128 arithmetic.
//! Ported from Bochs cpu/fpu/poly.cc.
//!
//! Provides EvalPoly, EvenPoly, OddPoly for use by transcendental functions.

use super::super::softfloat3e::f128::*;
use super::super::softfloat3e::softfloat::SoftFloatStatus;

//                            2         3         4               n
// f(x) ~ C + (C * x) + (C * x) + (C * x) + (C * x) + ... + (C * x)
//         0    1         2         3         4               n
//
//          --       2k                --        2k+1
//   p(x) = >  C  * x           q(x) = >  C   * x
//          --  2k                     --  2k+1
//
//   f(x) ~ [ p(x) + x * q(x) ]

pub fn eval_poly(x: Float128, arr: &[Float128], status: &mut SoftFloatStatus) -> Float128 {
    let mut n = arr.len();
    n -= 1;
    let mut r = arr[n];

    while n > 0 {
        n -= 1;
        r = f128_mul_add(r, x, arr[n], 0, status);
    }

    r
}

//                  2         4         6         8               2n
// f(x) ~ C + (C * x) + (C * x) + (C * x) + (C * x) + ... + (C * x)
//         0    1         2         3         4               n
//
//          --       4k                --        4k+2
//   p(x) = >  C  * x           q(x) = >  C   * x
//          --  2k                     --  2k+1
//
//                    2
//   f(x) ~ [ p(x) + x * q(x) ]

pub fn even_poly(x: Float128, arr: &[Float128], status: &mut SoftFloatStatus) -> Float128 {
    let x2 = f128_mul(x, x, status);
    eval_poly(x2, arr, status)
}

//                        3         5         7         9               2n+1
// f(x) ~ (C * x) + (C * x) + (C * x) + (C * x) + (C * x) + ... + (C * x)
//          0         1         2         3         4               n
//                        2         4         6         8               2n
//      = x * [ C + (C * x) + (C * x) + (C * x) + (C * x) + ... + (C * x)
//               0    1         2         3         4               n
//
//          --       4k                --        4k+2
//   p(x) = >  C  * x           q(x) = >  C   * x
//          --  2k                     --  2k+1
//
//                        2
//   f(x) ~ x * [ p(x) + x * q(x) ]

pub fn odd_poly(x: Float128, arr: &[Float128], status: &mut SoftFloatStatus) -> Float128 {
    let ep = even_poly(x, arr, status);
    f128_mul(x, ep, status)
}
