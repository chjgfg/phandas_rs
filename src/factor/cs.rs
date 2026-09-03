//! 横截面运算子（逐 timestamp），对应 Python 侧 `_apply_cs_operation` 家族。

use super::constants::{EPSILON, SIGNAL_LONG_SUM, SIGNAL_SHORT_SUM, SIGNAL_TOLERANCE, TOLERANCE_FLOAT};
use super::core::Factor;
use super::numeric::{
    argsort_stable, count_valid, fmt_num, has_nan, nanmean, nanmedian, nanstd, rank_pct, Driver,
    RankMethod,
};

impl Factor {
    // ========================================================================
    // 横截面运算子（逐 timestamp），对应 Python 侧 `_apply_cs_operation` 家族
    // ========================================================================

    /// 横截面百分位排名，对应 Python 侧 `rank()`。
    /// 当期存在任一 NaN，或当期所有值相同时，整期输出 NaN。
    ///
    /// - 入参：无。
    /// - 加工：逐期检查——含 NaN 或取值全同则整期置 NaN；否则按 `method='min'` 求名次，
    ///   再除以标的数得到百分位。
    /// - 出参：取值落在 `(0, 1]` 的新因子（最小值得 `1/N`、最大值得 `1`），
    ///   名为 `rank(<原名>)`。常用于把量纲各异的原始量压到同一尺度。
    pub fn rank(&self) -> Factor {
        self.map_rows(format!("rank({})", self.name), |row| {
            if has_nan(row) {
                return vec![f64::NAN; row.len()];
            }
            let all_same = row.windows(2).all(|w| w[0] == w[1]);
            if all_same {
                return vec![f64::NAN; row.len()];
            }
            rank_pct(row, RankMethod::Min)
        })
    }

    /// 横截面均值广播，对应 Python 侧 `mean()`。当期含 NaN 时整期 NaN。
    ///
    /// - 入参：无。
    /// - 加工：逐期检查是否含 NaN；干净则求当期均值并**广播回每个标的**。
    /// - 出参：同形状因子，同一期内所有标的取值相同，名为 `mean(<原名>)`。
    ///   常作为"减去市场平均"的中间量使用。
    pub fn cs_mean(&self) -> Factor {
        self.map_rows(format!("mean({})", self.name), |row| {
            if has_nan(row) {
                vec![f64::NAN; row.len()]
            } else {
                vec![nanmean(row); row.len()]
            }
        })
    }

    /// 横截面中位数广播，对应 Python 侧 `median()`。当期含 NaN 时整期 NaN。
    ///
    /// - 入参：无。
    /// - 加工：逐期检查是否含 NaN；干净则求当期中位数并广播回每个标的。
    /// - 出参：同形状因子，同一期内取值相同，名为 `median(<原名>)`。
    ///   比 [`Factor::cs_mean`] 更抗极端值。
    pub fn cs_median(&self) -> Factor {
        self.map_rows(format!("median({})", self.name), |row| {
            if has_nan(row) {
                vec![f64::NAN; row.len()]
            } else {
                vec![nanmedian(row); row.len()]
            }
        })
    }

    /// 横截面去均值（可选除以标准差、可选截断），对应 Python 侧 `normalize()`。
    ///
    /// - 入参：`use_std` 是否再除以当期样本标准差（`ddof = 1`）；
    ///   `limit` 截断幅度，`0.0` 表示不截断，否则把结果夹到 `[-limit, limit]`。
    /// - 加工：逐期检查含 NaN → 减去当期均值 → 按需除以标准差（标准差为 0 或 NaN 时整期 NaN）
    ///   → 按需截断。
    /// - 出参：同形状因子，每期和为 0（未截断时），名形如 `normalize(close,use_std=True,limit=0)`。
    pub fn normalize(&self, use_std: bool, limit: f64) -> Factor {
        let name = format!(
            "normalize({},use_std={},limit={})",
            self.name,
            if use_std { "True" } else { "False" },
            fmt_num(limit)
        );
        self.map_rows(name, move |row| {
            if has_nan(row) {
                return vec![f64::NAN; row.len()];
            }
            let m = nanmean(row);
            let mut out: Vec<f64> = row.iter().map(|x| x - m).collect();
            if use_std {
                let sd = nanstd(row, 1);
                if sd == 0.0 || sd.is_nan() {
                    return vec![f64::NAN; row.len()];
                }
                out.iter_mut().for_each(|x| *x /= sd);
            }
            if limit != 0.0 {
                out.iter_mut().for_each(|x| *x = x.clamp(-limit, limit));
            }
            out
        })
    }

