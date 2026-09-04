//! OKX 响应里那几个数据结构，以及下单请求的入参类型。
//!
//! 全部字段名沿用 OKX 的语义，只把驼峰改成蛇形。数值一律在解析时就从字符串转成 `f64`
//! （OKX 的数值字段一律是字符串、空串表示不适用），所以下游拿到的都是能直接算的数。

use serde_json::Value;

use super::contract::InstrumentSpec;
use super::json::{f_of, opt_f, s_of};

/// 持仓模式，对应账户配置里的 `posMode`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionMode {
    /// 单向持仓（`net_mode`）。下单时 `posSide` 填 `net`。**再平衡要求这个模式。**
    Net,
    /// 双向持仓（`long_short_mode`）。下单时 `posSide` 填 `long` / `short`。
    LongShort,
}

impl PositionMode {
    /// 由 OKX 的字符串解析。
    ///
    /// - 入参：`s` `posMode` 原值。
    /// - 加工：与两个已知取值精确匹配。
    /// - 出参：`Some(PositionMode)`；未知取值给 `None`（宁可报错也不猜）。
    pub fn parse(s: &str) -> Option<PositionMode> {
        match s {
            "net_mode" => Some(PositionMode::Net),
            "long_short_mode" => Some(PositionMode::LongShort),
            _ => None,
        }
    }

    /// OKX 侧的字符串形式。
    pub fn as_str(self) -> &'static str {
        match self {
            PositionMode::Net => "net_mode",
            PositionMode::LongShort => "long_short_mode",
        }
    }
}

/// 保证金模式，下单与设杠杆都要带。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarginMode {
    /// 全仓。上游永续路径固定用这个。
    #[default]
    Cross,
    /// 逐仓。
    Isolated,
}

impl MarginMode {
    /// OKX 侧的 `tdMode` / `mgnMode` 取值。
    pub fn as_str(self) -> &'static str {
        match self {
            MarginMode::Cross => "cross",
            MarginMode::Isolated => "isolated",
        }
    }
}

/// 下单方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    /// 买入 / 开多 / 平空。
    Buy,
    /// 卖出 / 开空 / 平多。
    Sell,
}

impl OrderSide {
    /// OKX 侧的 `side` 取值。
    pub fn as_str(self) -> &'static str {
        match self {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        }
    }

    /// 按名义额的正负定方向。
    ///
    /// - 入参：`delta_usd` 目标减当前的名义额差额。
    /// - 加工：正数买、负数卖。
    /// - 出参：方向。`0` 给 `Buy`，但调用方本来就该先用阈值把 `0` 附近滤掉。
    pub fn from_delta(delta_usd: f64) -> OrderSide {
        if delta_usd > 0.0 {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        }
    }
}

/// 账户配置，取自 `GET /api/v5/account/config` 的 `data[0]`。
#[derive(Debug, Clone, PartialEq)]
pub struct AccountConfig {
    /// 账户模式：`1` 现货、`2` 现货与合约、`3` 跨币种保证金、`4` 组合保证金。
    pub acct_lv: String,
    /// 持仓模式；OKX 给了未知取值时为 `None`。
    pub pos_mode: Option<PositionMode>,
    /// 持仓模式原值，报错信息里要照原样显示。
    pub pos_mode_raw: String,
    /// 账户 uid。
    pub uid: String,
}

impl AccountConfig {
    /// 从 `data[0]` 解析。
    ///
    /// - 入参：`v` 一项账户配置的 JSON。
    /// - 加工：取三个字段，`posMode` 顺手解析成枚举。
    /// - 出参：[`AccountConfig`]。字段缺失时退化成空串 / `None`，由
    ///   [`AccountConfig::check`] 统一报问题。
    pub(crate) fn from_json(v: &Value) -> AccountConfig {
        let raw = s_of(v, "posMode").to_string();
        AccountConfig {
            acct_lv: s_of(v, "acctLv").to_string(),
            pos_mode: PositionMode::parse(&raw),
            pos_mode_raw: raw,
            uid: s_of(v, "uid").to_string(),
        }
    }

    /// 账户模式是否支持永续合约。
    ///
    /// - 入参：无。
    /// - 加工：`acctLv` 是 `2`（现货与合约）或 `3`（跨币种保证金）。
    /// - 出参：支持为 `true`。与上游 `validate_account_config` 的判据一致。
    pub fn supports_swap(&self) -> bool {
        self.acct_lv == "2" || self.acct_lv == "3"
    }

    /// 是否单向持仓。再平衡的目标权重是净敞口，只在这个模式下成立。
    pub fn is_net_mode(&self) -> bool {
        self.pos_mode == Some(PositionMode::Net)
    }

