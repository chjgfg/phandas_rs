//! 中性化：向量投影与回归残差。

use super::constants::{EPSILON, MATRIX_COND_THRESHOLD};
use super::core::Factor;
use super::linalg::{cond_sym, gram, ols};
use super::numeric::{nanmean, nanstd};

impl Factor {
    // ========================================================================
    // 中性化
    // ========================================================================

    /// 向量投影中性化：逐期剔除自身在 `other` 方向上的分量，对应 Python 侧 `vector_neut()`。
    /// 有效配对少于 2 个、或 `other` 当期几乎无波动时，原值原样返回。
    ///
    /// - 入参：`other` 要剔除的风格因子（如成交量排名、市值排名）。
    /// - 加工：对齐两者索引 → 逐期取双方都有效的位置 → 若 `other` 的总体标准差
    ///   小于 `max(1e-10, 1e-4 × |均值|)` 或其模长平方小于 `1e-10` 则跳过该期
    ///   → 否则算投影系数 `⟨x, y⟩ / ⟨y, y⟩`，从 `x` 中减去 `系数 × y`。
    /// - 出参：同交集形状的因子，每期与 `other` 正交，名形如
    ///   `vector_neut(rank(close),rank(volume))`。跳过的期与无效位置保留原值。
    pub fn vector_neut(&self, other: &Factor) -> Factor {
        let (timestamps, symbols, xs, ys) = self.align(other);
        let n = symbols.len();
        let mut values = xs.clone();
        for ti in 0..timestamps.len() {
            let x = &xs[ti * n..(ti + 1) * n];
            let y = &ys[ti * n..(ti + 1) * n];
            let valid: Vec<usize> = (0..n).filter(|&j| !x[j].is_nan() && !y[j].is_nan()).collect();
            if valid.len() < 2 {
                continue;
            }
            let yv: Vec<f64> = valid.iter().map(|&j| y[j]).collect();
            let y_std = nanstd(&yv, 0);
            let y_mean_abs = nanmean(&yv).abs();
            if y_std < EPSILON.max(1e-4 * y_mean_abs) {
                continue;
            }
            let y_norm_sq: f64 = yv.iter().map(|v| v * v).sum();
            if y_norm_sq < EPSILON {
                continue;
            }
            let xy_dot: f64 = valid.iter().map(|&j| x[j] * y[j]).sum();
            let coef = xy_dot / y_norm_sq;
            for &j in &valid {
                values[ti * n + j] = x[j] - coef * y[j];
            }
        }
        Factor {
            name: format!("vector_neut({},{})", self.name, other.name),
            timestamps,
            symbols,
            values,
        }
    }

    /// 回归中性化：逐期对给定因子做带截距的最小二乘回归并取残差。
    /// 对应 Python 侧 `regression_neut()`。有效样本不足或设计矩阵病态时该期输出 NaN。
    ///
    /// - 入参：`neut_factors` 一个或多个要中性化掉的自变量因子。
    /// - 加工：把自身与全部自变量对齐到共同索引 → 逐期挑出所有变量都有效的标的
    ///   → 样本少于 2 个则该期 NaN → 拼出含截距列的设计矩阵，条件数超限（`cond(X) > 1e10`）
    ///   则该期 NaN → 否则解最小二乘，用 `实际值 − 拟合值` 填回有效位置。
    /// - 出参：残差因子，与全部自变量及截距正交，名形如 `regression_neut(close,[volume])`；
    ///   自变量列表为空时返回全 NaN。与 [`Factor::vector_neut`] 的区别是它带截距、
    ///   且支持多个自变量。
    pub fn regression_neut(&self, neut_factors: &[&Factor]) -> Factor {
        let names: Vec<String> = neut_factors.iter().map(|f| f.name.clone()).collect();
        let name = format!("regression_neut({},[{}])", self.name, names.join(","));

        if neut_factors.is_empty() {
            return self.like(vec![f64::NAN; self.values.len()], name);
        }
        // 逐个对齐，得到共同索引下的 Y 与各 X
        let mut timestamps = self.timestamps.clone();
        let mut symbols = self.symbols.clone();
        for f in neut_factors {
            let (ts, sy, _, _) = Factor {
                name: String::new(),
                timestamps: timestamps.clone(),
                symbols: symbols.clone(),
                values: vec![f64::NAN; timestamps.len() * symbols.len()],
            }
            .align(f);
            timestamps = ts;
            symbols = sy;
        }
        let y_all = self.reindex(&timestamps, &symbols);
        let x_all: Vec<Vec<f64>> = neut_factors
            .iter()
            .map(|f| f.reindex(&timestamps, &symbols))
            .collect();

        let n = symbols.len();
        let m = x_all.len();
        let mut values = vec![f64::NAN; timestamps.len() * n];
        for ti in 0..timestamps.len() {
            let base = ti * n;
            let valid: Vec<usize> = (0..n)
                .filter(|&j| {
                    !y_all[base + j].is_nan() && x_all.iter().all(|x| !x[base + j].is_nan())
                })
                .collect();
            if valid.len() < 2 {
                continue;
            }
            let design: Vec<Vec<f64>> = valid
                .iter()
                .map(|&j| {
                    let mut r = Vec::with_capacity(m + 1);
                    r.push(1.0);
                    r.extend(x_all.iter().map(|x| x[base + j]));
                    r
                })
                .collect();
            // Python 侧判据为 cond(X) > 1e10，等价于 cond(XᵗX) > 1e20
            if cond_sym(&gram(&design)) > MATRIX_COND_THRESHOLD * MATRIX_COND_THRESHOLD {
                continue;
            }

            let yv: Vec<f64> = valid.iter().map(|&j| y_all[base + j]).collect();
            let Some(params) = ols(&design, &yv) else {
                continue;
            };
            for (k, &j) in valid.iter().enumerate() {
                let fitted: f64 = design[k].iter().zip(params.iter()).map(|(a, b)| a * b).sum();
                values[base + j] = yv[k] - fitted;
            }
        }
        Factor { name, timestamps, symbols, values }
    }
}
