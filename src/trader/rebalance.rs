//! 组合再平衡：把目标权重变成一组市价单，对应上游 `Rebalancer` / `rebalance`。
//!
//! # 计划与执行是分开的
//!
//! ```text
//! Rebalancer::new(...).plan(&trader).await?   →  RebalancePlan   （只读，绝不发单）
//!     plan.preview()                          →  String          （纯文本，替代 rich 表格）
//!     plan.execute(&trader, Confirm::Yes)     →  RebalanceReport （唯一会发单的入口）
//! ```
//!
//! [`Confirm`] 只有一个变体 `Yes`——不能靠 `true` / `false` 蒙过去，读代码时一眼看得见哪一行
//! 会动真钱。上游的 `run()` 在 `preview=True` 时用 `input()` 等回车，非 TTY 环境会抛 `EOFError`；
//! 这里改成类型上的显式确认，库不碰标准输入。
//!
//! # 与上游的口径差异
//!
//! | 上游 | 本仓库 |
//! |---|---|
//! | 迭代 `set(...)`，下单顺序每个进程都不同 | 按标的名排序，可复现 |
//! | 平多判成 `flip`、平空判成 `close`，于是**平多不带 `reduceOnly`** | 目标为 0 一律是平仓且带 `reduceOnly` |
//! | 目标权重不做任何校验 | 计划里给出总目标名义额与 `budget` 的比，超 1 倍时在文本里标出来 |
//! | `minSz` 不校验，等交易所拒单 | 本地拦下，理由写进计划 |
//! | 权重按 `weight × budget` 直接给目标市值 | 同（不归一、不设杠杆上限，如实保留） |
//!
//! 最小调仓额沿用上游的 `MIN_TRADE_VALUE = 1.0`：`|目标 − 当前| < 1 USD` 就跳过。

use std::collections::{BTreeMap, BTreeSet};

use crate::net::HttpTransport;

use super::client::OkxTrader;
use super::contract::InstrumentSpec;
use super::types::{MarginMode, OrderAck, OrderRequest, OrderSide, Position, PositionMode};

/// 调仓额低于这个数（USD）就跳过，对应上游 `constants.MIN_TRADE_VALUE`。
pub const MIN_TRADE_VALUE: f64 = 1.0;

/// 合约 id 的默认后缀：USDT 本位永续。对应上游 `symbol_suffix='-USDT-SWAP'`。
pub const DEFAULT_SUFFIX: &str = "-USDT-SWAP";

/// 真发单前的显式确认。
///
/// 只有一个变体，所以 [`RebalancePlan::execute`] 的调用点必须写出
/// `Confirm::Yes`——不像布尔参数那样可以手滑传错。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    /// 我确认要把这份计划发到交易所。
    Yes,
}

/// 一个标的这一期该做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 调仓额不足 [`MIN_TRADE_VALUE`]，不动。
    Skip,
    /// 当前空仓、目标非零 → 开仓。
    Open,
    /// 同向加仓。
    Add,
    /// 同向减仓。
    Reduce,
    /// 目标为零 → 平掉（带 `reduceOnly`）。
    Close,
    /// 目标与当前反向 → 一张单直接翻过去。**只在单向持仓下成立。**
    Flip,
}

impl Action {
    /// 这个动作要不要带 `reduceOnly`。
    ///
    /// - 入参：无。
    /// - 加工：只有减仓与平仓带。
    /// - 出参：带则 `true`。
    ///
    /// 上游把「平掉多头」判成 `flip`、「平掉空头」判成 `close`，于是**平多不带
    /// `reduceOnly` 而平空带**——不对称，且平多那笔有冲成反向仓位的风险。本仓库按
    /// 「目标是不是 0」判定，两个方向一致。
    pub fn is_reduce_only(self) -> bool {
        matches!(self, Action::Reduce | Action::Close)
    }

