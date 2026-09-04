//! 事件驱动回测引擎，对应上游 `backtest.Backtester` 与 `backtest.backtest`。
//!
//! 调仓节奏是 T+1：第 `i` 期用第 `i-1` 期的因子值算目标市值，按第 `i` 期的价格成交。
//! 净值一天记一条，记录点在**当日成交之前**（与上游第一次 `update_market_value` 对齐）。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::date::shift_days;
use super::metrics::{performance_metrics, DrawdownPeriod, Metrics};
use super::portfolio::{Portfolio, Trade};
use super::stats::cummax;
use crate::factor::Factor;

/// 逐期的横截面快照：`标的 → 取值`。
type Cross = BTreeMap<String, f64>;

/// 目标市值的中性化方式，对应上游 `Backtester(neutralization=...)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Neutralization {
    /// 因子值直接乘净值当目标市值，不做任何调整。对应上游 `"none"`。
    None,
    /// 先去横截面均值再按绝对值和归一，保证多空市值抵消。对应上游默认的 `"market"`。
    #[default]
    Market,
}

impl Neutralization {
    /// 由字符串解析。
    ///
    /// - 入参：`s` 方式名，大小写不敏感。
    /// - 加工：与 `"none"` / `"market"` 匹配。
    /// - 出参：匹配成功返回枚举值；否则返回含可选值清单的错误。
    ///
    /// 与上游不同：上游只特判 `"none"`，其余任何字符串（含拼错的）都静默按市场中性处理。
    pub fn parse(s: &str) -> Result<Neutralization, String> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(Neutralization::None),
            "market" => Ok(Neutralization::Market),
            other => Err(format!(
                "Invalid neutralization: {other}. Must be one of ['none', 'market']"
            )),
        }
    }
}

/// 因子策略回测引擎。
///
/// 用建造者方式配置后调 [`Backtester::run`] 与 [`Backtester::calculate_metrics`]：
///
/// ```no_run
/// # use phandas_rs::factor::Panel;
/// # use phandas_rs::backtest::Backtester;
/// # fn main() -> Result<(), String> {
/// # let panel = Panel::from_csv("panel.csv")?;
/// let open = panel.factor("open")?;
/// let alpha = panel.factor("close")?.rank().signal();
/// let bt = Backtester::new(&open, &alpha).run()?.calculate_metrics(0.03);
/// println!("{}", bt.summary());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Backtester {
    strategy_name: String,
    price_name: String,
    cost_rates: (f64, f64),
    full_rebalance: bool,
    neutralization: Neutralization,
    /// 价格因子的逐期快照，已剔除含 NaN 的期。
    price_cache: BTreeMap<String, Cross>,
    /// 策略因子的逐期快照，已剔除含 NaN 的期。
    strategy_cache: BTreeMap<String, Cross>,
    /// 策略因子每期是否已经是美元中性信号，按**原始**数据判定（含 NaN 的期也判）。
    signal_flags: BTreeMap<String, bool>,
    /// 两个因子时间戳的交集，升序。
    common_dates: Vec<String>,
    /// 因含 NaN 而被跳过、且发生在首个有效期之后的时间戳。
    skipped_dates: Vec<String>,
    portfolio: Portfolio,
    metrics: Option<Metrics>,
    drawdown_periods: Vec<DrawdownPeriod>,
}

/// 把因子拆成逐期快照，含 NaN 的期整期丢弃。
///
/// - 入参：`factor` 待拆的因子。
/// - 加工：逐期取横截面 → 该期任一标的为 NaN 就整期不入缓存，并在已出现过有效期之后
///   记入跳过名单 → 否则装成 `标的 → 取值` 的映射。
/// - 出参：`(逐期缓存, 跳过的时间戳)`。语义照抄上游 `_build_date_cache`。
fn build_date_cache(factor: &Factor) -> (BTreeMap<String, Cross>, Vec<String>) {
    let mut cache: BTreeMap<String, Cross> = BTreeMap::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut seen_valid = false;
    for (ti, ts) in factor.timestamps().iter().enumerate() {
        let row = factor.row(ti);
        if row.iter().any(|v| v.is_nan()) {
            if seen_valid {
                skipped.push(ts.clone());
            }
            continue;
        }
        let cross = factor
            .symbols()
            .iter()
            .zip(row.iter())
            .map(|(s, v)| (s.clone(), *v))
            .collect();
        cache.insert(ts.clone(), cross);
        seen_valid = true;
    }
    (cache, skipped)
}

