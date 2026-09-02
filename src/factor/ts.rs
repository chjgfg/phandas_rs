//! 时序运算子（逐 symbol 滚动），对应 Python 侧 `ts_*` 家族。

use super::constants::{EPSILON, MATRIX_COND_THRESHOLD, TOLERANCE_FLOAT};
use super::core::Factor;
use super::linalg::{cond_sym, gram, invert, ols};
use super::numeric::{
    argsort_stable, clean_inf, count_valid, covariance, fmt_num, has_nan, nanmax, nanmean,
    nanmedian, nanmin, nanstd, Driver,
};

impl Factor {
    // ========================================================================
    // 时序运算子（逐 symbol 滚动），对应 Python 侧 `ts_*` 家族
    // ========================================================================

    /// 前移 `window` 期，对应 Python 侧 `ts_delay()`。
    ///
    /// - 入参：`window` 前移期数。
    /// - 加工：逐标的沿时间取 `window` 期之前的值；不足 `window` 期的开头填 NaN。
    /// - 出参：同形状因子，名形如 `ts_delay(close,20)`。构造动量、滞后项的基础件。
    pub fn ts_delay(&self, window: usize) -> Factor {
        self.map_series(format!("ts_delay({},{})", self.name, window), move |col| {
            (0..col.len())
                .map(|i| if i >= window { col[i - window] } else { f64::NAN })
                .collect()
        })
    }

    /// 与 `window` 期前之差，对应 Python 侧 `ts_delta()`。
    ///
    /// - 入参：`window` 回看期数。
    /// - 加工：逐标的算 `当期值 − window 期前的值`；开头不足期填 NaN。
    /// - 出参：同形状的绝对变动因子，名形如 `ts_delta(close,1)`。
    pub fn ts_delta(&self, window: usize) -> Factor {
        self.map_series(format!("ts_delta({},{})", self.name, window), move |col| {
            (0..col.len())
                .map(|i| if i >= window { col[i] - col[i - window] } else { f64::NAN })
                .collect()
        })
    }

    /// 时间序号（自 `start` 起递增），对应 Python 侧 `ts_step()`。忽略因子本身取值。
    ///
    /// - 入参：`start` 首期的序号。
    /// - 加工：第 `ti` 期的所有标的一律取 `ti + start`，完全不看原始值。
    /// - 出参：同形状的"时间坐标"因子，名形如 `ts_step(1)`。
    ///   主要给 [`Factor::ts_trend_strength`] 当自变量用。
    pub fn ts_step(&self, start: i64) -> Factor {
        let n = self.symbols.len();
        let values = (0..self.timestamps.len())
            .flat_map(|ti| std::iter::repeat_n((ti as i64 + start) as f64, n))
            .collect();
        self.like(values, format!("ts_step({start})"))
    }

