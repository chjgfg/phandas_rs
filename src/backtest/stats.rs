//! 收益率序列统计：口径逐项对齐 pandas 与 scipy，供绩效指标计算使用。
//!
//! 均值与标准差直接复用 [`crate::factor::numeric`] 里跳过 NaN 的实现（`nanstd(v, 1)`
//! 即 pandas 默认的 `ddof = 1`），本文件只补 pandas / scipy 特有口径的四项：
//! 线性插值分位数、Pearson 相关系数、有偏偏度、有偏 Pearson 峰度。

use crate::factor::numeric::nanmean;

/// 分位数，对齐 `pandas.Series.quantile(q)` 的默认线性插值。
///
/// - 入参：`v` 数值切片（NaN 会被跳过，同 pandas）；`q` 分位，落在 `[0, 1]`。
/// - 加工：滤掉 NaN 并升序排序 → 位置取 `q × (n - 1)` → 在相邻两个次序统计量之间
///   按小数部分线性插值。
/// - 出参：分位数；无有效值时返回 NaN。
pub fn quantile(v: &[f64], q: f64) -> f64 {
    let mut xs: Vec<f64> = v.iter().copied().filter(|x| !x.is_nan()).collect();
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).expect("已过滤 NaN"));
    let pos = q * (xs.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        xs[lo]
    } else {
        xs[lo] + (pos - lo as f64) * (xs[hi] - xs[lo])
    }
}

/// 中心矩 `Σ(x - x̄)^k / n`（总体口径，不做自由度修正）。
///
/// - 入参：`v` 数值切片（须已无 NaN）；`k` 阶数。
/// - 加工：先求均值，再累加偏差的 `k` 次方后除以样本数。
/// - 出参：第 `k` 阶中心矩；切片为空时返回 NaN。
fn moment(v: &[f64], k: i32) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let m = nanmean(v);
    v.iter().map(|x| (x - m).powi(k)).sum::<f64>() / v.len() as f64
}

/// 偏度，对齐 `scipy.stats.skew` 的默认（有偏）口径。
///
/// - 入参：`v` 数值切片。
/// - 加工：`三阶中心矩 / 二阶中心矩^1.5`。
/// - 出参：偏度；二阶中心矩为 0（取值全同）时返回 NaN——scipy 在这种输入上也给 NaN
///   并附"灾难性抵消"警告，此处保持一致。
pub fn skew(v: &[f64]) -> f64 {
    let m2 = moment(v, 2);
    if m2.is_nan() || m2 <= 0.0 {
        return f64::NAN;
    }
    moment(v, 3) / m2.powf(1.5)
}

/// Pearson 峰度，对齐 `scipy.stats.kurtosis(fisher=False)` 的有偏口径（**未减 3**）。
///
/// - 入参：`v` 数值切片。
/// - 加工：`四阶中心矩 / 二阶中心矩²`。
/// - 出参：峰度；二阶中心矩为 0 时返回 NaN（同 scipy）。
///   注意与 [`crate::factor::Factor::ts_kurtosis`] 不同，那个是减了 3 的超额峰度。
pub fn kurtosis_pearson(v: &[f64]) -> f64 {
    let m2 = moment(v, 2);
    if m2.is_nan() || m2 <= 0.0 {
        return f64::NAN;
    }
    moment(v, 4) / (m2 * m2)
}

/// Pearson 相关系数，对应 `scipy.stats.linregress(...)` 返回的 `rvalue`。
///
/// - 入参：`x` / `y` 等长数值切片（须已无 NaN）。
/// - 加工：两侧各自去均值后算内积，除以两侧偏差平方和之积的平方根。
/// - 出参：相关系数，落在 `[-1, 1]`；长度不足 2、长度不等或任一侧无波动时返回 NaN。
pub fn pearson_r(x: &[f64], y: &[f64]) -> f64 {
    if x.len() != y.len() || x.len() < 2 {
        return f64::NAN;
    }
    let (mx, my) = (nanmean(x), nanmean(y));
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (a, b) in x.iter().zip(y.iter()) {
        let (dx, dy) = (a - mx, b - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return f64::NAN;
    }
    sxy / (sxx * syy).sqrt()
}

/// 逐点累计最大值，对应 `pandas.Series.expanding().max()` / `cummax()`。
///
/// - 入参：`v` 数值切片。
/// - 加工：从左到右滚动取到目前为止的最大值。
/// - 出参：等长向量，第 `i` 位是 `v[..=i]` 的最大值。
pub fn cummax(v: &[f64]) -> Vec<f64> {
    let mut best = f64::NEG_INFINITY;
    v.iter()
        .map(|x| {
            if *x > best {
                best = *x;
            }
            best
        })
        .collect()
}