    /// 用于文本报告的短标签。
    pub fn label(self) -> &'static str {
        match self {
            Action::Skip => "skip",
            Action::Open => "open",
            Action::Add => "add",
            Action::Reduce => "reduce",
            Action::Close => "close",
            Action::Flip => "flip",
        }
    }

    /// 按当前与目标名义额判定动作。
    ///
    /// - 入参：`current_usd` 当前净名义额（带符号）；`target_usd` 目标净名义额（带符号）。
    /// - 加工：先按 [`MIN_TRADE_VALUE`] 滤掉小额；再按「目标是否为 0」「是否同向」分支。
    /// - 出参：动作。
    ///
    /// 与上游 `_determine_action` 的差别只在平仓那一支：上游按当前仓位的正负分成 `flip` /
    /// `close`，这里按目标是否为 0 判，故平多平空对称。上游那个 `'none'` 分支是死代码
    /// （`current == target == 0` 会先被小额闸门拦成 `skip`），不移植。
    pub fn decide(current_usd: f64, target_usd: f64) -> Action {
        let diff = target_usd - current_usd;
        if diff.abs() < MIN_TRADE_VALUE {
            return Action::Skip;
        }
        if target_usd == 0.0 {
            return Action::Close;
        }
        if current_usd == 0.0 {
            return Action::Open;
        }
        if (current_usd > 0.0) != (target_usd > 0.0) {
            return Action::Flip;
        }
        // 同向：看绝对值是变大还是变小
        if target_usd.abs() > current_usd.abs() {
            Action::Add
        } else {
            Action::Reduce
        }
    }
}

/// 计划里的一条：某个标的这一期的目标、现状、动作与要发的单。
#[derive(Debug, Clone, PartialEq)]
pub struct Leg {
    /// 裸标的名，如 `"BTC"`。
    pub symbol: String,
    /// 合约 id，如 `"BTC-USDT-SWAP"`。
    pub inst_id: String,
    /// 目标权重（`weight × budget` 即目标名义额）。当前持有但不在目标里的标的为 `0`。
    pub weight: f64,
    /// 目标净名义额（USD，带符号）。
    pub target_usd: f64,
    /// 当前净名义额（USD，带符号）。
    pub current_usd: f64,
    /// 判定出来的动作。
    pub action: Action,
    /// 要发的单；`Skip` 或本地被拦下时为 `None`。
    pub order: Option<OrderRequest>,
    /// 这张单实际会成交的名义额（USD，绝对值）。向下取整后通常略小于 `|target − current|`。
    pub order_usd: f64,
    /// 为什么没有单：小额跳过、不足 `minSz`、合约不可交易、查不到规格等。
    pub note: Option<String>,
}

impl Leg {
    /// 调仓额（目标减当前，带符号）。
    pub fn delta_usd(&self) -> f64 {
        self.target_usd - self.current_usd
    }
}

/// 一份再平衡计划。**构造它不发任何单**，只读了持仓、规格与行情。
#[derive(Debug, Clone, PartialEq)]
pub struct RebalancePlan {
    /// 预算基数（USD），目标名义额 = `weight × budget`。
    pub budget: f64,
    /// 杠杆倍数，执行时对每个要下单的合约设一次。
    pub leverage: u32,
    /// 账户持仓模式，决定下单时 `posSide` 填什么。
    pub pos_mode: PositionMode,
    /// 逐标的的计划，按标的名排序（可复现）。
    pub legs: Vec<Leg>,
}

impl RebalancePlan {
    /// 会真发出去的那些单，顺序与 [`RebalancePlan::legs`] 一致。
    pub fn orders(&self) -> Vec<OrderRequest> {
        self.legs.iter().filter_map(|l| l.order.clone()).collect()
    }

    /// 当前持仓的总名义额（绝对值之和）。
    pub fn current_gross_usd(&self) -> f64 {
        self.legs.iter().map(|l| l.current_usd.abs()).sum()
    }

    /// 目标持仓的总名义额（绝对值之和）。
    pub fn target_gross_usd(&self) -> f64 {
        self.legs.iter().map(|l| l.target_usd.abs()).sum()
    }

    /// 目标总名义额相对预算的倍数。
    ///
    /// - 入参：无。
    /// - 加工：目标毛额除以预算。
    /// - 出参：倍数。上游对权重不做任何校验，`sum(|w|)` 给 2.0 就真的照 2 倍预算下单；
    ///   本仓库同样不拦，但把这个数摆在 [`RebalancePlan::preview`] 里，超 1 倍会标出来。
    pub fn gross_leverage(&self) -> f64 {
        if self.budget == 0.0 {
            return f64::NAN;
        }
        self.target_gross_usd() / self.budget
    }
}

/// 再平衡的入参与建计划逻辑，对应上游 `Rebalancer` 的构造与 `plan()`。
#[derive(Debug, Clone)]
pub struct Rebalancer {
    /// 目标权重：裸标的名 → 权重。`weight × budget` 即目标净名义额，负数表示做空。
    weights: BTreeMap<String, f64>,
    /// 预算基数（USD）。一般取 [`super::Balance::total_equity`]。
    budget: f64,
    /// 合约 id 后缀，默认 [`DEFAULT_SUFFIX`]。
    suffix: String,
    /// 杠杆倍数，默认 5（同上游）。
    leverage: u32,
}