    /// 滚动求和，对应 Python 侧 `ts_sum()`。窗口须填满且不含 NaN。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取长度 `window` 的窗口，窗口干净时累加。
    /// - 出参：同形状因子，前 `window - 1` 期为 NaN，名形如 `ts_sum(close,20)`。
    pub fn ts_sum(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_sum({},{})", self.name, window), |w| {
            w.iter().sum()
        })
    }

    /// 滚动乘积，对应 Python 侧 `ts_product()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口后连乘。
    /// - 出参：同形状因子，前 `window - 1` 期为 NaN。常用于把逐期收益率连乘成累计收益。
    pub fn ts_product(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_product({},{})", self.name, window), |w| {
            w.iter().product()
        })
    }

    /// 滚动均值，对应 Python 侧 `ts_mean()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口后求算术平均。
    /// - 出参：同形状的移动平均因子，前 `window - 1` 期为 NaN。
    pub fn ts_mean(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_mean({},{})", self.name, window), |w| {
            w.iter().sum::<f64>() / w.len() as f64
        })
    }

    /// 滚动中位数，对应 Python 侧 `ts_median()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口后求中位数。
    /// - 出参：同形状因子，前 `window - 1` 期为 NaN；比移动平均更抗单期跳空。
    pub fn ts_median(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_median({},{})", self.name, window), nanmedian)
    }

    /// 滚动标准差（`ddof = 1`），对应 Python 侧 `ts_std_dev()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口后求样本标准差。
    /// - 出参：同形状的波动率因子，前 `window - 1` 期为 NaN。
    pub fn ts_std_dev(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_std_dev({},{})", self.name, window), |w| {
            nanstd(w, 1)
        })
    }

    /// 滚动最小值，对应 Python 侧 `ts_min()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口后取最小值。
    /// - 出参：同形状因子，前 `window - 1` 期为 NaN；配合 [`Factor::ts_max`] 可构造通道位置。
    pub fn ts_min(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_min({},{})", self.name, window), nanmin)
    }

    /// 滚动最大值，对应 Python 侧 `ts_max()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口后取最大值。
    /// - 出参：同形状因子，前 `window - 1` 期为 NaN。
    pub fn ts_max(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_max({},{})", self.name, window), nanmax)
    }

    /// 窗口内最大值距当期的期数（0 表示就在当期），对应 Python 侧 `ts_arg_max()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口 → 找最大值位置（并列取最早）→ 换算成"距窗口末端的距离"。
    /// - 出参：同形状因子，取值落在 `[0, window - 1]`，衡量"高点出现在多久以前"。
    pub fn ts_arg_max(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_arg_max({},{})", self.name, window), |w| {
            let mut best = 0usize;
            for i in 1..w.len() {
                if w[i] > w[best] {
                    best = i;
                }
            }
            (w.len() - 1 - best) as f64
        })
    }

    /// 窗口内最小值距当期的期数，对应 Python 侧 `ts_arg_min()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口 → 找最小值位置（并列取最早）→ 换算成距窗口末端的距离。
    /// - 出参：同形状因子，取值落在 `[0, window - 1]`，衡量"低点出现在多久以前"。
    pub fn ts_arg_min(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_arg_min({},{})", self.name, window), |w| {
            let mut best = 0usize;
            for i in 1..w.len() {
                if w[i] < w[best] {
                    best = i;
                }
            }
            (w.len() - 1 - best) as f64
        })
    }

    /// 窗口内 NaN 个数，对应 Python 侧 `ts_count_nans()`。
    /// 注意：窗口内若无任何有效值，pandas 的 `min_periods=1` 会直接给出 NaN，此处保持一致。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取窗口（开头允许不满）→ 至少有 1 个有效值时数出 NaN 个数，
    ///   全窗口皆 NaN 时输出 NaN。
    /// - 出参：同形状因子，可用来做数据质量筛查。
    pub fn ts_count_nans(&self, window: usize) -> Factor {
        self.rolling(
            window,
            1,
            format!("ts_count_nans({},{})", self.name, window),
            |w| (w.len() - count_valid(w)) as f64,
        )
    }

    /// 当期值与滚动均值之差，对应 Python 侧 `ts_av_diff()`。
    ///
    /// - 入参：`window` 均值窗口期数。
    /// - 加工：先算 [`Factor::ts_mean`]，再逐格相减。
    /// - 出参：同形状因子，正值表示当期高于近期均值，名形如 `ts_av_diff(close,20)`。
    pub fn ts_av_diff(&self, window: usize) -> Factor {
        let mean = self.ts_mean(window);
        self.subtract(&mean)
            .rename(&format!("ts_av_diff({},{})", self.name, window))
    }

    /// 滚动百分位排名（当期值在窗口内的名次），对应 Python 侧 `ts_rank()`。
    /// 窗口含 NaN 或窗口内取值全部相同时输出 NaN。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口 → 取值全同则 NaN → 否则稳定排序给出 `1..window` 的名次，
    ///   取**当期（窗口末端）**那一档除以 `window`。
    /// - 出参：同形状因子，取值落在 `(0, 1]`——`1` 表示当期是窗口内新高。
    ///   与横截面 [`Factor::rank`] 的区别：这里比的是"自己的历史"，那里比的是"同期其他标的"。
    pub fn ts_rank(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_rank({},{})", self.name, window), |w| {
            if w.windows(2).all(|p| p[0] == p[1]) {
                return f64::NAN;
            }
            let order = argsort_stable(w);
            let mut rank = vec![0.0; w.len()];
            for (k, &i) in order.iter().enumerate() {
                rank[i] = (k + 1) as f64;
            }
            rank[w.len() - 1] / w.len() as f64
        })
    }

    /// 滚动 min-max 缩放后加上常数，对应 Python 侧 `ts_scale()`。
    ///
    /// - 入参：`window` 窗口期数；`constant` 缩放后统一加上的偏移量。
    /// - 加工：算窗口最小与最大 → `(当期 − min) / (max − min)`（除数绝对值不超过 `1e-10`
    ///   时该格为 NaN）→ `±inf` 归一为 NaN → 加 `constant`。
    /// - 出参：同形状因子，取值落在 `[constant, constant + 1]`，表示当期在近期区间中的相对位置。
    pub fn ts_scale(&self, window: usize, constant: f64) -> Factor {
        let lo = self.ts_min(window);
        let hi = self.ts_max(window);
        let scaled = self.subtract(&lo).divide(&hi.subtract(&lo));
        let cleaned = scaled.map_values(scaled.name.clone(), clean_inf);
        cleaned.add(constant).rename(&format!(
            "ts_scale({},{},{})",
            self.name,
            window,
            fmt_num(constant)
        ))
    }

    /// 滚动 z-score，对应 Python 侧 `ts_zscore()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：`(当期 − 滚动均值) / 滚动标准差`，除数过小时该格 NaN，`±inf` 归一为 NaN。
    /// - 出参：同形状因子，衡量当期偏离自身近期水平多少个标准差，名形如 `ts_zscore(close,30)`。
    pub fn ts_zscore(&self, window: usize) -> Factor {
        let mean = self.ts_mean(window);
        let std = self.ts_std_dev(window);
        let z = self.subtract(&mean).divide(&std);
        let name = format!("ts_zscore({},{})", self.name, window);
        z.map_values(name, clean_inf)
    }

    /// 滚动分位数映射：先取 `ts_rank`，再做分位数反函数映射。对应 Python 侧 `ts_quantile()`。
    ///
    /// - 入参：`window` 窗口期数；`driver` 目标分布。
    /// - 加工：先算 [`Factor::ts_rank`] 得到 `(0, 1]` 的时序名次 → 夹到 `[1e-6, 1 - 1e-6]`
    ///   → 过目标分布的分位数反函数。
    /// - 出参：同形状因子，把"时序名次"重塑成目标分布形状，
    ///   名形如 `ts_quantile(close,30,gaussian)`。
    pub fn ts_quantile(&self, window: usize, driver: Driver) -> Factor {
        let ranked = self.ts_rank(window);
        let name = format!(
            "ts_quantile({},{},{})",
            self.name,
            window,
            driver.label()
        );
        ranked.map_values(name, move |r| {
            if r.is_nan() {
                f64::NAN
            } else {
                driver.ppf(r.clamp(TOLERANCE_FLOAT, 1.0 - TOLERANCE_FLOAT))
            }
        })
    }

    /// 滚动超额峰度（`ddof = 0`，已减 3），对应 Python 侧 `ts_kurtosis()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口 → 取值全同或总体标准差小于 `1e-10` 则 NaN
    ///   → 否则算 `四阶中心矩 / 标准差⁴ − 3`。
    /// - 出参：同形状因子，正值表示比正态分布更"尖峰厚尾"（极端行情更频繁）。
    pub fn ts_kurtosis(&self, window: usize) -> Factor {
        self.rolling_full(window, format!("ts_kurtosis({},{})", self.name, window), |w| {
            if w.windows(2).all(|p| p[0] == p[1]) {
                return f64::NAN;
            }
            let m = nanmean(w);
            let sd = nanstd(w, 0);
            if sd < EPSILON {
                return f64::NAN;
            }
            let m4: f64 = w.iter().map(|x| (x - m).powi(4)).sum::<f64>() / w.len() as f64;
            m4 / sd.powi(4) - 3.0
        })
    }

    /// 滚动样本偏度，对应 Python 侧 `ts_skewness()`（按其算子组合方式实现）。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：偏离项取 `当期值 − 当期滚动均值` → 分别对其三次方、二次方做滚动求和
    ///   → `n · Σ偏离³ / ((Σ偏离²)^1.5 · (n-1)(n-2))`。
    /// - 出参：同形状因子，正值表示右偏。**注意**：由于是两层滚动复合，需要
    ///   `2 × window - 1` 期才有首个有效值，且这不是窗口内的标准样本偏度——
    ///   这是 Python 版的实现方式，此处如实复刻。
    pub fn ts_skewness(&self, window: usize) -> Factor {
        let n = window as f64;
        let mean = self.ts_mean(window);
        let diff = self.subtract(&mean);

        let sum_cube = diff.power(3.0).ts_sum(window);
        let sum_sq = diff.power(2.0).ts_sum(window);
        let numerator = sum_cube.multiply(n);
        let denominator = sum_sq.power(1.5).multiply((n - 1.0) * (n - 2.0));
        numerator
            .divide(&denominator)
            .rename(&format!("ts_skewness({},{})", self.name, window))
    }

    /// 用窗口内第 `k` 个最近的有效值回填当期缺失，对应 Python 侧 `ts_backfill()`。
    ///
    /// - 入参：`window` 回看窗口期数；`k` 取倒数第几个有效值（`1` 即最近的一个）。
    /// - 加工：逐标的取窗口（开头允许不满）→ 当期非 NaN 则原样保留 → 当期为 NaN 时，
    ///   若窗口内有效值个数不少于 `k`，取倒数第 `k` 个；不足则仍为 NaN。
    ///   窗口内全为 NaN 时输出 NaN。
    /// - 出参：`Ok(Factor)`，缺口被历史值填上；`k = 0` 时返回 `Err`。
    pub fn ts_backfill(&self, window: usize, k: usize) -> Result<Factor, String> {
        if k == 0 {
            return Err("k must be a positive integer".to_string());
        }
        Ok(self.rolling(
            window,
            1,
            format!("ts_backfill({},{},{})", self.name, window, k),
            move |w| {
                let last = w[w.len() - 1];
                if !last.is_nan() {
                    return last;
                }
                let valid: Vec<f64> = w.iter().copied().filter(|x| !x.is_nan()).collect();
                if valid.len() >= k {
                    valid[valid.len() - k]
                } else {
                    last
                }
            },
        ))
    }

    /// 指数衰减加权均值，对应 Python 侧 `ts_decay_exp_window()`。
    /// `nan = true` 时只用有效值加权；`nan = false` 时缺失按 0 参与、权重仍取全窗口。
    ///
    /// - 入参：`window` 窗口期数；`factor` 衰减系数，须落在开区间 `(0, 1)`；
    ///   `nan` 缺失值的处理方式。
    /// - 加工：权重为 `factor^j`（`j` 从旧到新递减到 0），**最近一期权重最大为 1**
    ///   → `nan = true` 时只对有效值加权、权重和也只累计这些位置（窗口全 NaN 则输出 NaN）；
    ///   `nan = false` 时缺失当 0 参与、权重和取全窗口 → 加权和除以权重和。
    /// - 出参：`Ok(Factor)`，等价于带记忆的平滑序列；`factor` 越界时返回 `Err`。
    pub fn ts_decay_exp_window(
        &self,
        window: usize,
        factor: f64,
        nan: bool,
    ) -> Result<Factor, String> {
        if !(factor > 0.0 && factor < 1.0) {
            return Err("Factor must be between 0 and 1 (exclusive)".to_string());
        }
        let name = format!(
            "ts_decay_exp_window({},{},{},{})",
            self.name,
            window,
            fmt_num(factor),
            if nan { "True" } else { "False" }
        );

        Ok(self.map_series(name, move |col| {
            (0..col.len())
                .map(|i| {
                    let start = (i + 1).saturating_sub(window);
                    let w = &col[start..=i];
                    // 权重按距当期远近递减：最近一期权重为 factor^0 = 1
                    let weights: Vec<f64> =
                        (0..w.len()).rev().map(|j| factor.powi(j as i32)).collect();
                    let (mut ws, mut wsum) = (0.0, 0.0);
                    if nan {
                        if count_valid(w) == 0 {
                            return f64::NAN;
                        }
                        for (v, wt) in w.iter().zip(weights.iter()) {
                            if !v.is_nan() {
                                ws += v * wt;
                                wsum += wt;
                            }
                        }
                    } else {
                        for (v, wt) in w.iter().zip(weights.iter()) {
                            ws += if v.is_nan() { 0.0 } else { v * wt };
                            wsum += wt;
                        }
                    }
                    if wsum > 0.0 {
                        ws / wsum
                    } else if nan {
                        f64::NAN
                    } else {
                        0.0
                    }
                })
                .collect()
        }))
    }

    /// 线性衰减加权均值，对应 Python 侧 `ts_decay_linear()`。
    /// `dense = true` 时只用有效值加权；`dense = false`（默认）时缺失按 0 参与、权重取全窗口。
    ///
    /// - 入参：`window` 窗口期数；`dense` 缺失值的处理方式。
    /// - 加工：权重按"由旧到新"取 `window, window-1, ..., 1` → `dense = true` 时只对有效值
    ///   加权并只累计对应权重，`dense = false` 时缺失当 0 参与、权重和取全窗口
    ///   （两种模式下窗口全 NaN 都输出 NaN）→ 加权和除以权重和。
    /// - 出参：同形状的加权平滑因子。
    ///
    /// 注意权重方向：Python 侧用 `np.arange(len, 0, -1)` 对齐"由旧到新"的窗口，
    /// 因此**最早一期权重最大**，与同族的 [`Factor::ts_decay_exp_window`] 恰好相反。
    /// 此处如实复刻该行为，未做"修正"。
    pub fn ts_decay_linear(&self, window: usize, dense: bool) -> Factor {
        let name = format!(
            "ts_decay_linear({},{},dense={})",
            self.name,
            window,
            if dense { "True" } else { "False" }
        );

        self.map_series(name, move |col| {
            (0..col.len())
                .map(|i| {
                    let start = (i + 1).saturating_sub(window);
                    let w = &col[start..=i];
                    // 权重与"由旧到新"的窗口对齐，最早一期权重最大（复刻 Python 行为）
                    let weights: Vec<f64> =
                        (1..=w.len()).rev().map(|j| j as f64).collect();
                    let (mut ws, mut wsum) = (0.0, 0.0);
                    if dense {
                        if count_valid(w) == 0 {
                            return f64::NAN;
                        }
                        for (v, wt) in w.iter().zip(weights.iter()) {
                            if !v.is_nan() {
                                ws += v * wt;
                                wsum += wt;
                            }
                        }
                    } else {
                        if count_valid(w) == 0 {
                            return f64::NAN;
                        }
                        for (v, wt) in w.iter().zip(weights.iter()) {
                            ws += if v.is_nan() { 0.0 } else { v * wt };
                            wsum += wt;
                        }
                    }
                    if wsum > 0.0 {
                        ws / wsum
                    } else {
                        f64::NAN
                    }
                })
                .collect()
        })
    }

    /// 滚动相关系数，对应 Python 侧 `ts_corr()`。
    /// 沿用 Python 的整列前置校验：该标的有效配对少于 2 个、或整列标准差为 0 时全列 NaN。
    ///
    /// - 入参：`other` 另一因子；`window` 窗口期数。
    /// - 加工：对齐两者索引 → 逐标的沿时间取窗口，要求窗口填满且两侧都无 NaN
    ///   → 任一侧窗口标准差为 0 则该格 NaN → 否则算 `协方差 / (标准差x × 标准差y)`。
    /// - 出参：同交集形状因子，取值落在 `[-1, 1]`，名形如 `ts_corr(close,volume,30)`。
    pub fn ts_corr(&self, other: &Factor, window: usize) -> Factor {
        let name = format!("ts_corr({},{},{})", self.name, other.name, window);
        self.rolling_pair(other, window, name, |x, y| {
            let sx = nanstd(x, 1);
            let sy = nanstd(y, 1);
            if sx == 0.0 || sy == 0.0 {
                return f64::NAN;
            }
            covariance(x, y) / (sx * sy)
        })
    }

    /// 滚动协方差（`ddof = 1`），对应 Python 侧 `ts_covariance()`。
    ///
    /// - 入参：`other` 另一因子；`window` 窗口期数。
    /// - 加工：对齐后逐标的取干净窗口，算 `Σ(x - x̄)(y - ȳ) / (n - 1)`。
    /// - 出参：同交集形状因子，与 [`Factor::ts_corr`] 的差别是未做标准差归一，保留量纲。
    pub fn ts_covariance(&self, other: &Factor, window: usize) -> Factor {
        let name = format!("ts_covariance({},{},{})", self.name, other.name, window);
        self.rolling_pair(other, window, name, covariance)
    }

    /// 双因子滚动窗口通用实现：对齐后逐 symbol 沿时间滚动，窗口须填满且两侧均无 NaN。
    ///
    /// - 入参：`other` 另一因子；`window` 窗口期数；`name` 结果因子名；
    ///   `f` 接收两条等长窗口、返回一个标量的闭包。
    /// - 加工：先对齐索引 → 逐标的抽出两条列 → 从第 `window - 1` 期起取配对窗口，
    ///   任一侧含 NaN 就跳过（保留 NaN）→ 否则调用 `f`。
    /// - 出参：同交集形状的新因子，前 `window - 1` 期为 NaN。
    fn rolling_pair(
        &self,
        other: &Factor,
        window: usize,
        name: String,
        f: impl Fn(&[f64], &[f64]) -> f64,
    ) -> Factor {
        let (timestamps, symbols, xs, ys) = self.align(other);
        let n = symbols.len();
        let t = timestamps.len();
        let mut values = vec![f64::NAN; xs.len()];
        for si in 0..n {
            let xc: Vec<f64> = (0..t).map(|ti| xs[ti * n + si]).collect();
            let yc: Vec<f64> = (0..t).map(|ti| ys[ti * n + si]).collect();
            for i in 0..t {
                if i + 1 < window {
                    continue;
                }
                let xw = &xc[i + 1 - window..=i];
                let yw = &yc[i + 1 - window..=i];
                if has_nan(xw) || has_nan(yw) {
                    continue;
                }
                values[i * n + si] = f(xw, yw);
            }
        }
        Factor { name, timestamps, symbols, values }
    }

    /// 滚动自相关：当期序列与其滞后 `lag` 期序列的相关系数。对应 Python 侧 `ts_autocorr()`。
    ///
    /// - 入参：`window` 窗口期数；`lag` 滞后期数，须为正。
    /// - 加工：先做 [`Factor::ts_delay`] 得到滞后序列 → 与自身做 [`Factor::ts_corr`]。
    /// - 出参：`Ok(Factor)`，取值落在 `[-1, 1]`——正值意味动量延续、负值意味均值回复；
    ///   `lag = 0` 时返回 `Err`。
    pub fn ts_autocorr(&self, window: usize, lag: usize) -> Result<Factor, String> {
        if lag == 0 {
            return Err("lag must be positive".to_string());
        }
        let lagged = self.ts_delay(lag);
        Ok(self
            .ts_corr(&lagged, window)
            .rename(&format!("ts_autocorr({},{},{})", self.name, window, lag)))
    }

    /// 滚动线性回归，对应 Python 侧 `ts_regression()`。
    ///
    /// `rettype` 决定输出：`0` 残差、`1` 截距、`2` 首个斜率、`3` 拟合值、`4` SSE、`5` SST、
    /// `6` R²、`7` MSE、`8`/`9` 系数标准误、`100 + i` 第 `i` 个斜率。
    /// `lag` 对自变量整体滞后。窗口内有效样本少于 `自变量个数 + 2` 或设计矩阵病态时输出 NaN。
    ///
    /// - 入参：`x_factors` 一个或多个自变量因子；`window` 窗口期数；
    ///   `lag` 自变量的滞后期数（`0` 为不滞后）；`rettype` 输出内容选择。
    /// - 加工：自变量先各自 [`Factor::ts_delay`] → 全部对齐到共同索引 → 逐标的、逐期取窗口
    ///   → 挑出所有变量都有效的样本，个数不足 `m + 2` 则跳过 → 拼含截距列的设计矩阵，
    ///   Gram 矩阵条件数超过 `1e10` 则跳过 → 解最小二乘并按 `rettype` 取值。
    /// - 出参：同交集形状因子，含义随 `rettype` 变化，名形如
    ///   `ts_regression(close,open,30,lag=0,rettype=6)`；自变量列表为空时全 NaN。
    ///   `rettype = 8` 在单自变量下会索引越界，此处返回 NaN（Python 侧抛 `IndexError`）。
    pub fn ts_regression(
        &self,
        x_factors: &[&Factor],
        window: usize,
        lag: usize,
        rettype: i32,
    ) -> Factor {
        let x_names: Vec<String> = x_factors.iter().map(|f| f.name.clone()).collect();
        let name = format!(
            "ts_regression({},{},{},lag={},rettype={})",
            self.name,
            x_names.join(","),
            window,
            lag,
            rettype
        );
        if x_factors.is_empty() {
            return self.like(vec![f64::NAN; self.values.len()], name);
        }

        // 自变量先按 symbol 滞后，再统一对齐到共同索引
        let lagged: Vec<Factor> = x_factors
            .iter()
            .map(|f| if lag > 0 { f.ts_delay(lag) } else { (*f).clone() })
            .collect();
        let mut timestamps = self.timestamps.clone();
        let mut symbols = self.symbols.clone();
        for f in &lagged {
            let probe = Factor {
                name: String::new(),
                timestamps: timestamps.clone(),
                symbols: symbols.clone(),
                values: vec![f64::NAN; timestamps.len() * symbols.len()],
            };
            let (ts, sy, _, _) = probe.align(f);
            timestamps = ts;
            symbols = sy;
        }
        let y_all = self.reindex(&timestamps, &symbols);
        let x_all: Vec<Vec<f64>> = lagged.iter().map(|f| f.reindex(&timestamps, &symbols)).collect();

        let n = symbols.len();
        let t = timestamps.len();
        let m = x_all.len();
        let mut values = vec![f64::NAN; y_all.len()];

        for si in 0..n {
            for i in 0..t {
                if i + 1 < window {
                    continue;
                }
                let rows: Vec<usize> = (i + 1 - window..=i)
                    .filter(|&r| {
                        !y_all[r * n + si].is_nan()
                            && x_all.iter().all(|x| !x[r * n + si].is_nan())
                    })
                    .collect();
                if rows.len() < m + 2 {
                    continue;
                }
                let yv: Vec<f64> = rows.iter().map(|&r| y_all[r * n + si]).collect();
                let design: Vec<Vec<f64>> = rows
                    .iter()
                    .map(|&r| {
                        let mut row = Vec::with_capacity(m + 1);
                        row.push(1.0);
                        row.extend(x_all.iter().map(|x| x[r * n + si]));
                        row
                    })
                    .collect();
                let g = gram(&design);
                if cond_sym(&g) > MATRIX_COND_THRESHOLD {
                    continue;
                }
                let Some(params) = ols(&design, &yv) else {
                    continue;
                };
                let fitted: Vec<f64> = design
                    .iter()
                    .map(|row| row.iter().zip(params.iter()).map(|(a, b)| a * b).sum())
                    .collect();
                let residuals: Vec<f64> =
                    yv.iter().zip(fitted.iter()).map(|(y, f)| y - f).collect();
                let sse: f64 = residuals.iter().map(|r| r * r).sum();
                let ybar = nanmean(&yv);
                let sst: f64 = yv.iter().map(|y| (y - ybar) * (y - ybar)).sum();
                let df = yv.len() as i64 - m as i64 - 1;

                let out = match rettype {
                    0 => Some(residuals[residuals.len() - 1]),
                    1 => Some(params[0]),
                    2 => params.get(1).copied(),
                    3 => Some(fitted[fitted.len() - 1]),
                    4 => Some(sse),
                    5 => Some(sst),
                    6 => {
                        if sst > 0.0 {
                            Some(1.0 - sse / sst)
                        } else {
                            None
                        }
                    }
                    7 => {
                        if df > 0 {
                            Some(sse / df as f64)
                        } else {
                            None
                        }
                    }
                    8 | 9 => {
                        let mse = if df > 0 { sse / df as f64 } else { 0.0 };
                        if mse > 0.0 {
                            let idx = if rettype == 8 { 2 } else { 0 };
                            invert(&g)
                                .filter(|inv| idx < inv.len())
                                .map(|inv| (mse * inv[idx][idx]).sqrt())
                        } else {
                            None
                        }
                    }
                    r if r >= 100 => params.get((r - 100 + 1) as usize).copied(),
                    _ => None,
                };
                if let Some(v) = out {
                    values[i * n + si] = v;
                }
            }
        }
        Factor { name, timestamps, symbols, values }
    }

    /// 滚动变异系数：标准差与均值绝对值之比，对应 Python 侧 `ts_cv()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：逐标的取干净窗口 → `样本标准差 / (|均值| + 1e-10)` → `±inf` 归一为 NaN。
    /// - 出参：同形状因子，无量纲的相对波动度量，便于跨标的比较。
    pub fn ts_cv(&self, window: usize) -> Factor {
        let f = self.rolling_full(window, format!("ts_cv({},{})", self.name, window), |w| {
            nanstd(w, 1) / (nanmean(w).abs() + EPSILON)
        });
        let name = f.name.clone();
        f.map_values(name, clean_inf)
    }

    /// 滚动跳跃度：窗口内逐期绝对变动之和与区间极差之比，对应 Python 侧 `ts_jumpiness()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：先算单期差分取绝对值再滚动求和（走过的总路程）→ 除以 `滚动最大 − 滚动最小`
    ///   加 `1e-10`（直线距离）→ `±inf` 归一为 NaN。
    /// - 出参：同形状因子，接近 1 表示单边趋势，远大于 1 表示来回震荡。
    pub fn ts_jumpiness(&self, window: usize) -> Factor {
        let total_jump = self.ts_delta(1).abs().ts_sum(window);
        let range = self.ts_max(window).subtract(&self.ts_min(window));
        let ratio = total_jump.divide(&range.add(EPSILON));
        ratio.map_values(
            format!("ts_jumpiness({},{})", self.name, window),
            clean_inf,
        )
    }

    /// 滚动趋势强度：对时间序号回归的 R²，对应 Python 侧 `ts_trend_strength()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：用 [`Factor::ts_step`] 造时间坐标当自变量 → 调
    ///   [`Factor::ts_regression`] 取 `rettype = 6`（R²）。
    /// - 出参：同形状因子，取值落在 `[0, 1]`——越接近 1 说明窗口内走势越接近一条直线。
    pub fn ts_trend_strength(&self, window: usize) -> Factor {
        let step = self.ts_step(1);
        self.ts_regression(&[&step], window, 0, 6)
            .rename(&format!("ts_trend_strength({},{})", self.name, window))
    }

    /// 滚动方差比：`k` 期差分方差与 `k ×` 单期差分方差之比，对应 Python 侧 `ts_vr()`。
    ///
    /// - 入参：`window` 窗口期数；`k` 长周期差分的跨度，须为正。
    /// - 加工：分别算 `k` 期差分与 1 期差分的滚动标准差并平方 → 相除，分母为
    ///   `k × 单期方差 + 1e-10` → `±inf` 归一为 NaN。
    /// - 出参：`Ok(Factor)`——随机漫步下约等于 1，大于 1 偏趋势、小于 1 偏震荡；
    ///   `k = 0` 时返回 `Err`。
    pub fn ts_vr(&self, window: usize, k: usize) -> Result<Factor, String> {
        if k == 0 {
            return Err("k must be positive".to_string());
        }
        let var_k = self.ts_delta(k).ts_std_dev(window).power(2.0);
        let var_1 = self.ts_delta(1).ts_std_dev(window).power(2.0);
        let ratio = var_k.divide(&var_1.multiply(k as f64).add(EPSILON));
        Ok(ratio.map_values(format!("ts_vr({},{},{})", self.name, window, k), clean_inf))
    }

    /// 滚动反转频率：窗口内一阶差分符号变化的比例，对应 Python 侧 `ts_reversal_count()`。
    ///
    /// - 入参：`window` 窗口期数。
    /// - 加工：窗口内有效值不足 3 个直接 NaN → 算相邻差分 → 滤掉 NaN 差分，
    ///   不足 2 个则 NaN → 数相邻两差分乘积为负（方向翻转）的次数，除以相邻对数。
    /// - 出参：同形状因子，取值落在 `[0, 1]`——0 表示全程单边，接近 1 表示每期都在拐头。
    pub fn ts_reversal_count(&self, window: usize) -> Factor {
        self.rolling(
            window,
            3,
            format!("ts_reversal_count({},{})", self.name, window),
            |w| {
                if w.len() < 3 {
                    return f64::NAN;
                }

                let diffs: Vec<f64> = w.windows(2).map(|p| p[1] - p[0]).collect();
                let valid: Vec<f64> = diffs.into_iter().filter(|d| !d.is_nan()).collect();
                if valid.len() < 2 {
                    return f64::NAN;
                }
                let changes = valid.windows(2).filter(|p| p[0] * p[1] < 0.0).count();
                changes as f64 / (valid.len() - 1) as f64
            },
        )
    }
}