    /// 横截面 z-score，等价于 `normalize(use_std = true, limit = 0.0)`。
    ///
    /// - 入参：无。
    /// - 加工：逐期减均值再除以样本标准差。
    /// - 出参：同形状因子，每期均值 0、标准差 1，名为 `zscore(<原名>)`。
    pub fn zscore(&self) -> Factor {
        self.normalize(true, 0.0).rename(&format!("zscore({})", self.name))
    }

    /// 横截面缩放：默认按绝对值和归一到 `scale`；给定多空比例时分别归一。
    /// 对应 Python 侧 `scale()`。
    ///
    /// - 入参：`scale` 目标绝对值和；`longscale` / `shortscale` 分别指定多头、空头侧的
    ///   目标规模（任一为 `Some` 就走分侧归一，未给的那侧回退用 `scale`）。
    /// - 加工：默认路径——求当期非 NaN 值的绝对值和，为 0 则原样返回，否则整期同除该和再乘
    ///   `scale`。分侧路径——正值按正值绝对值和归一到 `+longscale`，负值按负值绝对值和归一到
    ///   `-shortscale`，零值与 NaN 保持不动。
    /// - 出参：同形状因子，可直接当作仓位权重，名形如 `scale(close,scale=1)`。
    pub fn scale(&self, scale: f64, longscale: Option<f64>, shortscale: Option<f64>) -> Factor {
        let mut parts = vec![format!("scale={}", fmt_num(scale))];
        if let Some(v) = longscale {
            parts.push(format!("longscale={}", fmt_num(v)));
        }
        if let Some(v) = shortscale {
            parts.push(format!("shortscale={}", fmt_num(v)));
        }
        let name = format!("scale({},{})", self.name, parts.join(","));

        self.map_rows(name, move |row| {
            if longscale.is_some() || shortscale.is_some() {
                let ls = longscale.unwrap_or(scale);
                let ss = shortscale.unwrap_or(scale);
                let long_sum: f64 = row.iter().filter(|x| **x > 0.0).map(|x| x.abs()).sum();
                let short_sum: f64 = row.iter().filter(|x| **x < 0.0).map(|x| x.abs()).sum();
                row.iter()
                    .map(|&x| {
                        if x > 0.0 && ls > 0.0 && long_sum > 0.0 {
                            x / long_sum * ls
                        } else if x < 0.0 && ss > 0.0 && short_sum > 0.0 {
                            x / short_sum * -ss
                        } else {
                            x
                        }
                    })
                    .collect()
            } else {
                let abs_sum: f64 = row.iter().filter(|x| !x.is_nan()).map(|x| x.abs()).sum();
                if abs_sum == 0.0 {
                    row.to_vec()
                } else {
                    row.iter().map(|x| x / abs_sum * scale).collect()
                }
            }
        })
    }

    /// 横截面分位数映射，对应 Python 侧 `quantile(driver, sigma)`。
    /// 有效值少于 2 个时整期 NaN。
    ///
    /// - 入参：`driver` 目标分布（高斯 / 均匀 / 柯西）；`sigma` 结果缩放倍数（`1.0` 为不缩放）。
    /// - 加工：逐期给有效值排名并归一到 `(0, 1]` → 用 `1/N + r · (1 - 2/N)` 压进开区间避免
    ///   端点取到 `±inf` → 再夹一次 `[1e-6, 1 - 1e-6]` → 过目标分布的分位数反函数 → 按需乘 `sigma`。
    /// - 出参：同形状因子，把"名次"重塑成目标分布的形状（高斯下即近似正态化），
    ///   名形如 `quantile(close,driver=gaussian,sigma=1)`。
    pub fn quantile(&self, driver: Driver, sigma: f64) -> Factor {
        let name = format!(
            "quantile({},driver={},sigma={})",
            self.name,
            driver.label(),
            fmt_num(sigma)
        );
        self.map_rows(name, move |row| {
            let n_valid = count_valid(row);
            if n_valid < 2 {
                return vec![f64::NAN; row.len()];
            }
            let nf = n_valid as f64;
            // 有效值按升序赋 1..N 名次，再压缩到 (0, 1) 开区间后做 ppf 映射
            let valid: Vec<usize> = argsort_stable(row)
                .into_iter()
                .filter(|&i| !row[i].is_nan())
                .collect();
            let mut ranked = vec![f64::NAN; row.len()];
            for (k, &i) in valid.iter().enumerate() {
                ranked[i] = (k + 1) as f64 / nf;
            }
            ranked
                .into_iter()
                .map(|r| {
                    if r.is_nan() {
                        return f64::NAN;
                    }
                    let eps = TOLERANCE_FLOAT;
                    let shifted = 1.0 / nf + r.clamp(eps, 1.0 - eps) * (1.0 - 2.0 / nf);
                    let p = shifted.clamp(eps, 1.0 - eps);
                    let mapped = driver.ppf(p);
                    if sigma != 1.0 {
                        mapped * sigma
                    } else {
                        mapped
                    }
                })
                .collect()
        })
    }

