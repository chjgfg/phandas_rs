//! Factor 的可读渲染：单行摘要与矩阵表格。

use std::fmt;

use super::core::Factor;

impl fmt::Display for Factor {
    /// - 入参：`f` 格式化器。
    /// - 加工：拼接因子名、期数、标的数与时间范围成单行摘要。
    /// - 出参：形如 `Factor('rank(close)', 957 periods × 6 symbols, ...)` 的一行文本。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let range = match (self.timestamps.first(), self.timestamps.last()) {
            (Some(a), Some(b)) => format!("{a} to {b}"),
            _ => "empty".to_string(),
        };

        write!(
            f,
            "Factor('{}', {} periods × {} symbols, {})",
            self.name,
            self.timestamps.len(),
            self.symbols.len(),
            range
        )
    }
}

impl Factor {
    /// 渲染为矩阵表格（行 = timestamp，列 = symbol）。
    /// 期数超过 `max_rows` 时只显示首尾各半并以 `...` 省略中间，对应 Python 侧 `show()`。
    ///
    /// - 入参：`max_rows` 最多完整显示的期数。
    /// - 加工：先输出因子摘要行与列头 → 期数不超过上限时逐期渲染，超过则渲染首尾各
    ///   `max_rows / 2` 期、中间插一行 `...`。数值保留 4 位小数，NaN 显示为 `NaN`。
    /// - 出参：多行可打印字符串（不直接输出，由调用方决定去向）。
    pub fn show(&self, max_rows: usize) -> String {
        let t = self.timestamps.len();
        let mut out = format!("{self}\n");
        let width = 12;
        out.push_str(&format!("{:<19}", "timestamp"));
        for s in &self.symbols {
            out.push_str(&format!("{s:>width$}"));
        }
        out.push('\n');

        let render = |out: &mut String, ti: usize| {
            out.push_str(&format!("{:<19}", self.timestamps[ti]));
            for si in 0..self.symbols.len() {
                let v = self.at(ti, si);
                if v.is_nan() {
                    out.push_str(&format!("{:>width$}", "NaN"));
                } else {
                    out.push_str(&format!("{v:>width$.4}"));
                }
            }
            out.push('\n');
        };

        if t <= max_rows {
            for ti in 0..t {
                render(&mut out, ti);
            }
        } else {
            let half = max_rows / 2;
            for ti in 0..half {
                render(&mut out, ti);
            }
            out.push_str(&format!("{:<19}...\n", ""));
            for ti in (t - half)..t {
                render(&mut out, ti);
            }
        }
        out
    }
}