impl Rebalancer {
    /// 构造。
    ///
    /// - 入参：`weights` 目标权重；`budget` 预算基数（USD，须为正）。
    /// - 加工：只做入参校验，不联网。
    /// - 出参：`Ok(Rebalancer)`；权重表为空、预算非正、或有权重是 NaN 时返回 `Err`。
    pub fn new(
        weights: impl IntoIterator<Item = (String, f64)>,
        budget: f64,
    ) -> Result<Rebalancer, String> {
        let weights: BTreeMap<String, f64> = weights.into_iter().collect();
        if weights.is_empty() {
            return Err("目标权重表不能为空".to_string());
        }
        if budget.is_nan() || budget <= 0.0 {
            return Err(format!("预算须为正数，给的是 {budget}"));
        }
        if let Some((s, _)) = weights.iter().find(|(_, w)| w.is_nan()) {
            return Err(format!("{s} 的权重是 NaN"));
        }
        Ok(Rebalancer {
            weights,
            budget,
            suffix: DEFAULT_SUFFIX.to_string(),
            leverage: 5,
        })
    }

    /// 换合约后缀并返回自身。
    pub fn suffix(mut self, suffix: impl Into<String>) -> Rebalancer {
        self.suffix = suffix.into();
        self
    }

    /// 换杠杆倍数并返回自身。
    pub fn leverage(mut self, leverage: u32) -> Rebalancer {
        self.leverage = leverage;
        self
    }

    /// 建计划。**只读，不发任何单。**
    ///
    /// - 入参：`trader` 客户端。
    /// - 加工：
    ///   1. 读账户配置并校验（`auto_fix = false`，不偷偷改账户设置）。
    ///   2. 读持仓，只保留后缀匹配的，按裸标的名归集当前净名义额。
    ///   3. 标的集 = 目标权重 ∪ 当前持仓，**按名排序**。
    ///   4. 对每个需要动的标的取合约规格与最新价，本地算张数：按 `lotSz` 向下对齐、
    ///      按 `minSz` 拦截、看 `state` 是否 `live`、看是否超 `maxMktSz`。
    /// - 出参：`Ok(RebalancePlan)`。任何读取失败都会硬失败——建计划阶段就该如此，
    ///   总比拿着半份数据去下单好。
    pub async fn plan<T: HttpTransport>(
        &self,
        trader: &OkxTrader<T>,
    ) -> Result<RebalancePlan, String> {
        let cfg = trader.validate_account_config(false).await?;
        let pos_mode = cfg
            .pos_mode
            .ok_or_else(|| format!("无法识别的持仓模式：{:?}", cfg.pos_mode_raw))?;

        let positions = trader.positions(None).await?;
        let current: BTreeMap<String, f64> = positions
            .iter()
            .filter(|p| p.inst_id.ends_with(&self.suffix))
            .map(|p: &Position| (p.base_symbol().to_string(), p.notional_usd))
            .collect();

        let symbols: BTreeSet<&str> = self
            .weights
            .keys()
            .map(String::as_str)
            .chain(current.keys().map(String::as_str))
            .collect();

        // 先定下每个标的的动作，再只为需要下单的那些取规格与行情
        let mut drafts = Vec::new();
        for sym in symbols {
            let weight = self.weights.get(sym).copied().unwrap_or(0.0);
            let target_usd = weight * self.budget;
            let current_usd = current.get(sym).copied().unwrap_or(0.0);
            drafts.push((
                sym.to_string(),
                format!("{sym}{}", self.suffix),
                weight,
                target_usd,
                current_usd,
                Action::decide(current_usd, target_usd),
            ));
        }
        let need: Vec<String> = drafts
            .iter()
            .filter(|d| d.5 != Action::Skip)
            .map(|d| d.1.clone())
            .collect();
        let market = trader.specs_and_prices(&need).await?;

        let legs = drafts
            .into_iter()
            .map(
                |(symbol, inst_id, weight, target_usd, current_usd, action)| {
                    let (order, order_usd, note) =
                        build_order(&inst_id, target_usd, current_usd, action, &market);
                    Leg {
                        symbol,
                        inst_id,
                        weight,
                        target_usd,
                        current_usd,
                        action,
                        order,
                        order_usd,
                        note,
                    }
                },
            )
            .collect();

        Ok(RebalancePlan {
            budget: self.budget,
            leverage: self.leverage,
            pos_mode,
            legs,
        })
    }
}

