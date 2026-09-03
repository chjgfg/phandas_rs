//! 一元数学变换与二元运算（含比较），均为逐元素算子。

use super::constants::EPSILON;
use super::core::{Factor, Operand};
use super::numeric::{clean_inf, fmt_num, truthy};

impl Factor {
    // ========================================================================
    // 一元数学变换（逐元素）
    // ========================================================================

    /// 绝对值。
    ///
    /// - 入参：无。
    /// - 加工：逐格取绝对值，NaN 保持 NaN。
    /// - 出参：同形状因子，名为 `abs(<原名>)`。
    pub fn abs(&self) -> Factor {
        self.map_values(format!("abs({})", self.name), f64::abs)
    }

    /// 符号函数：正 `1`、负 `-1`、零 `0`，NaN 保持 NaN。对应 `np.sign`。
    ///
    /// - 入参：无。
    /// - 加工：逐格判正负零（注意与 Rust 的 `signum` 不同——这里 `0.0` 得 `0` 而非 `1`）。
    /// - 出参：同形状因子，取值只有 `-1 / 0 / 1`，用于剥离幅度只留方向。
    pub fn sign(&self) -> Factor {
        self.map_values(format!("sign({})", self.name), |x| {
            if x.is_nan() {
                f64::NAN
            } else if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        })
    }

    /// 倒数，零值输出 NaN。
    ///
    /// - 入参：无。
    /// - 加工：逐格算 `1 / x`，`x` 恰为 0 时输出 NaN（避免 `±inf`）。
    /// - 出参：同形状因子，名为 `inverse(<原名>)`，可把"越大越好"翻成"越小越好"。
    pub fn inverse(&self) -> Factor {
        self.map_values(format!("inverse({})", self.name), |x| {
            if x != 0.0 {
                1.0 / x
            } else {
                f64::NAN
            }
        })
    }

    /// 对数，非正值输出 NaN。`base = None` 时为自然对数。
    ///
    /// - 入参：`base` 对数底数；`None` 表示自然底 `e`。
    /// - 加工：逐格判断 `x > 0`，否则 NaN；给定底数时算 `ln(x) / ln(base)`。
    /// - 出参：`Ok(Factor)`，名为 `log(<原名>)` 或 `log(<原名>,base=2)`；
    ///   底数非正或等于 1 时返回 `Err`。常用于压缩成交量、市值这类长尾量。
    pub fn log(&self, base: Option<f64>) -> Result<Factor, String> {
        match base {
            None => Ok(self.map_values(format!("log({})", self.name), |x| {
                if x > 0.0 {
                    x.ln()
                } else {
                    f64::NAN
                }
            })),
            Some(b) => {
                if b <= 0.0 || b == 1.0 {
                    return Err(format!(
                        "Invalid log base: {}. Base must be positive and not equal to 1.",
                        fmt_num(b)
                    ));
                }
                let name = format!("log({},base={})", self.name, fmt_num(b));
                Ok(self.map_values(name, move |x| {
                    if x > 0.0 {
                        x.ln() / b.ln()
                    } else {
                        f64::NAN
                    }
                }))
            }
        }
    }

    /// 自然对数，等价于 `log(None)`。
    ///
    /// - 入参：无。
    /// - 加工：逐格算 `ln(x)`，非正值输出 NaN。
    /// - 出参：同形状因子，名为 `log(<原名>)`。
    pub fn ln(&self) -> Factor {
        self.log(None).expect("自然对数无需校验底数")
    }

    /// 平方根，负值输出 NaN。
    ///
    /// - 入参：无。
    /// - 加工：逐格判断 `x >= 0` 后开平方，负值输出 NaN。
    /// - 出参：同形状因子，名为 `sqrt(<原名>)`；比对数更温和的压缩变换。
    pub fn sqrt(&self) -> Factor {
        self.map_values(format!("sqrt({})", self.name), |x| {
            if x >= 0.0 {
                x.sqrt()
            } else {
                f64::NAN
            }
        })
    }

    /// 保号对数：`sign(x) · ln(1 + |x|)`，压缩量纲同时保留方向。
    ///
    /// - 入参：无。
    /// - 加工：逐格取符号，乘上 `|x|` 的 `ln(1 + ·)`。
    /// - 出参：同形状因子，名为 `s_log_1p(<原名>)`。相比 [`Factor::ln`]，
    ///   它接受负值和 0，适合压缩收益率、资金流这类可正可负的量。
    pub fn s_log_1p(&self) -> Factor {
        self.map_values(format!("s_log_1p({})", self.name), |x| {
            x.signum() * x.abs().ln_1p()
        })
    }

    // ========================================================================
    // 二元运算（因子或标量）
    // ========================================================================

