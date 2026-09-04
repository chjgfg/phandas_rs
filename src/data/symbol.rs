//! 标的名映射与历史改名表。
//!
//! 上游把计价币种写死在 `f'{sym}/USDT'` 里，这里同样写死成 [`QUOTE`]——没有办法请求
//! USDC / BTC 计价的合约，与上游一致。传入的标的名是**裸名**（`"ETH"`、`"POL"`），
//! 输出与 `symbol` 列里存的也是裸名，只有打 API 时才拼成 Binance 的市场 id。
//!
//! 顺带记一条上游行为：若调用方传了 `"ETH/USDT"`，上游拼出 `"ETH/USDT/USDT"`、查不到市场，
//! 于是发一条 warning 把该标的整个跳过。本仓库 [`market_id`] 拼出 `"ETH/USDTUSDT"`，
//! 同样查不到、同样跳过，结果一致。

/// 计价币种。上游写死 USDT，这里集中成一个常量便于查。
pub const QUOTE: &str = "USDT";

/// 一次历史改名：同一条链在某个时点换了代币符号，行情要分两段取。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolRename {
    /// 改名后的符号，也是调用方传进来的那个。
    pub new_symbol: &'static str,
    /// 改名前的符号，用于取 `cutoff_date` 之前的历史。
    pub old_symbol: &'static str,
    /// 分界日（`YYYY-MM-DD`）。该日 `00:00:00Z` 起用新符号，之前用旧符号。
    pub cutoff_date: &'static str,
}

/// 内置改名表，对应上游 `constants.SYMBOL_RENAMES`。
///
/// 上游那张 dict 里还有个 `new_symbol` 字段，但代码从不读它——真正当新符号用的是 dict 的键。
/// 这里把键并进结构体的 `new_symbol`，语义等价且少一处冗余。
pub const SYMBOL_RENAMES: [SymbolRename; 1] = [SymbolRename {
    new_symbol: "POL",
    old_symbol: "MATIC",
    cutoff_date: "2024-09-01",
}];

/// 裸标的名 → Binance 市场 id。
///
/// - 入参：`symbol` 裸标的名，如 `"ETH"`。
/// - 加工：直接与 [`QUOTE`] 拼接。
/// - 出参：形如 `"ETHUSDT"` 的市场 id，可直接当 `symbol` 查询参数用。
pub fn market_id(symbol: &str) -> String {
    format!("{symbol}{QUOTE}")
}

/// 查这个标的有没有历史改名。
///
/// - 入参：`symbol` 裸标的名。
/// - 加工：在 [`SYMBOL_RENAMES`] 里按 `new_symbol` 精确匹配。
/// - 出参：`Some(&SymbolRename)` 表示要分两段取；`None` 表示照常单段取。
pub fn rename_for(symbol: &str) -> Option<&'static SymbolRename> {
    SYMBOL_RENAMES.iter().find(|r| r.new_symbol == symbol)
}
