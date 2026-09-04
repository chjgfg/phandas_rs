//! 分组运算子（逐 timestamp × 分组），对应 Python 侧 `group_*` 家族。

use std::collections::BTreeMap;

use super::constants::{group_definitions, EPSILON};
use super::core::Factor;
use super::numeric::{fmt_num, nanmax, nanmean, nanmedian, nanmin, nanstd, rank_pct, RankMethod};

impl Factor {
    // ========================================================================
    // 分组运算子（逐 timestamp × 分组），对应 Python 侧 `group_*` 家族
    // ========================================================================

    /// 由 symbol → 组号映射生成分组因子，对应 Python 侧 `group(mapping_dict)`。
    /// 未出现在映射中的 symbol 记为 NaN（视为无分组）。
    ///
    /// - 入参：`mapping` 标的名到组号的映射；`name` 结果因子名（`None` 时用 `group(custom_map)`）。
    /// - 加工：按自身的标的顺序查表得到一行组号 → 该行在所有期上重复（分组不随时间变化）。
    /// - 出参：与自身同形状的"组号因子"，专门用于喂给 `group_*` 系列方法。
    pub fn group_map(&self, mapping: &BTreeMap<String, f64>, name: Option<&str>) -> Factor {
        let row: Vec<f64> = self
            .symbols
            .iter()
            .map(|s| mapping.get(s).copied().unwrap_or(f64::NAN))
            .collect();
        let values = (0..self.timestamps.len())
            .flat_map(|_| row.iter().copied())
            .collect();
        self.like(values, name.unwrap_or("group(custom_map)").to_string())
    }

    /// 由内置分组名生成分组因子，对应 Python 侧 `group('SECTOR_L1_L2')`。
    ///
    /// - 入参：`mapping` 内置方案名（见 [`group_definitions`]）。
    /// - 加工：查内置表取出映射 → 转交 [`Factor::group_map`]。
    /// - 出参：`Ok(组号因子)`，名形如 `group(SECTOR_L1_L2)`；方案名未知时返回 `Err`。
    pub fn group_named(&self, mapping: &str) -> Result<Factor, String> {
        let dict = group_definitions(mapping).ok_or_else(|| {
            format!(
                "Unknown mapping name '{mapping}'. Available: ['SECTOR_L1_L2', 'DAPP_ACTIVITY']"
            )
        })?;
        Ok(self.group_map(&dict, Some(&format!("group({mapping})"))))
    }

    /// 分组运算通用实现：逐期按组号切分，把每组的值交给 `f` 处理并写回原位。
    /// 组号为 NaN 的位置视为"被 pandas groupby 丢弃"，聚合结果记为 NaN。
    ///
    /// - 入参：`group` 组号因子；`name` 结果因子名；`f` 接收一组值、返回同长度结果的闭包。
    /// - 加工：对齐两者索引 → 逐期按组号（以 f64 位模式为键）分桶 → 每桶的值交给 `f`
    ///   → 结果按原下标写回。组号为 NaN 的标的不进任何桶，输出保持 NaN。
    /// - 出参：同交集形状的新因子。debug 构建下会断言 `f` 的返回长度。
    fn by_group(&self, group: &Factor, name: String, f: impl Fn(&[f64]) -> Vec<f64>) -> Factor {
        let (timestamps, symbols, xs, gs) = self.align(group);
        let n = symbols.len();
        let mut values = vec![f64::NAN; xs.len()];

        for ti in 0..timestamps.len() {
            let base = ti * n;
            let mut buckets: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
            for j in 0..n {
                let g = gs[base + j];
                if g.is_nan() {
                    continue;
                }
                buckets.entry(g.to_bits()).or_default().push(j);
            }
            for idxs in buckets.values() {
                let vals: Vec<f64> = idxs.iter().map(|&j| xs[base + j]).collect();
                let res = f(&vals);
                debug_assert_eq!(res.len(), idxs.len(), "分组运算须返回等长结果");
                for (k, &j) in idxs.iter().enumerate() {
                    values[base + j] = res[k];
                }
            }
        }
        Factor {
            name,
            timestamps,
            symbols,
            values,
        }
    }

    /// 组内去均值，对应 Python 侧 `group_neutralize()`。
    ///
    /// - 入参：`group` 组号因子（通常来自 [`Factor::group_map`]）。
    /// - 加工：逐期按组分桶 → 每桶减去桶内均值（均值跳过 NaN）。
    /// - 出参：同形状因子，每组内部和为 0，剔除了"板块共同涨跌"，
    ///   名形如 `group_neutralize(close,group(SECTOR_L1_L2))`。
    pub fn group_neutralize(&self, group: &Factor) -> Factor {
        let name = format!("group_neutralize({},{})", self.name, group.name);
        self.by_group(group, name, |vals| {
            let m = nanmean(vals);
            vals.iter().map(|x| x - m).collect()
        })
    }