impl Backtester {
    /// 建账。默认双边手续费 `0.0003`、初始资金 `1000`、不做全量调仓、市场中性。
    ///
    /// - 入参：`entry_price` 成交价因子（上游常用 `open`）；`strategy` 策略因子。
    /// - 加工：把两个因子各自拆成逐期快照（含 NaN 的期丢弃）→ 逐期预判策略因子是否
    ///   已是美元中性信号 → 求两者时间戳的交集。此处只做准备，不校验数据是否够跑，
    ///   校验留给 [`Backtester::run`]（与上游构造函数一致）。
    /// - 出参：待运行的 [`Backtester`]。
    pub fn new(entry_price: &Factor, strategy: &Factor) -> Backtester {
        let (price_cache, _) = build_date_cache(entry_price);
        let (strategy_cache, skipped_dates) = build_date_cache(strategy);
        let signal_flags = strategy
            .timestamps()
            .iter()
            .map(|ts| (ts.clone(), strategy.is_signal(Some(ts))))
            .collect();
        let price_dates: BTreeSet<&str> = entry_price
            .timestamps()
            .iter()
            .map(String::as_str)
            .collect();
        let common_dates = strategy
            .timestamps()
            .iter()
            .filter(|ts| price_dates.contains(ts.as_str()))
            .cloned()
            .collect();

        Backtester {
            strategy_name: strategy.name.clone(),
            price_name: entry_price.name.clone(),
            cost_rates: (0.0003, 0.0003),
            full_rebalance: false,
            neutralization: Neutralization::Market,
            price_cache,
            strategy_cache,
            signal_flags,
            common_dates,
            skipped_dates,
            portfolio: Portfolio::new(1000.0),
            metrics: None,
            drawdown_periods: Vec::new(),
        }
    }

    /// 设双边手续费率，对应上游 `transaction_cost=(buy, sell)`。
    ///
    /// - 入参：`buy` 买入费率；`sell` 卖出费率。
    /// - 加工：仅记录，成交时按方向取用。
    /// - 出参：配置后的自身，可继续链式调用。
    pub fn transaction_cost(mut self, buy: f64, sell: f64) -> Backtester {
        self.cost_rates = (buy, sell);
        self
    }

    /// 设初始资金，对应上游 `initial_capital`。
    ///
    /// - 入参：`v` 初始资金。
    /// - 加工：重建一个空组合（必须在 [`Backtester::run`] 之前调用）。
    /// - 出参：配置后的自身。
    pub fn initial_capital(mut self, v: f64) -> Backtester {
        self.portfolio = Portfolio::new(v);
        self
    }

    /// 是否每期先清空全部持仓再按目标建仓，对应上游 `full_rebalance`。
    ///
    /// - 入参：`v` 打开则每期先按当期价平掉所有持仓。
    /// - 加工：仅记录。
    /// - 出参：配置后的自身。注意打开后手续费会显著变高。
    pub fn full_rebalance(mut self, v: bool) -> Backtester {
        self.full_rebalance = v;
        self
    }

    /// 设目标市值的中性化方式，对应上游 `neutralization`。
    ///
    /// - 入参：`v` 见 [`Neutralization`]。
    /// - 加工：仅记录。
    /// - 出参：配置后的自身。
    pub fn neutralization(mut self, v: Neutralization) -> Backtester {
        self.neutralization = v;
        self
    }