    /// 逐条检查再平衡的前置条件。
    ///
    /// - 入参：无。
    /// - 加工：校验账户模式与持仓模式。
    /// - 出参：全部通过给空向量；否则给人话的问题清单。
    ///   与上游同样的两条判据，只是上游把它塞进一个 dict 返回，这里给 `Vec<String>`。
    pub fn check(&self) -> Vec<String> {
        let mut bad = Vec::new();
        if !self.supports_swap() {
            bad.push(format!(
                "账户模式须为 2（现货与合约）或 3（跨币种保证金），当前 acctLv = {:?}",
                self.acct_lv
            ));
        }
        if !self.is_net_mode() {
            bad.push(format!(
                "持仓模式须为 net_mode（单向持仓），当前 posMode = {:?}",
                self.pos_mode_raw
            ));
        }
        bad
    }
}

/// 一笔持仓，取自 `GET /api/v5/account/positions` 的一项。
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    /// 合约 id，如 `"BTC-USDT-SWAP"`。
    pub inst_id: String,
    /// `net` / `long` / `short`。
    pub pos_side: String,
    /// 张数，**带符号**（空头为负）。
    pub contracts: f64,
    /// 名义额（USD），**带符号**——OKX 的 `notionalUsd` 只给绝对值，符号按张数补。
    pub notional_usd: f64,
    /// 标记价。
    pub mark_px: f64,
    /// 开仓均价；刚建仓时 OKX 可能给空串，此时为 `0`。
    pub avg_px: f64,
    /// 未实现盈亏。
    pub upl: f64,
    /// 杠杆倍数；缺失时为 `1`。
    pub leverage: f64,
}

/// 名义额低于这个数（USD）就当没有持仓，对应上游 `constants.MIN_NOTIONAL_USD`。
pub const MIN_NOTIONAL_USD: f64 = 0.01;

impl Position {
    /// 从 `data` 里的一项解析；不值一提的空仓返回 `None`。
    ///
    /// - 入参：`v` 一项持仓的 JSON。
    /// - 加工：取张数与名义额，**按张数的正负给名义额补符号**（OKX 的 `notionalUsd` 是绝对值）；
    ///   张数为 0、名义额为 0、标记价为 0 或名义额绝对值小于 [`MIN_NOTIONAL_USD`] 的一律丢掉。
    /// - 出参：`Some(Position)`；被上述闸门滤掉时 `None`。判据与上游 `get_positions` 一致。
    pub(crate) fn from_json(v: &Value) -> Option<Position> {
        let contracts = f_of(v, "pos");
        let notional_abs = f_of(v, "notionalUsd").abs();
        let mark_px = f_of(v, "markPx");
        if contracts == 0.0
            || notional_abs == 0.0
            || mark_px == 0.0
            || notional_abs < MIN_NOTIONAL_USD
        {
            return None;
        }
        Some(Position {
            inst_id: s_of(v, "instId").to_string(),
            pos_side: s_of(v, "posSide").to_string(),
            contracts,
            notional_usd: if contracts > 0.0 {
                notional_abs
            } else {
                -notional_abs
            },
            mark_px,
            avg_px: f_of(v, "avgPx"),
            upl: f_of(v, "upl"),
            leverage: opt_f(v, "lever").filter(|l| *l > 0.0).unwrap_or(1.0),
        })
    }

    /// 合约 id 的基础币部分，如 `"BTC-USDT-SWAP"` → `"BTC"`。
    ///
    /// - 入参：无。
    /// - 加工：取第一个 `-` 之前的片段。
    /// - 出参：基础币符号。
    ///
    /// 上游用它当持仓字典的键，于是 `BTC-USDT-SWAP` 与 `BTC-USD-SWAP`（或某个交割合约）
    /// 会**塌成同一个键、后者覆盖前者**，另一条腿的仓位对再平衡完全不可见。本仓库把
    /// 完整 `inst_id` 留着，只在按后缀过滤之后才用这个方法，见 `rebalance`。
    pub fn base_symbol(&self) -> &str {
        self.inst_id.split('-').next().unwrap_or(&self.inst_id)
    }
}

/// 行情快照，取自 `GET /api/v5/market/ticker`。
#[derive(Debug, Clone, PartialEq)]
pub struct Ticker {
    /// 合约 id。
    pub inst_id: String,
    /// 最新成交价。
    pub last: f64,
    /// 买一价；OKX 可能给空串，此时为 `0`。
    pub bid: f64,
    /// 卖一价；同上。
    pub ask: f64,
}

impl Ticker {
    /// 从 `data[0]` 解析。
    ///
    /// - 入参：`v` 一项行情的 JSON。
    /// - 加工：取四个字段，空串按 `0` 处理（上游用裸 `float()`，遇空串直接抛异常）。
    /// - 出参：[`Ticker`]。
    pub(crate) fn from_json(v: &Value) -> Ticker {
        Ticker {
            inst_id: s_of(v, "instId").to_string(),
            last: f_of(v, "last"),
            bid: f_of(v, "bidPx"),
            ask: f_of(v, "askPx"),
        }
    }
}