/// 为一条 leg 造单，或说明为什么造不出来。
///
/// - 入参：`inst_id` 合约；`target_usd` / `current_usd` 目标与当前净名义额；`action` 动作；
///   `market` 合约规格与最新价。
/// - 加工：`Skip` 直接跳过；否则按 `|目标 − 当前|` 与最新价算张数——`lotSz` 向下对齐、
///   `minSz` 本地拦截、`state != "live"` 拦截、超 `maxMktSz` 拦截。
/// - 出参：`(单, 实际名义额, 备注)`。造不出单时前两项为 `None` / `0`，备注写清原因。
fn build_order(
    inst_id: &str,
    target_usd: f64,
    current_usd: f64,
    action: Action,
    market: &BTreeMap<String, (InstrumentSpec, f64)>,
) -> (Option<OrderRequest>, f64, Option<String>) {
    if action == Action::Skip {
        return (
            None,
            0.0,
            Some(format!(
                "调仓额 {:.4} USD 不足 {MIN_TRADE_VALUE}，跳过",
                (target_usd - current_usd).abs()
            )),
        );
    }
    let Some((spec, price)) = market.get(inst_id) else {
        return (None, 0.0, Some(format!("{inst_id} 查不到合约规格或行情")));
    };
    if !spec.is_live() {
        return (
            None,
            0.0,
            Some(format!("{inst_id} 当前不可交易（state = {}）", spec.state)),
        );
    }
    let delta = target_usd - current_usd;
    match spec.contracts_for_notional(delta.abs(), *price) {
        Err(why) => (None, 0.0, Some(why)),
        Ok(contracts) if spec.exceeds_market_cap(contracts) => (
            None,
            0.0,
            Some(format!(
                "{inst_id}：{contracts} 张超过单笔市价单上限 {} 张",
                spec.max_mkt_sz
            )),
        ),
        Ok(contracts) => {
            let side = OrderSide::from_delta(delta);
            let order = OrderRequest::market(spec, side, contracts, action.is_reduce_only());
            (Some(order), spec.notional_of(contracts, *price), None)
        }
    }
}

/// 执行结果，对应上游 `rebalance_portfolio` 返回的那个 dict。
#[derive(Debug, Clone, PartialEq)]
pub struct RebalanceReport {
    /// 逐单结果，顺序与 [`RebalancePlan::orders`] 一致。
    pub acks: Vec<OrderAck>,
    /// 设杠杆失败的合约与原因。**设杠杆失败不阻止下单**——杠杆只影响保证金占用，
    /// 上游则是直接把该标的整个跳过。
    pub leverage_errors: Vec<(String, String)>,
}

impl RebalanceReport {
    /// 成功的单数。
    pub fn successful(&self) -> usize {
        self.acks.iter().filter(|a| a.is_success()).count()
    }

    /// 失败的单数。
    pub fn failed(&self) -> usize {
        self.acks.len() - self.successful()
    }

    /// 是否全部成功（且至少发出过一张）。
    pub fn all_ok(&self) -> bool {
        !self.acks.is_empty() && self.failed() == 0
    }

    /// 文本报告。
    ///
    /// - 入参：无。
    /// - 加工：逐单一行，末尾汇总；设杠杆的失败单列一段。
    /// - 出参：多行字符串（不打印，由调用方决定去向，与仓库其余 `summary()` 一致）。
    pub fn summary(&self) -> String {
        let mut lines = vec![format!(
            "RebalanceReport(orders={}, ok={}, failed={})",
            self.acks.len(),
            self.successful(),
            self.failed()
        )];
        for a in &self.acks {
            let flag = if a.is_success() { "OK  " } else { "FAIL" };
            lines.push(format!(
                "  [{flag}] clOrdId={} ordId={} code={} {}",
                a.cl_ord_id, a.ord_id, a.code, a.msg
            ));
        }
        if !self.leverage_errors.is_empty() {
            lines.push("  设杠杆失败（不影响下单）：".to_string());
            for (id, why) in &self.leverage_errors {
                lines.push(format!("    {id}: {why}"));
            }
        }
        lines.join("\n")
    }
}

