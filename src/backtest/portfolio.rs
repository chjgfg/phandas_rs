//! 组合记账：现金、持仓、成交流水与按市价重估，对应上游 `backtest.Portfolio`。

use std::collections::BTreeMap;

/// 一笔成交，对应上游 `Portfolio.trade_log` 里的一条记录。
#[derive(Debug, Clone, PartialEq)]
pub struct Trade {
    /// 成交日期。
    pub date: String,
    /// 标的名。
    pub symbol: String,
    /// 成交金额 = 数量 × 价格，买入为正、卖出为负。
    pub trade_value: f64,
    /// 该笔手续费，恒为非负。
    pub cost: f64,
}

/// 组合状态：现金、按标的的持仓数量与市值、净值历史、成交流水。
#[derive(Debug, Clone)]
pub struct Portfolio {
    initial_capital: f64,
    cash: f64,
    positions: BTreeMap<String, f64>,
    holdings: BTreeMap<String, f64>,
    total_value: f64,
    history: Vec<(String, f64)>,
    trade_log: Vec<Trade>,
}

impl Portfolio {
    /// 以初始资金开仓建账。
    ///
    /// - 入参：`initial_capital` 初始资金，全部记为现金。
    /// - 加工：现金与净值都置为初始资金，持仓与流水为空。
    /// - 出参：全新的 [`Portfolio`]。
    pub fn new(initial_capital: f64) -> Portfolio {
        Portfolio {
            initial_capital,
            cash: initial_capital,
            positions: BTreeMap::new(),
            holdings: BTreeMap::new(),
            total_value: initial_capital,
            history: Vec::new(),
            trade_log: Vec::new(),
        }
    }

    /// 按给定价格重估持仓市值与净值，**不写净值历史**。
    ///
    /// - 入参：`prices` 当期 `标的 → 价格`。
    /// - 加工：清空并重建 `holdings`——只有在 `prices` 里报到价的标的才计入市值
    ///   （其余仍留在 `positions` 但不计入净值，与上游一致）→ 净值 = 现金 + 持仓市值。
    /// - 出参：无（就地更新）。
    pub fn revalue(&mut self, prices: &BTreeMap<String, f64>) {
        self.holdings.clear();
        let mut holdings_value = 0.0;
        for (symbol, qty) in &self.positions {
            if let Some(px) = prices.get(symbol) {
                let value = qty * px;
                self.holdings.insert(symbol.clone(), value);
                holdings_value += value;
            }
        }
        self.total_value = self.cash + holdings_value;
    }

    /// 把当前净值记入历史。
    ///
    /// - 入参：`date` 该条记录的时间戳。
    /// - 加工：追加 `(时间戳, 当前净值)`。
    /// - 出参：无。上游把重估与记录合在一个方法里，导致 `full_rebalance` 模式下同一天
    ///   写两条；此处拆开，由调用方保证一天只记一次。
    pub fn record(&mut self, date: &str) {
        self.history.push((date.to_string(), self.total_value));
    }

    /// 成交一笔，扣减现金并累计手续费。
    ///
    /// - 入参：`symbol` 标的；`quantity` 成交数量（正买负卖）；`price` 成交价；
    ///   `cost_rates` `(买入费率, 卖出费率)`；`date` 成交日期。
    /// - 加工：成交金额 = 数量 × 价格 → 手续费 = `|成交金额| × 对应方向费率`
    ///   → 现金扣掉 `成交金额 + 手续费` → 更新持仓数量，绝对值小于 `1e-10` 时移除该标的
    ///   → 追加一条流水。
    /// - 出参：无。
    pub fn execute_trade(
        &mut self,
        symbol: &str,
        quantity: f64,
        price: f64,
        cost_rates: (f64, f64),
        date: &str,
    ) {
        let trade_value = quantity * price;
        let rate = if quantity > 0.0 {
            cost_rates.0
        } else {
            cost_rates.1
        };
        let cost = trade_value.abs() * rate;

        self.cash -= trade_value + cost;
        let new_qty = self.positions.get(symbol).copied().unwrap_or(0.0) + quantity;
        if new_qty.abs() < 1e-10 {
            self.positions.remove(symbol);
        } else {
            self.positions.insert(symbol.to_string(), new_qty);
        }
        self.trade_log.push(Trade {
            date: date.to_string(),
            symbol: symbol.to_string(),
            trade_value,
            cost,
        });
    }

    /// 初始资金。
    pub fn initial_capital(&self) -> f64 {
        self.initial_capital
    }

    /// 当前现金。
    pub fn cash(&self) -> f64 {
        self.cash
    }

    /// 当前净值（现金 + 已报价持仓市值）。
    pub fn total_value(&self) -> f64 {
        self.total_value
    }

    /// 当前持仓数量，`标的 → 数量`。
    pub fn positions(&self) -> &BTreeMap<String, f64> {
        &self.positions
    }

    /// 最近一次重估得到的持仓市值，`标的 → 市值`。
    pub fn holdings(&self) -> &BTreeMap<String, f64> {
        &self.holdings
    }

    /// 净值历史，`(时间戳, 净值)`，按时间升序且一天一条。
    pub fn history(&self) -> &[(String, f64)] {
        &self.history
    }

    /// 成交流水，按成交顺序。
    pub fn trade_log(&self) -> &[Trade] {
        &self.trade_log
    }
}