    /// 跑完整个回测。
    ///
    /// - 入参：无（全部配置已在建造阶段给定）。
    /// - 加工：定位起始期 → 播种一条日期为起始期前一天的初始净值 → 逐期：
    ///   按当期价重估并记一条净值 → 用**前一期**因子算目标市值 →
    ///   `full_rebalance` 时先清仓再重估 → 按目标与现持仓之差下单。
    ///   当期价格缺失（含 NaN 被丢弃）时整期跳过，不记净值也不交易。
    /// - 出参：`Ok(跑完的自身)`；交集不足 2 期、或找不到"前一期有因子且当期有价格"的
    ///   起始点时返回 `Err`。
    pub fn run(mut self) -> Result<Backtester, String> {
        if self.common_dates.len() < 2 {
            return Err("Insufficient overlapping dates for backtesting".to_string());
        }
        let start_idx = self
            .find_start_date()
            .ok_or_else(|| "No valid start date found with overlapping data".to_string())?;

        let first = self.common_dates[start_idx].clone();
        let seed = shift_days(&first, -1).unwrap_or_else(|| first.clone());
        self.portfolio.record(&seed);

        for i in start_idx..self.common_dates.len() {
            let current_date = self.common_dates[i].clone();
            let prev_date = self.common_dates[i - 1].clone();
            let Some(prices) = self.price_cache.get(&current_date).cloned() else {
                continue;
            };
            if prices.is_empty() {
                continue;
            }

            self.portfolio.revalue(&prices);
            self.portfolio.record(&current_date);

            let target = self.target_holdings(&prev_date);

            if self.full_rebalance {
                for (symbol, qty) in self.portfolio.positions().clone() {
                    if let Some(px) = prices.get(&symbol) {
                        self.portfolio.execute_trade(
                            &symbol,
                            -qty,
                            *px,
                            self.cost_rates,
                            &current_date,
                        );
                    }
                }
                // 只重估、不再记一条净值：上游此处会写第二条，导致同一天两个净值
                self.portfolio.revalue(&prices);
            }

            for (symbol, qty) in self.generate_orders(&target, &prices) {
                let px = prices[&symbol];
                self.portfolio
                    .execute_trade(&symbol, qty, px, self.cost_rates, &current_date);
            }
        }
        Ok(self)
    }
    /// 算绩效指标，对应上游 `calculate_metrics`。
    ///
    /// - 入参：`risk_free_rate` 年化无风险利率，上游默认 `0.03`。
    /// - 加工：把净值历史转成逐期收益率（`pct_change` 后丢掉首个 NaN）→ 交给
    ///   `metrics::performance_metrics` 算 11 项指标与回撤区间。
    /// - 出参：填好指标的自身；净值历史不足 2 条时指标保持 `None`。
    pub fn calculate_metrics(mut self, risk_free_rate: f64) -> Backtester {
        let returns = self.returns();
        match performance_metrics(&returns, risk_free_rate) {
            Some((m, periods)) => {
                self.metrics = Some(m);
                self.drawdown_periods = periods;
            }
            None => {
                self.metrics = None;
                self.drawdown_periods = Vec::new();
            }
        }
        self
    }

    /// 起始期下标：第一个满足"前一期有策略数据、当期有价格数据"的位置。
    ///
    /// - 入参：无。
    /// - 加工：从下标 1 起逐个试，两侧都要求缓存里存在且非空。
    /// - 出参：`Some(下标)`；找不到时 `None`。对应上游 `_find_start_date`。
    fn find_start_date(&self) -> Option<usize> {
        (1..self.common_dates.len()).find(|&i| {
            let has_strategy = self
                .strategy_cache
                .get(&self.common_dates[i - 1])
                .is_some_and(|c| !c.is_empty());
            let has_price = self
                .price_cache
                .get(&self.common_dates[i])
                .is_some_and(|c| !c.is_empty());
            has_strategy && has_price
        })
    }