impl RebalancePlan {
    /// 纯文本预览，替代上游的 `rich` 表格。**不发单。**
    ///
    /// - 入参：无。
    /// - 加工：表头一行摘要，逐标的一行，最后合计。列宽固定，money 列右对齐。
    /// - 出参：多行字符串。排版风格与 [`crate::backtest::Backtester::summary`] 一致。
    pub fn preview(&self) -> String {
        let gross = self.gross_leverage();
        let mut lines = vec![
            format!(
                "RebalancePlan(budget=${:.2}, leverage={}x, pos_mode={}, orders={})",
                self.budget,
                self.leverage,
                self.pos_mode.as_str(),
                self.orders().len()
            ),
            format!(
                "  当前毛敞口 ${:.2} → 目标毛敞口 ${:.2}（{:.2}× 预算{}）",
                self.current_gross_usd(),
                self.target_gross_usd(),
                gross,
                if gross > 1.0 + 1e-9 {
                    "，注意已超过 1 倍"
                } else {
                    ""
                }
            ),
            String::new(),
            format!(
                "  {:<8}{:>10}{:>13}{:>13}{:>13}{:>9}{:>13}",
                "Symbol", "Weight", "Current", "Target", "Delta", "Action", "OrderUSD"
            ),
            format!("  {}", "-".repeat(79)),
        ];
        for l in &self.legs {
            lines.push(format!(
                "  {:<8}{:>10}{:>13}{:>13}{:>13}{:>9}{:>13}",
                trunc(&l.symbol, 8),
                format!("{:+.4}", l.weight),
                money(l.current_usd),
                money(l.target_usd),
                money(l.delta_usd()),
                l.action.label(),
                if l.order.is_some() {
                    money(l.order_usd)
                } else {
                    "-".to_string()
                },
            ));
            if let Some(note) = &l.note {
                lines.push(format!("           ↳ {note}"));
            }
        }
        lines.push(format!("  {}", "-".repeat(79)));
        lines.push(format!(
            "  合计 {} 条，其中 {} 条要下单",
            self.legs.len(),
            self.orders().len()
        ));
        lines.join("\n")
    }

    /// **真发单。** 这是本模块唯一会动真钱的入口。
    ///
    /// - 入参：`trader` 客户端；`_confirm` 显式确认，只能传 [`Confirm::Yes`]。
    /// - 加工：先对每个要下单的合约设一次杠杆（失败只记账、不阻止下单——杠杆只影响保证金
    ///   占用），再把全部单交给 [`OkxTrader::place_orders`]，由它按 20 张切批。
    /// - 出参：`Ok(RebalanceReport)`；整批被拒（连 `data[]` 都没有）时返回 `Err`。
    ///   逐单成败看 [`OrderAck::is_success`]，**部分失败不算整体失败**。
    ///
    /// 没有单要发时直接返回空报告，不打任何请求。
    pub async fn execute<T: HttpTransport>(
        &self,
        trader: &OkxTrader<T>,
        _confirm: Confirm,
    ) -> Result<RebalanceReport, String> {
        let orders = self.orders();
        if orders.is_empty() {
            return Ok(RebalanceReport {
                acks: Vec::new(),
                leverage_errors: Vec::new(),
            });
        }
        let mut leverage_errors = Vec::new();
        for o in &orders {
            if let Err(why) = trader
                .set_leverage(&o.inst_id, self.leverage, MarginMode::Cross, None)
                .await
            {
                leverage_errors.push((o.inst_id.clone(), why));
            }
        }
        let acks = trader.place_orders(&orders, self.pos_mode).await?;
        Ok(RebalanceReport {
            acks,
            leverage_errors,
        })
    }
}

/// 按 Unicode 字符数截断。
fn trunc(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// 金额渲染：负数用 `-$` 前缀，与上游 `fmt_usd` 一致。
fn money(v: f64) -> String {
    if v < 0.0 {
        format!("-${:.2}", v.abs())
    } else {
        format!("${v:.2}")
    }
}

/// 便捷入口：建计划并立刻打印预览，对应上游 `rebalance(..., auto_run=False)`。
///
/// - 入参：`trader` 客户端；`weights` 目标权重；`budget` 预算基数。
/// - 加工：[`Rebalancer::new`] → [`Rebalancer::plan`]。
/// - 出参：`Ok(RebalancePlan)`。**刻意不发单**——上游的 `rebalance()` 默认
///   `auto_run=True` 会直接执行（`preview=True` 时靠 `input()` 拦一下），这里把执行留给
///   调用方显式调 [`RebalancePlan::execute`]。
pub async fn rebalance<T: HttpTransport>(
    trader: &OkxTrader<T>,
    weights: impl IntoIterator<Item = (String, f64)>,
    budget: f64,
) -> Result<RebalancePlan, String> {
    Rebalancer::new(weights, budget)?.plan(trader).await
}
