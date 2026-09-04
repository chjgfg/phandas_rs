//! 合约规格与张数换算。
//!
//! 上游 `convert_coin_contract` 把「名义额 ↔ 张数」这步甩给 OKX 的
//! `GET /api/v5/public/convert-contract-coin`，而且**没传 `opType`**——等于把「向上还是向下
//! 取整」交给服务端默认值。按 OKX 文档，该参数默认 `close`，也就是**向上**取整：一笔 $1.2 的
//! 调仓在最小一张值 $10 的合约上会被放大成 $10 的单，且上游那句 `contract_size <= 0` 的保护
//! 因此永远不会触发。
//!
//! 这里改成本地算，三件事随之确定下来：
//!
//! 1. **取整方向写死向下**（[`InstrumentSpec::contracts_for_notional`]）。宁可少下一点也不
//!    超出目标——再平衡本来就是逐期收敛的，欠一点下期补，超出去却可能直接翻向。
//! 2. **本地按 `minSz` 拦截**。上游从不校验，小于最小下单量的单子要等交易所拒了才知道。
//! 3. **少一次网络往返**，也不再依赖一个语义没写清的服务端默认值。
//!
//! 线性永续（`XXX-USDT-SWAP`）的换算关系：
//!
//! ```text
//! 一张 = ctVal × ctMult   （基础币计，例如 0.01 BTC）
//! 张数 = 名义额 / (价格 × 一张)
//! ```

use serde_json::Value;

use super::json::{f_of, s_of};

/// 一个合约的规格，取自 `GET /api/v5/account/instruments` 的一项。
///
/// 字段名沿用 OKX 的语义，只把驼峰改成蛇形。
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentSpec {
    /// 合约 id，如 `"BTC-USDT-SWAP"`。
    pub inst_id: String,
    /// 状态。可交易时为 `"live"`；上游读了但从不校验，本仓库在下单前会看它。
    pub state: String,
    /// 合约面值（基础币计）。
    pub ct_val: f64,
    /// 合约乘数，线性永续通常是 `1`。
    pub ct_mult: f64,
    /// 下单量步长（张）。
    pub lot_sz: f64,
    /// 最小下单量（张）。
    pub min_sz: f64,
    /// 价格步长。市价单用不上，留着备用。
    pub tick_sz: f64,
    /// 单笔市价单的最大张数；`0` 表示响应里没给。
    pub max_mkt_sz: f64,
    /// `lot_sz` 的小数位数，用来格式化发出去的 `sz`——
    /// 直接 `{}` 打 `f64` 可能出 `0.30000000000000004`，OKX 会拒。
    pub size_decimals: usize,
}

/// 数一个十进制字符串的小数位数。
///
/// - 入参：`s` 形如 `"0.001"` / `"1"` / `"1E-4"` 的数值串。
/// - 加工：小数点后的字符数；带指数的形式退化成按值反推（`1E-4` → 4 位）。
/// - 出参：小数位数，最多 12（超过没有实际意义，也避免格式化出一长串 0）。
fn decimals_of(s: &str) -> usize {
    if let Some((_, frac)) = s.split_once('.') {
        // 可能是 "0.0001" 也可能是 "1.0E-4"，后者交给下面的兜底
        if !frac.contains(['e', 'E']) {
            return frac.trim_end_matches('0').len().min(12);
        }
    }
    match s.parse::<f64>() {
        Ok(v) if v > 0.0 && v < 1.0 => (-v.log10().floor()) as usize,
        _ => 0,
    }
}

impl InstrumentSpec {
    /// 从一段 JSON 文本解析，内容是 `instruments` 响应里的**一项**。
    ///
    /// - 入参：`s` 形如 `{"instId":"BTC-USDT-SWAP","ctVal":"0.01",...}` 的 JSON 对象文本。
    /// - 加工：解析成 [`Value`] 后转发给 [`InstrumentSpec::from_json`]。
    /// - 出参：`Ok(InstrumentSpec)`；JSON 不合法或必需字段缺失时返回 `Err`。
    ///
    /// 有了这条，调用方与测试都不必自己依赖 `serde_json`。
    pub fn from_json_str(s: &str) -> Result<InstrumentSpec, String> {
        let v: Value =
            serde_json::from_str(s).map_err(|e| format!("instrument JSON 不合法：{e}"))?;
        InstrumentSpec::from_json(&v)
    }

