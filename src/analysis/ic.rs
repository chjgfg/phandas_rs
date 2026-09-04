//! IC（信息系数）：对应上游 `analysis.ic()` 与 `_compute_ic_vectorized`。
//!
//! IC 是"当期因子横截面"与"未来 h 期收益横截面"的相关系数，逐期算一个数，
//! 再对整条序列求均值 / 标准差 / IR / t 值。上游只在 `method == 'spearman'` 时
//! 把两侧换成名次，其余任何取值都落到 Pearson 分支，故此处的口径枚举只有两项。

use super::corr::CorrMethod;
use crate::factor::numeric::{nanmean, nanstd, ranks, RankMethod};
use crate::factor::{Factor, EPSILON};

/// IC 的相关系数口径，对应上游 `ic(method='spearman')`。
///
/// 与 [`CorrMethod`] 分开是有意的：上游 `_compute_ic_vectorized` 只判断
/// `method == 'spearman'`，传 `'kendall'` 会**静默按 Pearson 计算**。这里不复刻那个
/// 静默降级，只提供实际支持的两项。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IcMethod {
    /// 秩 IC：两侧各取横截面平均名次后再算相关系数，上游默认。
    #[default]
    Spearman,
    /// 常规 IC：直接用原始取值算相关系数。
    Pearson,
}

impl IcMethod {
    /// 由字符串解析。
    ///
    /// - 入参：`s` 口径名称。
    /// - 加工：与 `"spearman"` / `"pearson"` 精确匹配。
    /// - 出参：匹配成功返回 [`IcMethod`]；否则返回含可选值清单的错误信息。
    ///   注意上游对非 `"spearman"` 的任何字符串都按 Pearson 处理，此处一律拒绝。
    pub fn parse(s: &str) -> Result<IcMethod, String> {
        match s {
            "spearman" => Ok(IcMethod::Spearman),
            "pearson" => Ok(IcMethod::Pearson),
            other => Err(format!(
                "Invalid method: {other}. Must be one of ['spearman', 'pearson']"
            )),
        }
    }

    /// 口径名称的字符串形式，用于报告排版。
    pub fn label(self) -> &'static str {
        match self {
            IcMethod::Spearman => "Spearman",
            IcMethod::Pearson => "Pearson",
        }
    }
}

impl From<IcMethod> for CorrMethod {
    /// - 入参：`m` IC 口径。
    /// - 加工：映射到同名的相关系数口径。
    /// - 出参：[`CorrMethod`]。注意 IC 的实际算法并不走 [`super::corr::corr`]——
    ///   上游有自己的一套分母口径，见 [`ic_series`]。
    fn from(m: IcMethod) -> CorrMethod {
        match m {
            IcMethod::Spearman => CorrMethod::Spearman,
            IcMethod::Pearson => CorrMethod::Pearson,
        }
    }
}

/// 单个持有期的 IC 统计，对应上游 `ic()` 结果里 `results[factor_name][horizon]` 那个字典。
#[derive(Debug, Clone, PartialEq)]
pub struct IcStats {
    /// 持有期（前瞻期数）。
    pub horizon: usize,
    /// IC 序列均值。
    pub ic_mean: f64,
    /// IC 序列标准差，**`ddof = 0`**（上游用 `np.nanstd` 的默认口径，不是 pandas 的 1）。
    pub ic_std: f64,
    /// 信息比率 `ic_mean / ic_std`；`ic_std` 非正时为 `0.0`。
    pub ir: f64,
    /// t 值 `ic_mean / (ic_std / √n)`，等于 `ir × √n`；`ic_std` 非正时为 `0.0`。
    pub t_stat: f64,
    /// 逐期 IC，`(timestamp, ic)`；被有效性闸门挡掉的期不入列。
    pub ic_series: Vec<(String, f64)>,
}

impl IcStats {
    /// 无有效期时的空统计，对应上游 `len(ic_series) == 0` 分支给的全 NaN 字典。
    ///
    /// - 入参：`horizon` 持有期。
    /// - 加工：四项统计量全置 NaN，序列置空。
    /// - 出参：[`IcStats`]。注意与"有效期存在但 `ic_std = 0`"不同，后者 `ir` / `t_stat` 是 `0.0`。
    fn empty(horizon: usize) -> IcStats {
        IcStats {
            horizon,
            ic_mean: f64::NAN,
            ic_std: f64::NAN,
            ir: f64::NAN,
            t_stat: f64::NAN,
            ic_series: Vec::new(),
        }
    }
}