    /// 由前一期因子值算本期目标市值，对应上游 `_calculate_target_holdings`。
    ///
    /// - 入参：`prev_date` 取因子值的那一期。
    /// - 加工：`Neutralization::None` 时因子值直接乘净值；该期因子已是美元中性信号时
    ///   同样直接乘；否则去均值后按绝对值和归一再乘净值，绝对值和小于 `1e-10` 时全 0。
    /// - 出参：`标的 → 目标市值`。前一期因子缺失（含 NaN 被丢弃）时返回空表，
    ///   下游会因此清空全部持仓——这是上游行为，如实保留。
    fn target_holdings(&self, prev_date: &str) -> Cross {
        let tv = self.portfolio.total_value();
        let empty = Cross::new();
        let factors = self.strategy_cache.get(prev_date).unwrap_or(&empty);
        if factors.is_empty() {
            return Cross::new();
        }
        let as_is = self.neutralization == Neutralization::None
            || self.signal_flags.get(prev_date).copied().unwrap_or(false);
        if as_is {
            return factors.iter().map(|(s, v)| (s.clone(), v * tv)).collect();
        }
        let mean = factors.values().sum::<f64>() / factors.len() as f64;
        let abs_sum: f64 = factors.values().map(|v| (v - mean).abs()).sum();
        if abs_sum < 1e-10 {
            return factors.keys().map(|s| (s.clone(), 0.0)).collect();
        }
        factors
            .iter()
            .map(|(s, v)| (s.clone(), (v - mean) / abs_sum * tv))
            .collect()
    }

    /// 由目标市值与现持仓市值算出下单数量，对应上游 `_generate_orders`。
    ///
    /// - 入参：`target` 目标市值；`prices` 当期价格。
    /// - 加工：取目标与现持仓标的的并集 → 当期没报价的标的跳过 →
    ///   成交金额 = 目标 − 现持仓，绝对值超过 `1e-10` 才生成订单 → 除以价格得数量。
    /// - 出参：`标的 → 成交数量`（正买负卖）。
    fn generate_orders(&self, target: &Cross, prices: &Cross) -> Cross {
        let current = self.portfolio.holdings();
        let symbols: BTreeSet<&String> = target.keys().chain(current.keys()).collect();
        let mut orders = Cross::new();
        for symbol in symbols {
            let Some(px) = prices.get(symbol) else {
                continue;
            };
            let trade_value = target.get(symbol).copied().unwrap_or(0.0)
                - current.get(symbol).copied().unwrap_or(0.0);
            if trade_value.abs() > 1e-10 {
                orders.insert(symbol.clone(), trade_value / px);
            }
        }
        orders
    }
    /// 净值序列，`(时间戳, 净值)`，首条是起始期前一天的初始资金。
    pub fn equity(&self) -> &[(String, f64)] {
        self.portfolio.history()
    }

    /// 逐期收益率，对应 `pandas.Series.pct_change().dropna()`。
    ///
    /// - 入参：无。
    /// - 加工：相邻净值算 `本期 / 上期 - 1`，首期无前值故不产出。
    /// - 出参：长度为净值条数减一的 `(时间戳, 收益率)` 序列。
    pub fn returns(&self) -> Vec<(String, f64)> {
        self.equity()
            .windows(2)
            .map(|w| (w[1].0.clone(), w[1].1 / w[0].1 - 1.0))
            .collect()
    }

    /// 回撤序列 `净值 / 历史最高 - 1`，基于组合自身的净值历史。
    pub fn drawdown(&self) -> Vec<(String, f64)> {
        let values: Vec<f64> = self.equity().iter().map(|(_, v)| *v).collect();
        let peaks = cummax(&values);
        self.equity()
            .iter()
            .zip(peaks.iter())
            .map(|((d, v), p)| (d.clone(), v / p - 1.0))
            .collect()
    }