    /// 幂运算，`±inf` 结果归一为 NaN。
    ///
    /// - 入参：`exponent` 指数，可是因子（逐格对应）或标量。
    /// - 加工：对齐（若为因子）后逐格算 `x^e`，溢出成 `±inf` 的结果归一为 NaN。
    /// - 出参：新因子，名形如 `(close**2)`。注意负底数配非整数指数会得 NaN。
    pub fn power<'a>(&self, exponent: impl Into<Operand<'a>>) -> Factor {
        let me = self.name.clone();
        self.combine(
            exponent,
            |rhs| format!("({me}**{rhs})"),
            |x, e| clean_inf(x.powf(e)),
        )
    }

    /// 保号幂运算：`sign(x) · |x|^e`，`±inf` 结果归一为 NaN。
    ///
    /// - 入参：`exponent` 指数，可是因子或标量。
    /// - 加工：先取符号，再对绝对值求幂，最后把符号乘回；`±inf` 归一为 NaN。
    /// - 出参：新因子，名形如 `signed_power(close,0.5)`。相比 [`Factor::power`]，
    ///   负值也能安全做分数次幂——常用于压缩极端值又不丢方向。
    pub fn signed_power<'a>(&self, exponent: impl Into<Operand<'a>>) -> Factor {
        let me = self.name.clone();
        self.combine(
            exponent,
            |rhs| format!("signed_power({me},{rhs})"),
            |x, e| {
                let s = if x.is_nan() {
                    f64::NAN
                } else if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                };
                clean_inf(s * x.abs().powf(e))
            },
        )
    }

    /// 加法。
    ///
    /// - 入参：`other` 右操作数，可是因子（按 (时间, 标的) 取交集）或标量（广播）。
    /// - 加工：对齐后逐格相加；任一侧为 NaN 则结果 NaN。
    /// - 出参：新因子，名形如 `(close+open)`。等价于运算符 `&a + &b`。
    pub fn add<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.infix(other, "+", |x, y| x + y)
    }

    /// 减法。
    ///
    /// - 入参：`other` 右操作数，可是因子或标量。
    /// - 加工：对齐后逐格相减；任一侧为 NaN 则结果 NaN。
    /// - 出参：新因子，名形如 `(close-open)`。等价于运算符 `&a - &b`。
    pub fn subtract<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.infix(other, "-", |x, y| x - y)
    }

    /// 乘法。
    ///
    /// - 入参：`other` 右操作数，可是因子或标量。
    /// - 加工：对齐后逐格相乘；任一侧为 NaN 则结果 NaN。
    /// - 出参：新因子，名形如 `(close*2)`。等价于运算符 `&a * &b`。
    pub fn multiply<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.infix(other, "*", |x, y| x * y)
    }

    /// 除法：除数为因子时，`|除数| ≤ 1e-10` 输出 NaN；除数为标量 0 时整体输出 NaN。
    /// 与 Python 侧 `__truediv__` 的 `safe_div` 一致。
    ///
    /// - 入参：`other` 除数，可是因子或标量。
    /// - 加工：因子除数——逐格检查 `|y| > 1e-10` 才相除，否则该格 NaN；
    ///   标量除数——为 0 时整体 NaN，否则逐格相除。
    /// - 出参：新因子，名形如 `(close/ts_delay(close,20))`。等价于运算符 `&a / &b`，
    ///   不会产生 `±inf`。
    pub fn divide<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        let operand = other.into();
        let me = self.name.clone();
        match operand {
            Operand::Factor(_) => self.combine(
                operand,
                |rhs| format!("({me}/{rhs})"),
                |x, y| if y.abs() > EPSILON { x / y } else { f64::NAN },
            ),
            Operand::Scalar(v) => self.combine(
                operand,
                |rhs| format!("({me}/{rhs})"),
                move |x, _| if v == 0.0 { f64::NAN } else { x / v },
            ),
        }
    }

    /// 逐元素取较大者，任一为 NaN 则输出 NaN（对应 `np.maximum`）。
    ///
    /// - 入参：`other` 比较对象，可是因子或标量。
    /// - 加工：对齐后逐格比较；**任一侧为 NaN 就输出 NaN**（不同于"跳过 NaN"的语义）。
    /// - 出参：新因子，名形如 `max(close,0)`。常用来做下限截断。
    pub fn maximum<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        let me = self.name.clone();
        self.combine(
            other,
            |rhs| format!("max({me},{rhs})"),
            |x, y| {
                if x.is_nan() || y.is_nan() {
                    f64::NAN
                } else {
                    x.max(y)
                }
            },
        )
    }

    /// 逐元素取较小者，任一为 NaN 则输出 NaN（对应 `np.minimum`）。
    ///
    /// - 入参：`other` 比较对象，可是因子或标量。
    /// - 加工：对齐后逐格比较；任一侧为 NaN 就输出 NaN。
    /// - 出参：新因子，名形如 `min(close,100)`。常用来做上限截断。
    pub fn minimum<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        let me = self.name.clone();
        self.combine(
            other,
            |rhs| format!("min({me},{rhs})"),
            |x, y| {
                if x.is_nan() || y.is_nan() {
                    f64::NAN
                } else {
                    x.min(y)
                }
            },
        )
    }

    /// 取负，等价于 `multiply(-1.0)`。对应 Python 侧 `reverse()`。
    ///
    /// - 入参：无。
    /// - 加工：逐格乘 `-1`。
    /// - 出参：新因子，名形如 `(close*-1)`。等价于运算符 `-&a`；
    ///   用于把因子方向翻转（"越小越好"变"越大越好"）。
    pub fn reverse(&self) -> Factor {
        self.multiply(-1.0)
    }

    /// 条件选择：`cond` 为真处取自身，否则取 `other`。
    /// `cond` 中 NaN 视为假，非零视为真。对应 Python 侧 `where()`。
    ///
    /// - 入参：`cond` 条件因子（NaN 当假、非零当真）；`other` 条件为假时的取值，可是因子或标量。
    /// - 加工：把条件与备选都对齐到同一索引 → 逐格按条件二选一。
    /// - 出参：新因子，名形如 `where(close)`。用于按条件屏蔽或替换部分取值。
    pub fn where_cond<'a>(&self, cond: &Factor, other: impl Into<Operand<'a>>) -> Factor {
        let name = format!("where({})", self.name);
        match other.into() {
            Operand::Factor(o) => {
                let (timestamps, symbols, xs, os) = self.align(o);
                let cs = cond.reindex(&timestamps, &symbols);
                let values = (0..xs.len())
                    .map(|i| if truthy(cs[i]) { xs[i] } else { os[i] })
                    .collect();
                Factor { name, timestamps, symbols, values }
            }
            Operand::Scalar(v) => {
                let (timestamps, symbols, xs, cs) = self.align(cond);
                let values = (0..xs.len())
                    .map(|i| if truthy(cs[i]) { xs[i] } else { v })
                    .collect();
                Factor { name, timestamps, symbols, values }
            }
        }
    }

    /// 比较运算通用实现：结果为 `1.0` / `0.0`，涉及 NaN 的比较取 `0.0`。
    ///
    /// - 入参：`other` 比较对象；`op` 符号文本；`f` 返回布尔的比较闭包。
    /// - 加工：套用 [`Factor::combine`]，把布尔结果编码成 `1.0` / `0.0`。
    ///   NaN 参与的比较在 IEEE 语义下恒为假，故输出 `0.0`。
    /// - 出参：只含 `1.0` / `0.0` 的新因子，可直接当 [`Factor::where_cond`] 的条件。
    fn compare<'a>(&self, other: impl Into<Operand<'a>>, op: &str, f: impl Fn(f64, f64) -> bool) -> Factor {
        let me = self.name.clone();
        self.combine(
            other,
            |rhs| format!("({me}{op}{rhs})"),
            move |x, y| if f(x, y) { 1.0 } else { 0.0 },
        )
    }

    /// 小于比较，输出 `1.0` / `0.0`。
    ///
    /// - 入参：`other` 比较对象，可是因子或标量。
    /// - 加工：逐格判断 `x < y`。
    /// - 出参：条件因子，名形如 `(close<15)`。
    pub fn lt<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.compare(other, "<", |x, y| x < y)
    }

    /// 小于等于比较。
    ///
    /// - 入参：`other` 比较对象，可是因子或标量。
    /// - 加工：逐格判断 `x <= y`。
    /// - 出参：条件因子，名形如 `(close<=15)`。
    pub fn le<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.compare(other, "<=", |x, y| x <= y)
    }

    /// 大于比较。
    ///
    /// - 入参：`other` 比较对象，可是因子或标量。
    /// - 加工：逐格判断 `x > y`。
    /// - 出参：条件因子，名形如 `(close>15)`。
    pub fn gt<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.compare(other, ">", |x, y| x > y)
    }

    /// 大于等于比较。
    ///
    /// - 入参：`other` 比较对象，可是因子或标量。
    /// - 加工：逐格判断 `x >= y`。
    /// - 出参：条件因子，名形如 `(close>=15)`。
    pub fn ge<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.compare(other, ">=", |x, y| x >= y)
    }

    /// 相等比较（数值意义，非 `PartialEq`）。
    ///
    /// - 入参：`other` 比较对象，可是因子或标量。
    /// - 加工：逐格判断 `x == y`；NaN 与任何值都不相等，故 NaN 处输出 `0.0`。
    /// - 出参：条件因子，名形如 `(close==10)`。命名带 `_val` 后缀以避免与 `PartialEq::eq` 冲突。
    pub fn eq_val<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.compare(other, "==", |x, y| x == y)
    }

    /// 不等比较（数值意义）。NaN 与任何值都"不等"，与 pandas 一致。
    ///
    /// - 入参：`other` 比较对象，可是因子或标量。
    /// - 加工：逐格判断 `x != y`；含 NaN 时按 IEEE 语义判为"不等"，输出 `1.0`。
    /// - 出参：条件因子，名形如 `(close!=10)`。
    pub fn ne_val<'a>(&self, other: impl Into<Operand<'a>>) -> Factor {
        self.compare(other, "!=", |x, y| x != y)
    }

    // ========================================================================
    // 标量在左的二元运算，对应 Python 侧 `__rsub__` / `__rtruediv__` / `__rpow__`
    //
    // Python 只要在类上定义反射运算符，`2 - close` 就成立：`int.__sub__` 返回
    // NotImplemented 后，解释器会回头调用 `Factor.__rsub__`。Rust 没有这套自动分派，
    // 需要在 ops.rs 里为 `f64` 显式实现 Sub / Div。
    // 加法与乘法可交换，直接转发到 add / multiply，因子名也随之写成 `(close+2)`——
    // 与 Python 的 `__radd__` / `__rmul__` 转发给正向方法后的命名一致。
    // ========================================================================

    /// 标量在左的逐元素运算通用实现。
    ///
    /// - 入参：`lhs` 位于左侧的标量；`op` 中缀符号文本；`f` 二元运算闭包（第一个参数收 `lhs`）。
    /// - 加工：逐格调用 `f(lhs, x)`；右侧只有自身一个因子，无需对齐。
    /// - 出参：形状与索引不变的新因子，名形如 `(2-close)`。
    pub(crate) fn scalar_infix(&self, lhs: f64, op: &str, f: impl Fn(f64, f64) -> f64) -> Factor {
        let name = format!("({}{}{})", fmt_num(lhs), op, self.name);
        self.map_values(name, move |x| f(lhs, x))
    }

    /// 标量减因子：`lhs - self`，对应 Python 侧 `__rsub__`。
    ///
    /// - 入参：`lhs` 被减数标量。
    /// - 加工：逐格算 `lhs - x`，NaN 照常传播。
    /// - 出参：新因子，名形如 `(1-rank(close))`。等价于运算符 `1.0 - &f`，
    ///   常用于翻转已归一到 `(0, 1]` 的因子方向。
    pub fn scalar_sub(&self, lhs: f64) -> Factor {
        self.scalar_infix(lhs, "-", |a, b| a - b)
    }

    /// 标量为底的幂：`base ^ self`，`±inf` 结果归一为 NaN。对应 Python 侧 `__rpow__`。
    ///
    /// - 入参：`base` 位于底数位置的标量。
    /// - 加工：逐格算 `base^x`，溢出成 `±inf` 的结果归一为 NaN（与 [`Factor::power`] 一致）。
    /// - 出参：新因子，名形如 `(2**ts_zscore(close,30))`。
    ///   Rust 没有 `**` 运算符，故这一个只提供方法形式。
    pub fn scalar_power(&self, base: f64) -> Factor {
        self.scalar_infix(base, "**", |b, e| clean_inf(b.powf(e)))
    }
    /// 标量除因子：`lhs / self`，自身**恰为** 0 处输出 NaN。对应 Python 侧 `__rtruediv__`。
    ///
    /// - 入参：`lhs` 被除数标量。
    /// - 加工：逐格检查 `x != 0` 才相除，否则该格 NaN。
    /// - 出参：新因子，名形如 `(2/close)`。等价于运算符 `2.0 / &f`。
    ///
    /// 除零判据与 [`Factor::divide`] 不同：那里因子作除数时用 `|y| > 1e-10`，这里是精确等零。
    /// 这是如实复刻 Python 侧 `__rtruediv__` 与 `__truediv__` 的不一致，
    /// 后果是 `2.0 / &f` 在 `f` 极小但非零处会得到极大值（极端情况下 `±inf`，
    /// 上游此处也未做 inf 清理），而 `&g / &f` 在同一位置会得 NaN。
    pub fn scalar_div(&self, lhs: f64) -> Factor {
        self.scalar_infix(lhs, "/", |a, b| if b != 0.0 { a / b } else { f64::NAN })
    }
}