    /// 组内均值广播，对应 Python 侧 `group_mean()`。
    ///
    /// - 入参：`group` 组号因子。
    /// - 加工：逐期按组分桶 → 求桶内均值 → 广播回该桶每个成员。
    /// - 出参：同形状因子，同组成员取值相同，可当作"板块基准"使用。
    pub fn group_mean(&self, group: &Factor) -> Factor {
        let name = format!("group_mean({},{})", self.name, group.name);
        self.by_group(group, name, |vals| vec![nanmean(vals); vals.len()])
    }

    /// 组内中位数广播，对应 Python 侧 `group_median()`。
    ///
    /// - 入参：`group` 组号因子。
    /// - 加工：逐期按组分桶 → 求桶内中位数 → 广播回该桶每个成员。
    /// - 出参：同形状因子，同组成员取值相同；比组内均值更抗极端值。
    pub fn group_median(&self, group: &Factor) -> Factor {
        let name = format!("group_median({},{})", self.name, group.name);
        self.by_group(group, name, |vals| vec![nanmedian(vals); vals.len()])
    }

    /// 组内百分位排名（并列取平均名次），对应 Python 侧 `group_rank()`。
    ///
    /// - 入参：`group` 组号因子。
    /// - 加工：逐期按组分桶 → 桶内按 `method='average'` 求名次并除以桶内有效个数。
    /// - 出参：同形状因子，取值落在 `(0, 1]`，实现"同板块内部横向比较"。
    ///   注意与横截面 [`Factor::rank`] 的并列规则不同（这里取平均、那里取最小）。
    pub fn group_rank(&self, group: &Factor) -> Factor {
        let name = format!("group_rank({},{})", self.name, group.name);
        self.by_group(group, name, |vals| rank_pct(vals, RankMethod::Average))
    }

    /// 组内 min-max 缩放到 [0, 1]，极差过小时取 0.5。对应 Python 侧 `group_scale()`。
    ///
    /// - 入参：`group` 组号因子。
    /// - 加工：逐期按组分桶 → 取桶内最小与最大 → 极差大于 `1e-10` 时算 `(x - min) / 极差`，
    ///   否则整桶置 `0.5`（单一成员组或组内取值全同都会走这条）。
    /// - 出参：同形状因子，取值落在 `[0, 1]`。
    pub fn group_scale(&self, group: &Factor) -> Factor {
        let name = format!("group_scale({},{})", self.name, group.name);
        self.by_group(group, name, |vals| {
            let lo = nanmin(vals);
            let hi = nanmax(vals);
            let denom = hi - lo;
            if denom > EPSILON {
                vals.iter().map(|x| (x - lo) / denom).collect()
            } else {
                vec![0.5; vals.len()]
            }
        })
    }

    /// 组内 z-score，组内标准差过小时输出 NaN。对应 Python 侧 `group_zscore()`。
    ///
    /// - 入参：`group` 组号因子。
    /// - 加工：逐期按组分桶 → 桶内减均值再除以样本标准差（`ddof = 1`）；
    ///   标准差不超过 `1e-10` 时整桶置 NaN（单一成员组必然如此）。
    /// - 出参：同形状因子，每组内部均值 0、标准差 1。
    pub fn group_zscore(&self, group: &Factor) -> Factor {
        let name = format!("group_zscore({},{})", self.name, group.name);
        self.by_group(group, name, |vals| {
            let m = nanmean(vals);
            let sd = nanstd(vals, 1);
            if sd > EPSILON {
                vals.iter().map(|x| (x - m) / sd).collect()
            } else {
                vec![f64::NAN; vals.len()]
            }
        })
    }

    /// 组内按绝对值和归一，绝对值和过小时输出 0。对应 Python 侧 `group_normalize()`。
    ///
    /// - 入参：`group` 组号因子；`scale` 每组的目标绝对值和。
    /// - 加工：逐期按组分桶 → 求桶内绝对值和（跳过 NaN）→ 大于 `1e-10` 时同除该和再乘 `scale`，
    ///   否则整桶置 `0.0`。
    /// - 出参：同形状因子，每组绝对值和等于 `scale`，可用于"按板块分配等额资金"。
    pub fn group_normalize(&self, group: &Factor, scale: f64) -> Factor {
        let name = format!(
            "group_normalize({},{},{})",
            self.name,
            group.name,
            fmt_num(scale)
        );
        self.by_group(group, name, move |vals| {
            let abs_sum: f64 = vals.iter().filter(|x| !x.is_nan()).map(|x| x.abs()).sum();
            if abs_sum > EPSILON {
                vals.iter().map(|x| x / abs_sum * scale).collect()
            } else {
                vec![0.0; vals.len()]
            }
        })
    }
}