    /// 从 `instruments` 响应的一项解析。
    ///
    /// - 入参：`v` 一项 instrument 的 JSON 对象。
    /// - 加工：逐字段按 OKX 的「数值也是字符串」约定解析；`lotSz` 额外记下小数位数。
    /// - 出参：`Ok(InstrumentSpec)`；缺 `instId`、或 `ctVal` / `lotSz` 非正时返回 `Err`
    ///   （这三项缺了就没法算张数，不能静默当 0 用）。
    pub fn from_json(v: &Value) -> Result<InstrumentSpec, String> {
        let inst_id = match s_of(v, "instId") {
            "" => return Err("instruments 响应里缺 instId".to_string()),
            s => s.to_string(),
        };
        let lot_raw = s_of(v, "lotSz");
        let spec = InstrumentSpec {
            state: s_of(v, "state").to_string(),
            ct_val: f_of(v, "ctVal"),
            // 响应里没给乘数时按 1 处理：线性永续绝大多数就是 1
            ct_mult: {
                let m = f_of(v, "ctMult");
                if m > 0.0 {
                    m
                } else {
                    1.0
                }
            },
            lot_sz: f_of(v, "lotSz"),
            min_sz: f_of(v, "minSz"),
            tick_sz: f_of(v, "tickSz"),
            max_mkt_sz: f_of(v, "maxMktSz"),
            size_decimals: decimals_of(lot_raw),
            inst_id,
        };
        if spec.ct_val <= 0.0 {
            return Err(format!("{} 的 ctVal 非正，无法换算张数", spec.inst_id));
        }
        if spec.lot_sz <= 0.0 {
            return Err(format!("{} 的 lotSz 非正，无法对齐下单量", spec.inst_id));
        }
        Ok(spec)
    }

    /// 是否可交易。
    ///
    /// - 入参：无。
    /// - 加工：判断 `state == "live"`。
    /// - 出参：可交易为 `true`。上游读了 `state` 却从不校验，本仓库在建计划时会看。
    pub fn is_live(&self) -> bool {
        self.state == "live"
    }

    /// 一张合约折多少基础币。
    ///
    /// - 入参：无。
    /// - 加工：`ct_val × ct_mult`。
    /// - 出参：基础币数量，例如 `BTC-USDT-SWAP` 是 `0.01`。
    pub fn coin_per_contract(&self) -> f64 {
        self.ct_val * self.ct_mult
    }

    /// 一张合约在给定价格下的名义额。
    ///
    /// - 入参：`price` 标记价或最新价。
    /// - 加工：`price × 一张的基础币数`。
    /// - 出参：单张名义额（USD）。它就是这个合约上「能下的最小一笔」的量级。
    pub fn notional_per_contract(&self, price: f64) -> f64 {
        price * self.coin_per_contract()
    }

    /// 名义额 → 可下的张数，按 `lot_sz` **向下**对齐。
    ///
    /// - 入参：`notional_usd` 目标名义额的绝对值（USD）；`price` 成交价参考。
    /// - 加工：`张数 = 名义额 / 单张名义额` → 按 `lot_sz` 向下取整 → 与 `min_sz` 比。
    ///   取整用 `(x / lot + 1e-9).floor()` 而不是裸 `floor`：`0.3 / 0.1` 在二进制里是
    ///   `2.9999…`，裸 `floor` 会少掉一整个 lot。
    /// - 出参：`Ok(张数)`；价格非正、名义额非正，或对齐后不足 `min_sz` 时返回说明原因的 `Err`
    ///   ——**本地就拦下**，不像上游要等交易所拒单才知道。
    pub fn contracts_for_notional(&self, notional_usd: f64, price: f64) -> Result<f64, String> {
        if price.is_nan() || price <= 0.0 {
            return Err(format!("{} 的参考价非正：{price}", self.inst_id));
        }
        if notional_usd.is_nan() || notional_usd <= 0.0 {
            return Err(format!("{} 的目标名义额非正：{notional_usd}", self.inst_id));
        }
        let per = self.notional_per_contract(price);
        let raw = notional_usd / per;
        let contracts = (raw / self.lot_sz + 1e-9).floor() * self.lot_sz;
        if contracts < self.min_sz {
            return Err(format!(
                "{}：名义额 {notional_usd:.4} 按向下取整只有 {contracts} 张，低于最小下单量 {} 张（单张约 {per:.4}）",
                self.inst_id, self.min_sz
            ));
        }
        Ok(contracts)
    }

    /// 张数 → 名义额。
    ///
    /// - 入参：`contracts` 张数；`price` 成交价参考。
    /// - 加工：`张数 × 单张名义额`。
    /// - 出参：名义额（USD）。用来在计划里显示「实际会成交多少」，与目标值对照看取整损失。
    pub fn notional_of(&self, contracts: f64, price: f64) -> f64 {
        contracts * self.notional_per_contract(price)
    }

    /// 按 `lot_sz` 的小数位数把张数格式化成可发出的 `sz`。
    ///
    /// - 入参：`contracts` 张数。
    /// - 加工：按 [`InstrumentSpec::size_decimals`] 定点格式化。
    /// - 出参：字符串。**必须走这条而不是直接 `{}` 打 `f64`**——后者可能出
    ///   `0.30000000000000004` 这种，OKX 直接拒。上游用 Python 的 `str()`，同样有
    ///   `str(1e-05) == '1e-05'` 这类风险。
    pub fn format_size(&self, contracts: f64) -> String {
        format!("{:.*}", self.size_decimals, contracts)
    }

    /// 单笔市价单是否超过 `max_mkt_sz`。
    ///
    /// - 入参：`contracts` 张数。
    /// - 加工：`max_mkt_sz` 为 `0`（响应没给）时一律判否。
    /// - 出参：超限为 `true`。上游完全不看这个字段，超了由交易所拒单。
    pub fn exceeds_market_cap(&self, contracts: f64) -> bool {
        self.max_mkt_sz > 0.0 && contracts > self.max_mkt_sz
    }
}