    /// 多空价差组合：因子值最高的一批置 `+0.5`、最低的一批置 `-0.5`，其余为 0。
    /// 对应 Python 侧 `spread(pct)`。有效值少于 2 个时整期为 0。
    ///
    /// - 入参：`pct` 单侧入选比例，须落在开区间 `(0, 1)`。
    /// - 加工：名额 `n_long = max(1, ⌊标的数 × pct⌋)` → 稳定排序 → 末尾 `n_long` 个记 `+0.5`、
    ///   开头 `n_long` 个记 `-0.5`（两侧重叠时空头覆盖多头，复刻 Python 的赋值顺序）。
    /// - 出参：`Ok(Factor)`，取值只有 `+0.5 / 0 / -0.5` 三档，名形如 `spread(close,0.5)`；
    ///   `pct` 越界时返回 `Err`。
    pub fn spread(&self, pct: f64) -> Result<Factor, String> {
        if !(pct > 0.0 && pct < 1.0) {
            return Err("pct must be between 0 and 1".to_string());
        }
        let name = format!("spread({},{})", self.name, fmt_num(pct));
        Ok(self.map_rows(name, move |row| {
            let n = row.len();
            let mut out = vec![0.0; n];
            if count_valid(row) < 2 {
                return out;
            }
            let n_long = ((n as f64 * pct) as usize).max(1);
            let order = argsort_stable(row);
            // 与 Python 一致：先写多头再写空头，重叠时空头覆盖
            for &i in order.iter().rev().take(n_long) {
                out[i] = 0.5;
            }
            for &i in order.iter().take(n_long) {
                out[i] = -0.5;
            }
            out
        }))
    }

    /// 转为美元中性权重：去均值后按绝对值和归一，多空各占 0.5。
    /// 对应 Python 侧 `signal()`。当期含 NaN 或去均值后绝对值和过小时整期 NaN。
    ///
    /// - 入参：无。
    /// - 加工：逐期检查含 NaN → 减去当期均值（保证多空抵消）→ 除以绝对值和
    ///   （和小于 `1e-10` 时视为无区分度，整期置 NaN）。
    /// - 出参：同形状因子，每期多头权重和 `+0.5`、空头 `-0.5`、总和 0，
    ///   可直接送入回测，名为 `signal(<原名>)`。
    pub fn signal(&self) -> Factor {
        self.map_rows(format!("signal({})", self.name), |row| {
            if has_nan(row) {
                return vec![f64::NAN; row.len()];
            }
            let m = nanmean(row);
            let demeaned: Vec<f64> = row.iter().map(|x| x - m).collect();
            let abs_sum: f64 = demeaned.iter().map(|x| x.abs()).sum();
            if abs_sum < EPSILON {
                return vec![f64::NAN; row.len()];
            }
            demeaned.into_iter().map(|x| x / abs_sum).collect()
        })
    }

    /// 判断指定期是否已经是美元中性信号，对应 Python 侧 `Factor._is_signal`。
    ///
    /// - 入参：`ts` 目标时间戳；传 `None` 表示取最后一期（对应上游的 `timestamp.max()`）。
    /// - 加工：取该期横截面 → 全为 NaN 或该期不存在时判否 → 分别累加正值与负值
    ///   （NaN 因比较恒假而被排除）→ 要求多头和 ≈ `0.5`、空头和 ≈ `-0.5`、总和 ≈ `0`。
    ///   容差沿用 `np.isclose` 的 `atol + rtol × |目标值|`，其中 `atol` 取
    ///   [`SIGNAL_TOLERANCE`]、`rtol` 取 numpy 默认的 `1e-5`。
    /// - 出参：满足三个条件返回 `true`。回测据此判断策略因子能否直接当权重用，
    ///   见 [`crate::backtest::Backtester`]。
    pub fn is_signal(&self, ts: Option<&str>) -> bool {
        let ti = match ts {
            Some(t) => self.timestamps.iter().position(|x| x == t),
            None => self.timestamps.len().checked_sub(1),
        };
        let Some(ti) = ti else { return false };
        let row = self.row(ti);
        if count_valid(row) == 0 {
            return false;
        }
        let long_sum: f64 = row.iter().filter(|v| **v > 0.0).sum();
        let short_sum: f64 = row.iter().filter(|v| **v < 0.0).sum();
        let isclose = |a: f64, b: f64| (a - b).abs() <= SIGNAL_TOLERANCE + 1e-5 * b.abs();
        isclose(long_sum, SIGNAL_LONG_SUM)
            && isclose(short_sum, SIGNAL_SHORT_SUM)
            && isclose(long_sum + short_sum, 0.0)
    }
}