/// 账户权益，取自 `GET /api/v5/account/balance` 的 `data[0]`。
#[derive(Debug, Clone, PartialEq)]
pub struct Balance {
    /// 总权益（USD）。再平衡的 `budget` 一般取这个。
    pub total_equity: f64,
    /// 可用权益。
    pub available_equity: f64,
    /// 已用初始保证金。
    pub used_margin: f64,
    /// 维持保证金。
    pub maint_margin: f64,
    /// 未实现盈亏。
    pub upl: f64,
    /// USDT 的现金余额。
    pub usdt_cash: f64,
}

impl Balance {
    /// 从 `data[0]` 解析。
    ///
    /// - 入参：`v` 一项账户权益的 JSON。
    /// - 加工：取五个总额字段，再从 `details` 里挑出 `ccy == "USDT"` 的现金余额。
    /// - 出参：[`Balance`]。
    pub(crate) fn from_json(v: &Value) -> Balance {
        let usdt_cash = v
            .get("details")
            .and_then(Value::as_array)
            .and_then(|ds| ds.iter().find(|d| s_of(d, "ccy") == "USDT"))
            .map(|d| f_of(d, "cashBal"))
            .unwrap_or(0.0);
        Balance {
            total_equity: f_of(v, "totalEq"),
            available_equity: f_of(v, "availEq"),
            used_margin: f_of(v, "imr"),
            maint_margin: f_of(v, "mmr"),
            upl: f_of(v, "upl"),
            usdt_cash,
        }
    }
}

/// 一张要发出去的市价单。
///
/// **只有市价单**：上游 `rebalance_portfolio` 一律传 `price=None`，本仓库同。故这里不带价格
/// 字段，也就不存在「限价单挂着不成交、下一期又算一遍」那类状态。
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRequest {
    /// 合约 id。
    pub inst_id: String,
    /// 方向。
    pub side: OrderSide,
    /// 张数，**已按 `lotSz` 的小数位数格式化好的字符串**。
    ///
    /// 刻意存字符串而不是 `f64`：直接 `{}` 打浮点会出 `0.30000000000000004`，OKX 直接拒。
    /// 用 [`OrderRequest::market`] 构造就不会踩到。
    pub size: String,
    /// 是否只减仓。目标为 0 的平仓单必须带上，否则有冲成反向仓位的风险。
    pub reduce_only: bool,
}

impl OrderRequest {
    /// 按合约规格构造一张市价单。
    ///
    /// - 入参：`spec` 合约规格；`side` 方向；`contracts` 张数（须已按 `lotSz` 对齐）；
    ///   `reduce_only` 是否只减仓。
    /// - 加工：用 [`InstrumentSpec::format_size`] 把张数定点格式化。
    /// - 出参：[`OrderRequest`]。走这条就不可能把浮点噪声发上去。
    pub fn market(
        spec: &InstrumentSpec,
        side: OrderSide,
        contracts: f64,
        reduce_only: bool,
    ) -> OrderRequest {
        OrderRequest {
            inst_id: spec.inst_id.clone(),
            side,
            size: spec.format_size(contracts),
            reduce_only,
        }
    }
}

/// 一张单的下单结果，取自 `POST /api/v5/trade/order` 或 `batch-orders` 的 `data` 项。
#[derive(Debug, Clone, PartialEq)]
pub struct OrderAck {
    /// 客户端自定义单号，用来把结果对回请求。
    pub cl_ord_id: String,
    /// 交易所单号；失败时为空串。
    pub ord_id: String,
    /// **逐单**状态码，`"0"` 才是成功。
    ///
    /// 注意与顶层 `code` 的区别：批量下单部分成功时顶层给 `"2"`，逐单的成败要看这里。
    /// 上游只判顶层 `code == '0'`，于是**部分成功时把每单的 `sCode` / `sMsg` 全丢了**。
    pub code: String,
    /// 逐单错误信息；成功时通常为空串。
    pub msg: String,
}

impl OrderAck {
    /// 从 `data` 里的一项解析。
    ///
    /// - 入参：`v` 一项下单结果的 JSON。
    /// - 加工：取 `clOrdId` / `ordId` / `sCode` / `sMsg`。
    /// - 出参：[`OrderAck`]。
    pub(crate) fn from_json(v: &Value) -> OrderAck {
        OrderAck {
            cl_ord_id: s_of(v, "clOrdId").to_string(),
            ord_id: s_of(v, "ordId").to_string(),
            code: s_of(v, "sCode").to_string(),
            msg: s_of(v, "sMsg").to_string(),
        }
    }

    /// 这一单是否成功。
    pub fn is_success(&self) -> bool {
        self.code == "0"
    }
}
