//! 运算符重载：支持 `&a + &b`、`&a * 2.0`、`2.0 - &a` 等写法。

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

/// 为"标量在左"的运算生成 2 组 `impl`：右值可为因子引用或拥有的因子。
///
/// - 入参（宏参数）：`$trait` 运算符 trait；`$method` trait 方法名；
///   `$inner` 转发到的 [`Factor`] 方法名（接收者为因子，参数为标量）。
/// - 加工：每个 `impl` 只做一次转发，不额外计算。
/// - 出参：新 [`Factor`]，语义完全等同于所转发的方法。
///
/// 之所以能给外部类型 `f64` 实现外部 trait：`&` 被标记为 `#[fundamental]`，
/// `&Factor` 与 `Factor` 都算本 crate 的本地类型，孤儿规则因此放行。
/// 反过来说，下游 crate 无法自行补这几个 impl，只能由本 crate 提供——
/// 这也是 Python 侧必须定义反射运算符的同一个道理：改不了 `f64` / `int` 本身。
macro_rules! impl_scalar_op {
    ($trait:ident, $method:ident, $inner:ident) => {
        impl $trait<&Factor> for f64 {
            type Output = Factor;
            fn $method(self, rhs: &Factor) -> Factor {
                Factor::$inner(rhs, self)
            }
        }

        impl $trait<Factor> for f64 {
            type Output = Factor;
            fn $method(self, rhs: Factor) -> Factor {
                Factor::$inner(&rhs, self)
            }
        }
    };
}

// `2.0 + &a` / `2.0 + a`：加法可交换，转发到 [`Factor::add`]，结果名为 `(a+2)`
impl_scalar_op!(Add, add, add);
// `2.0 * &a` / `2.0 * a`：乘法可交换，转发到 [`Factor::multiply`]，结果名为 `(a*2)`
impl_scalar_op!(Mul, mul, multiply);
// `2.0 - &a` / `2.0 - a` → 见 [`Factor::scalar_sub`]，结果名为 `(2-a)`
impl_scalar_op!(Sub, sub, scalar_sub);
// `2.0 / &a` / `2.0 / a` → 见 [`Factor::scalar_div`]，结果名为 `(2/a)`
impl_scalar_op!(Div, div, scalar_div);

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
