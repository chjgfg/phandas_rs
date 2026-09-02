//! 运算符重载：支持 `&a + &b`、`&a * 2.0` 等写法。

use std::ops::{Add, Div, Mul, Neg, Sub};

use super::core::Factor;

/// 为四则运算生成 4 组 `impl`：左值可借用或拥有，右值可为因子引用或标量。
///
/// - 入参（宏参数）：`$trait` 运算符 trait；`$method` trait 方法名；
///   `$inner` 转发到的 [`Factor`] 方法名。
/// - 加工：每个 `impl` 只做一次转发，不额外计算。
/// - 出参：新 [`Factor`]，入参 / 加工 / 出参语义完全等同于所转发的方法。
macro_rules! impl_factor_op {
    ($trait:ident, $method:ident, $inner:ident) => {
        impl $trait<&Factor> for &Factor {
            type Output = Factor;
            fn $method(self, rhs: &Factor) -> Factor {
                Factor::$inner(self, rhs)
            }
        }

        impl $trait<f64> for &Factor {
            type Output = Factor;
            fn $method(self, rhs: f64) -> Factor {
                Factor::$inner(self, rhs)
            }
        }

        impl $trait<&Factor> for Factor {
            type Output = Factor;
            fn $method(self, rhs: &Factor) -> Factor {
                Factor::$inner(&self, rhs)
            }
        }

        impl $trait<f64> for Factor {
            type Output = Factor;
            fn $method(self, rhs: f64) -> Factor {
                Factor::$inner(&self, rhs)
            }
        }
    };
}

// `&a + &b` / `a + 2.0` → 见 [`Factor::add`]
impl_factor_op!(Add, add, add);
// `&a - &b` / `a - 2.0` → 见 [`Factor::subtract`]
impl_factor_op!(Sub, sub, subtract);
// `&a * &b` / `a * 2.0` → 见 [`Factor::multiply`]
impl_factor_op!(Mul, mul, multiply);
// `&a / &b` / `a / 2.0` → 见 [`Factor::divide`]
impl_factor_op!(Div, div, divide);

impl Neg for &Factor {
    type Output = Factor;

    /// 取负：无额外入参，逐格乘 `-1`，出参见 [`Factor::reverse`]。
    fn neg(self) -> Factor {
        self.reverse()
    }
}

impl Neg for Factor {
    type Output = Factor;

    /// 取负（拥有所有权版本）：语义同 [`Factor::reverse`]。
    fn neg(self) -> Factor {
        self.reverse()
    }
}
