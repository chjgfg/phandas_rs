//! 绩效指标与回撤区间，口径照抄上游 `backtest._calculate_performance_metrics`。
//!
//! 年化因子固定 365（按**日历天数**折算，不是交易日），无风险利率由调用方给，
//! 上游默认 `0.03`。全部指标都建立在"由收益率序列重建的净值"之上——注意这与
//! 组合本身记录的净值序列不同：后者含播种那一条，前者从第一个收益率开始。

use super::date::span_days;
use super::stats::{cummax, kurtosis_pearson, pearson_r, quantile, skew};
use crate::factor::numeric::{nanmean, nanstd};

/// 单段回撤区间，对应上游 `_identify_drawdown_periods` 返回的一个 dict。
#[derive(Debug, Clone, PartialEq)]
pub struct DrawdownPeriod {
    /// 回撤开始的时间戳（首个跌破阈值的那一期）。
    pub start: String,
    /// 回撤结束的时间戳（首个回到阈值之上的那一期；未回本时为末期）。
    pub end: String,
    /// 区间内最深的回撤幅度，为负值。
    pub depth: f64,
    /// `start` 到 `end` 的日历天数；时间戳无法解析时为 0。
    pub duration_days: i64,
}

/// 一次回测的绩效指标，对应上游 `Backtester.metrics` 这个 dict。
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    /// 期末累计收益率。
    pub total_return: f64,
    /// 按日历天数折算的年化收益率。
    pub annual_return: f64,
    /// 年化波动率（收益率样本标准差 × `√365`）。
    pub annual_volatility: f64,
    /// 夏普比率，分母为年化波动率；波动率非正时为 0。
    pub sharpe_ratio: f64,
    /// 索提诺比率，分母只用负收益的年化波动率；下行波动非正时为 0。
    pub sortino_ratio: f64,
    /// 卡玛比率 = 年化收益 / |最大回撤|；最大回撤非负时为 0。
    pub calmar_ratio: f64,
    /// 最大回撤，为负值。
    pub max_drawdown: f64,
    /// 净值对时间序号回归的 R²，衡量净值曲线有多接近一条直线。
    pub linearity: f64,
    /// 收益率的 5% 分位数（历史 VaR）。
    pub var_95: f64,
    /// 不超过 `var_95` 的那部分收益率的均值（条件 VaR）。
    pub cvar: f64,
    /// 概率化夏普比率，取值落在 `[0, 1]`；调整项根号内为负时为 NaN。
    pub psr: f64,
}

/// 由收益率序列重建净值：`equity[i] = Π(1 + r[j])`，`j <= i`。
///
/// - 入参：`returns` `(时间戳, 收益率)` 序列。
/// - 加工：从 1 起逐期累乘 `1 + r`。
/// - 出参：等长的净值向量（不含起点 1）。
fn equity_from_returns(returns: &[(String, f64)]) -> Vec<f64> {
    let mut acc = 1.0;
    returns
        .iter()
        .map(|(_, r)| {
            acc *= 1.0 + r;
            acc
        })
        .collect()
}