    /// 每日换手率 = 当日成交金额绝对值之和 / 当日净值。
    ///
    /// - 入参：无。
    /// - 加工：按日汇总成交金额绝对值 → 与净值历史按日期取交集（播种那天没有成交，
    ///   因此不出现在结果里）→ 相除。
    /// - 出参：按日期升序的 `(时间戳, 换手率)`。
    pub fn turnover(&self) -> Vec<(String, f64)> {
        let mut daily: BTreeMap<&str, f64> = BTreeMap::new();
        for t in self.portfolio.trade_log() {
            *daily.entry(t.date.as_str()).or_insert(0.0) += t.trade_value.abs();
        }
        let nav: BTreeMap<&str, f64> = self
            .equity()
            .iter()
            .map(|(d, v)| (d.as_str(), *v))
            .collect();
        daily
            .into_iter()
            .filter_map(|(d, tv)| nav.get(d).map(|v| (d.to_string(), tv / v)))
            .collect()
    }

    /// 等权买入持有基准，对应上游 `_calculate_benchmark_equity`。
    ///
    /// - 入参：无。
    /// - 加工：用第一个真实交易日的价格把初始资金等分买入各标的，此后按价格缓存逐期重估，
    ///   期间不再调仓；当期没报价的标的不计入。
    /// - 出参：`(时间戳, 基准净值)`；净值历史不足 2 条或首个交易日无价格时返回空。
    pub fn benchmark_equity(&self) -> Vec<(String, f64)> {
        let history = self.equity();
        if history.len() < 2 {
            return Vec::new();
        }
        let first_date = history[1].0.as_str();
        let Some(prices_first) = self.price_cache.get(first_date) else {
            return Vec::new();
        };
        if prices_first.is_empty() {
            return Vec::new();
        }
        let alloc = self.portfolio.initial_capital() / prices_first.len() as f64;
        let holdings: Cross = prices_first
            .iter()
            .map(|(s, px)| (s.clone(), alloc / px))
            .collect();

        self.price_cache
            .range(first_date.to_string()..)
            .filter(|(_, prices)| !prices.is_empty())
            .map(|(date, prices)| {
                let value = holdings
                    .iter()
                    .filter_map(|(s, qty)| prices.get(s).map(|px| qty * px))
                    .sum();
                (date.clone(), value)
            })
            .collect()
    }

    /// 成交流水，按成交顺序。
    pub fn trades(&self) -> &[Trade] {
        self.portfolio.trade_log()
    }

    /// 绩效指标；未调 [`Backtester::calculate_metrics`] 或净值不足 2 条时为 `None`。
    pub fn metrics(&self) -> Option<&Metrics> {
        self.metrics.as_ref()
    }

    /// 回撤区间明细，按深度升序（最深在前）。
    pub fn drawdown_periods(&self) -> &[DrawdownPeriod] {
        &self.drawdown_periods
    }

    /// 因含 NaN 被整期丢弃、且发生在首个有效期之后的策略因子时间戳。
    ///
    /// 上游在这里发 `warnings.warn`，此处改为留给调用方自行决定怎么提示。
    pub fn skipped_dates(&self) -> &[String] {
        &self.skipped_dates
    }