/// 逐期横截面 IC 序列，对应上游 `_compute_ic_vectorized`。
///
/// - 入参：`factor` 因子；`forward_return` 前瞻收益因子；`method` 相关系数口径。
/// - 加工：两者先按 `(timestamp, symbol)` 取交集对齐（等价上游 `align(join='inner')`），
///   随后逐期——Spearman 口径下两侧各取横截面平均名次；算有效性闸门
///   （两侧标准差都大于 [`EPSILON`] 且**两侧同时非 NaN 的格子**不少于 3 个）；
///   不过闸门的期直接丢弃；过闸门的期各自减去当期均值，按下面的公式取比值。
/// - 出参：`(timestamp, ic)` 序列，只含过闸门且结果非 NaN 的期。
///
/// # 分母口径与真正的相关系数不同
///
/// 上游分子只累加"两侧都非 NaN"的格子，分母却各自累加"自己非 NaN"的全部格子：
///
/// ```text
/// numer = Σ_{f,r 都有效} (f − f̄)(r − r̄)
/// denom = √( Σ_{f 有效} (f − f̄)²  ×  Σ_{r 有效} (r − r̄)² )
/// ```
///
/// 两侧 NaN 分布不同时（例如某标的有因子值但前瞻收益越过了样本末尾），分母被多算，
/// `|IC|` 被系统性地压向 0，取值也不再保证落在 `[-1, 1]` 之内的相关系数语义。
/// 均值 `f̄` / `r̄` 同样是各自在自己有效格上取的。此处**如实复刻**，不做修正。
pub fn ic_series(factor: &Factor, forward_return: &Factor, method: IcMethod) -> Vec<(String, f64)> {
    let (timestamps, symbols, fv, rv) = factor.align(forward_return);
    let n = symbols.len();
    let mut out = Vec::new();

    for (ti, ts) in timestamps.iter().enumerate() {
        let f_raw = &fv[ti * n..(ti + 1) * n];
        let r_raw = &rv[ti * n..(ti + 1) * n];
        let (f_row, r_row) = match method {
            IcMethod::Spearman => (
                ranks(f_raw, RankMethod::Average),
                ranks(r_raw, RankMethod::Average),
            ),
            IcMethod::Pearson => (f_raw.to_vec(), r_raw.to_vec()),
        };

        let both = f_raw
            .iter()
            .zip(r_raw.iter())
            .filter(|(a, b)| !a.is_nan() && !b.is_nan())
            .count();
        if !(nanstd(&f_row, 1) > EPSILON && nanstd(&r_row, 1) > EPSILON && both >= 3) {
            continue;
        }

        let (f_mean, r_mean) = (nanmean(&f_row), nanmean(&r_row));
        let fd: Vec<f64> = f_row.iter().map(|x| x - f_mean).collect();
        let rd: Vec<f64> = r_row.iter().map(|x| x - r_mean).collect();
        let numer = sum_skip_nan(fd.iter().zip(rd.iter()).map(|(a, b)| a * b));
        let sf = sum_skip_nan(fd.iter().map(|x| x * x));
        let sr = sum_skip_nan(rd.iter().map(|x| x * x));

        let ic = numer / (sf * sr).sqrt();
        if !ic.is_nan() {
            out.push((ts.clone(), ic));
        }
    }
    out
}

/// 跳过 NaN 求和，对应 numpy 的 `np.nansum`。
///
/// - 入参：`it` 数值迭代器。
/// - 加工：过滤掉 NaN 后累加。
/// - 出参：有效值之和；全部为 NaN 时返回 `0.0`。
fn sum_skip_nan(it: impl Iterator<Item = f64>) -> f64 {
    it.filter(|x| !x.is_nan()).sum()
}

/// 单个持有期的完整 IC 统计，对应上游 `ic()` 里对一个 `(因子, horizon)` 组合做的那一段。
///
/// - 入参：`factor` 因子；`forward_return` 前瞻收益因子；`horizon` 持有期（仅回填到结果里）；
///   `method` 相关系数口径。
/// - 加工：先取 [`ic_series`]，序列为空则四项统计量全给 NaN；否则算均值与
///   `ddof = 0` 的标准差，标准差为正时再算 `ir` 与 `t_stat`，否则两者都给 `0.0`。
/// - 出参：[`IcStats`]。
pub fn ic_stats(
    factor: &Factor,
    forward_return: &Factor,
    horizon: usize,
    method: IcMethod,
) -> IcStats {
    let ic_series = ic_series(factor, forward_return, method);
    if ic_series.is_empty() {
        return IcStats::empty(horizon);
    }
    let values: Vec<f64> = ic_series.iter().map(|(_, v)| *v).collect();
    let ic_mean = nanmean(&values);
    // 上游用 np.nanstd，默认 ddof = 0；这里刻意不是 pandas 的 ddof = 1
    let ic_std = nanstd(&values, 0);
    let (ir, t_stat) = if ic_std > 0.0 {
        (
            ic_mean / ic_std,
            ic_mean / (ic_std / (values.len() as f64).sqrt()),
        )
    } else {
        (0.0, 0.0)
    };
    IcStats {
        horizon,
        ic_mean,
        ic_std,
        ir,
        t_stat,
        ic_series,
    }
}