/// 识别全部回撤区间，对应上游 `_identify_drawdown_periods`。
///
/// - 入参：`dates` 与 `equity` 等长的时间戳与净值。
/// - 加工：先算 `净值 / 历史最高 - 1`，逐期扫描——跌破 `-1e-6` 视为进入回撤，
///   回到阈值之上视为该段结束（**结束期取回本的那一期本身**，与上游一致）；
///   段内最深值作 `depth`，起止时间戳的日历天数差作 `duration_days`。
///   扫描结束时仍在回撤中，则以末期收尾。
/// - 出参：按 `depth` 升序（最深在前）的区间列表。
pub fn identify_drawdown_periods(dates: &[String], equity: &[f64]) -> Vec<DrawdownPeriod> {
    let peaks = cummax(equity);
    let dd: Vec<f64> = equity
        .iter()
        .zip(peaks.iter())
        .map(|(e, p)| e / p - 1.0)
        .collect();

    let mut periods: Vec<DrawdownPeriod> = Vec::new();
    let mut start: Option<usize> = None;
    let push = |periods: &mut Vec<DrawdownPeriod>, s: usize, e: usize| {
        let depth = dd[s..=e].iter().copied().fold(f64::INFINITY, f64::min);
        periods.push(DrawdownPeriod {
            start: dates[s].clone(),
            end: dates[e].clone(),
            depth,
            duration_days: span_days(&dates[s], &dates[e]).unwrap_or(0),
        });
    };
    for (i, v) in dd.iter().enumerate() {
        if *v < -1e-6 {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            push(&mut periods, s, i);
        }
    }
    if let Some(s) = start {
        push(&mut periods, s, dd.len() - 1);
    }
    periods.sort_by(|a, b| {
        a.depth
            .partial_cmp(&b.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    periods
}

/// 概率化夏普比率（PSR），对应上游 `Backtester._calculate_psr`。
///
/// - 入参：`r` 收益率序列；`sr_benchmark` 对标的夏普水平，上游固定传 `0.0`。
/// - 加工：先算观测夏普 `mean × 365 / (std × √365)`（`std` 为样本标准差），
///   再用偏度与 Pearson 峰度算调整项 `√(1 - skew·SR + ((kurt - 1) / 4)·SR²)`，
///   最后把 `(SR - 基准) / 调整项 × √(T / 365)` 过标准正态 CDF 并夹到 `[0, 1]`。
/// - 出参：落在 `[0, 1]` 的概率；样本不足 2 个时返回 `0.0`，调整项根号内为负时返回 NaN
///   （上游同此，`np.sqrt` 给 NaN 后一路传播）。
pub fn psr(r: &[f64], sr_benchmark: f64) -> f64 {
    if r.len() < 2 {
        return 0.0;
    }
    let std = nanstd(r, 1);
    let sr_obs = if std > 0.0 {
        nanmean(r) * 365.0 / (std * 365f64.sqrt())
    } else {
        0.0
    };
    let adjustment =
        (1.0 - skew(r) * sr_obs + ((kurtosis_pearson(r) - 1.0) / 4.0) * sr_obs * sr_obs).sqrt();
    let stat = (sr_obs - sr_benchmark) / adjustment * (r.len() as f64 / 365.0).sqrt();
    let p = crate::factor::numeric::norm_cdf(stat);
    if p.is_nan() {
        p
    } else {
        p.clamp(0.0, 1.0)
    }
}

/// 由收益率序列算出全部绩效指标，对应上游 `_calculate_performance_metrics` 加 PSR。
///
/// - 入参：`returns` `(时间戳, 收益率)` 序列；`risk_free_rate` 年化无风险利率
///   （上游默认 `0.03`）。
/// - 加工：重建净值 → 累计收益 → 用首末时间戳的**日历天数差**折年化收益
///   （时间戳无法解析时退化为用期数，同上游）→ 年化波动、夏普、最大回撤、卡玛、
///   净值线性度、下行波动与索提诺、VaR 与 CVaR → 最后算 PSR。
/// - 出参：`Some((指标, 回撤区间))`；收益率不足 2 期时返回 `None`（上游此时给空 dict）。
pub fn performance_metrics(
    returns: &[(String, f64)],
    risk_free_rate: f64,
) -> Option<(Metrics, Vec<DrawdownPeriod>)> {
    if returns.len() < 2 {
        return None;
    }
    let ann = 365.0_f64;
    let rs: Vec<f64> = returns.iter().map(|(_, r)| *r).collect();
    let dates: Vec<String> = returns.iter().map(|(d, _)| d.clone()).collect();
    let equity = equity_from_returns(returns);

    let total_return = equity[equity.len() - 1] - 1.0;
    // 与上游一致：索引是日期时按日历天数折算，否则退化为期数
    let days = span_days(&dates[0], &dates[dates.len() - 1]).unwrap_or(returns.len() as i64);
    let annual_return = if days > 0 {
        (1.0 + total_return).powf(ann / days as f64) - 1.0
    } else {
        0.0
    };

    let annual_volatility = nanstd(&rs, 1) * ann.sqrt();
    let sharpe_ratio = if annual_volatility > 0.0 {
        (annual_return - risk_free_rate) / annual_volatility
    } else {
        0.0
    };

    let peaks = cummax(&equity);
    let max_drawdown = equity
        .iter()
        .zip(peaks.iter())
        .map(|(e, p)| e / p - 1.0)
        .fold(f64::INFINITY, f64::min);
    let calmar_ratio = if max_drawdown < 0.0 {
        annual_return / max_drawdown.abs()
    } else {
        0.0
    };

    let t: Vec<f64> = (0..equity.len()).map(|i| i as f64).collect();
    let linearity = pearson_r(&t, &equity).powi(2);

    let downside: Vec<f64> = rs.iter().copied().filter(|x| *x < 0.0).collect();
    // 上游对空的下行序列取 0，单元素时 pandas 的 ddof=1 给 NaN，两者都会让分支落到 0
    let downside_vol = if downside.is_empty() {
        0.0
    } else {
        nanstd(&downside, 1) * ann.sqrt()
    };
    let sortino_ratio = if downside_vol > 0.0 {
        (annual_return - risk_free_rate) / downside_vol
    } else {
        0.0
    };

    let var_95 = quantile(&rs, 0.05);
    let tail: Vec<f64> = rs.iter().copied().filter(|x| *x <= var_95).collect();
    let cvar = if tail.is_empty() { 0.0 } else { nanmean(&tail) };

    let metrics = Metrics {
        total_return,
        annual_return,
        annual_volatility,
        sharpe_ratio,
        sortino_ratio,
        calmar_ratio,
        max_drawdown,
        linearity,
        var_95,
        cvar,
        psr: psr(&rs, 0.0),
    };
    Some((metrics, identify_drawdown_periods(&dates, &equity)))
}