    /// 组合本体，可进一步读现金、持仓与流水。
    pub fn portfolio(&self) -> &Portfolio {
        &self.portfolio
    }
    /// 概要报告，排版与上游 `summary()` 一致。
    ///
    /// - 入参：无。
    /// - 加工：取首末净值日期、策略名与 11 项指标，另把日均换手率年化（× 365）；
    ///   百分比按两位小数、比率按两位小数、线性度按四位小数排版。
    /// - 出参：多行可打印字符串；未算指标时给一行提示。与 [`crate::factor::Factor::info`]
    ///   一致，只返回文本不直接输出。
    pub fn summary(&self) -> String {
        let Some(m) = &self.metrics else {
            return "Backtester(no metrics available)".to_string();
        };
        let history = self.equity();
        if history.is_empty() {
            return "Backtester(no data)".to_string();
        }
        let turnover = self.turnover();
        let avg_turnover = if turnover.is_empty() {
            0.0
        } else {
            turnover.iter().map(|(_, v)| *v).sum::<f64>() / turnover.len() as f64 * 365.0
        };
        let pct = |v: f64, p: usize| format!("{:>7.*}%", p, v * 100.0);
        format!(
            "Backtester(strategy='{}', period={} to {})\n  \
             total_return:   {}    annual_return:  {}\n  \
             sharpe_ratio:   {:>8.2}    sortino_ratio:  {:>8.2}\n  \
             calmar_ratio:   {:>8.2}    max_drawdown:   {}\n  \
             linearity:      {:>8.4}    psr:            {}\n  \
             var_95:         {}    cvar:           {}\n  \
             turnover:       {}",
            self.strategy_name,
            history[0].0,
            history[history.len() - 1].0,
            pct(m.total_return, 2),
            pct(m.annual_return, 2),
            m.sharpe_ratio,
            m.sortino_ratio,
            m.calmar_ratio,
            pct(m.max_drawdown, 2),
            m.linearity,
            pct(m.psr, 1),
            pct(m.var_95, 2),
            pct(m.cvar, 2),
            pct(avg_turnover, 2),
        )
    }

    /// 最深的若干段回撤明细，排版与上游 `print_drawdown_periods` 一致。
    ///
    /// - 入参：`top_n` 最多列出的段数。
    /// - 加工：取按深度排序后的前 `top_n` 段，逐行给起止、深度与持续天数；
    ///   总段数超过 `top_n` 时末尾补一行计数。
    /// - 出参：多行可打印字符串；没有回撤段时给一行提示。
    pub fn drawdown_report(&self, top_n: usize) -> String {
        if self.drawdown_periods.is_empty() {
            return "Drawdown Periods: none detected".to_string();
        }
        let total = self.drawdown_periods.len();
        let shown = top_n.min(total);
        let mut out = format!("Drawdown Periods (top {shown}):\n");
        for (i, p) in self.drawdown_periods.iter().take(top_n).enumerate() {
            out.push_str(&format!(
                "  {}. {} to {}    depth={:.2}%    duration={}d\n",
                i + 1,
                p.start,
                p.end,
                p.depth * 100.0,
                p.duration_days
            ));
        }
        if total > top_n {
            out.push_str(&format!("  (showing {top_n} of {total} periods)\n"));
        }
        out
    }
}

impl fmt::Display for Backtester {
    /// - 入参：`f` 格式化器。
    /// - 加工：有净值历史时给策略名、起止日期与期数；否则退化为给策略名、价格因子名与费率。
    /// - 出参：形如 `Backtester(strategy=..., period=... to ..., days=...)` 的一行文本。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let history = self.equity();
        if history.is_empty() {
            write!(
                f,
                "Backtester(strategy={}, entry_price={}, cost={:.3}%)",
                self.strategy_name,
                self.price_name,
                self.cost_rates.0 * 100.0
            )
        } else {
            write!(
                f,
                "Backtester(strategy={}, period={} to {}, days={})",
                self.strategy_name,
                history[0].0,
                history[history.len() - 1].0,
                history.len()
            )
        }
    }
}

/// 一步跑完回测，对应上游 `backtest(...)` 在 `auto_run=True` 下的行为。
///
/// - 入参：`entry_price` 成交价因子；`strategy` 策略因子。
/// - 加工：按默认配置建账 → [`Backtester::run`] → 以 `0.03` 的无风险利率算指标。
/// - 出参：`Ok(跑完并算好指标的 Backtester)`；数据不足时返回 `Err`。
///   需要改手续费、初始资金等参数时直接用 [`Backtester::new`] 的链式配置。
pub fn backtest(entry_price: &Factor, strategy: &Factor) -> Result<Backtester, String> {
    Ok(Backtester::new(entry_price, strategy)
        .run()?
        .calculate_metrics(0.03))
}
