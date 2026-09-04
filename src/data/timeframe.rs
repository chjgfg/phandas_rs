//! K 线周期：Binance 的 `interval` 取值与它对应的时间网格。
//!
//! 上游用 `TIMEFRAME_MAP` 把周期串映射成 pandas 的频率别名，那张表有两个问题，本模块都不复现：
//!
//! 1. **`'1w' → 'W'`**：pandas 的 `W` 是 `W-SUN`（周日为界），而 Binance 的周线开在**周一**
//!    `00:00:00Z`。重建索引时一根都对不上，`ffill` 连种子都没有，整个面板出来全是 NaN。
//! 2. **`TIMEFRAME_MAP.get(tf, 'D')`**：`2h` / `6h` / `8h` / `12h` / `3d` / `3m` / `1s` 都是合法的
//!    Binance interval，会被原样发到 API，但频率别名回落成日线后重建索引只留午夜那一根，
//!    静默丢掉大半数据。
//!
//! 这里改成枚举：16 个合法取值一个不少，未知取值 [`Timeframe::parse`] 直接 `Err`；网格步长按
//! Binance 的**实际**开盘间隔算，并且网格起点取自真实数据（见 `process` 模块的对齐逻辑），
//! 所以周线锚在周一、月线锚在 1 日，不会错位。

use crate::net::time::add_months;

/// K 线周期，取值与 Binance `interval` 参数一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Timeframe {
    /// `1s`
    S1,
    /// `1m`
    M1,
    /// `3m`
    M3,
    /// `5m`
    M5,
    /// `15m`
    M15,
    /// `30m`
    M30,
    /// `1h`
    H1,
    /// `2h`
    H2,
    /// `4h`
    H4,
    /// `6h`
    H6,
    /// `8h`
    H8,
    /// `12h`
    H12,
    /// `1d`，上游默认
    #[default]
    D1,
    /// `3d`
    D3,
    /// `1w`，开在周一 `00:00:00Z`
    W1,
    /// `1M`，开在每月 1 日 `00:00:00Z`
    Mon1,
}

/// 全部合法取值，按周期升序。`parse` 与 `as_str` 共用这张表，避免两处漏改。
const ALL: [(Timeframe, &str); 16] = [
    (Timeframe::S1, "1s"),
    (Timeframe::M1, "1m"),
    (Timeframe::M3, "3m"),
    (Timeframe::M5, "5m"),
    (Timeframe::M15, "15m"),
    (Timeframe::M30, "30m"),
    (Timeframe::H1, "1h"),
    (Timeframe::H2, "2h"),
    (Timeframe::H4, "4h"),
    (Timeframe::H6, "6h"),
    (Timeframe::H8, "8h"),
    (Timeframe::H12, "12h"),
    (Timeframe::D1, "1d"),
    (Timeframe::D3, "3d"),
    (Timeframe::W1, "1w"),
    (Timeframe::Mon1, "1M"),
];

impl Timeframe {
    /// 由字符串解析，取值与 Binance `interval` 一致。
    ///
    /// - 入参：`s` 周期串，如 `"1d"` / `"4h"` / `"1M"`。**大小写敏感**：`1m` 是一分钟、
    ///   `1M` 是一个月，与 Binance 一致。
    /// - 加工：在内部的合法取值表里精确匹配。
    /// - 出参：`Ok(Timeframe)`；未知取值返回带完整合法值清单的 `Err`——上游对未知值静默
    ///   回落成日线并丢数据，这里拒掉。
    pub fn parse(s: &str) -> Result<Timeframe, String> {
        ALL.iter()
            .find(|(_, name)| *name == s)
            .map(|(tf, _)| *tf)
            .ok_or_else(|| {
                let names: Vec<&str> = ALL.iter().map(|(_, n)| *n).collect();
                format!("Invalid timeframe: {s}. Must be one of {names:?}")
            })
    }

    /// Binance `interval` 参数的字符串形式。
    ///
    /// - 入参：无（取自身枚举值）。
    /// - 加工：在内部的合法取值表里反查。
    /// - 出参：可直接拼进 query string 的周期串。
    pub fn as_str(self) -> &'static str {
        ALL.iter()
            .find(|(tf, _)| *tf == self)
            .map(|(_, name)| *name)
            .expect("ALL 覆盖全部枚举值")
    }

    /// 是否日内周期（决定时间戳字符串带不带时分秒）。
    ///
    /// - 入参：无。
    /// - 加工：`1d` 及更粗的算日级。
    /// - 出参：日内为 `true`，此时时间索引用 `YYYY-MM-DD HH:MM:SS`，否则用 `YYYY-MM-DD`
    ///   ——与上游 pandas 写出时间戳的形式一致（全是午夜时省掉时间部分）。
    pub fn is_intraday(self) -> bool {
        self < Timeframe::D1
    }

    /// 固定步长的毫秒数；`1M` 没有固定步长，返回 `None`。
    ///
    /// - 入参：无。
    /// - 加工：查表。
    /// - 出参：`Some(毫秒)`；月线返回 `None`，请改用 [`Timeframe::advance`]。
    pub fn step_millis(self) -> Option<i64> {
        let ms = match self {
            Timeframe::S1 => 1_000,
            Timeframe::M1 => 60_000,
            Timeframe::M3 => 3 * 60_000,
            Timeframe::M5 => 5 * 60_000,
            Timeframe::M15 => 15 * 60_000,
            Timeframe::M30 => 30 * 60_000,
            Timeframe::H1 => 3_600_000,
            Timeframe::H2 => 2 * 3_600_000,
            Timeframe::H4 => 4 * 3_600_000,
            Timeframe::H6 => 6 * 3_600_000,
            Timeframe::H8 => 8 * 3_600_000,
            Timeframe::H12 => 12 * 3_600_000,
            Timeframe::D1 => 86_400_000,
            Timeframe::D3 => 3 * 86_400_000,
            Timeframe::W1 => 7 * 86_400_000,
            Timeframe::Mon1 => return None,
        };
        Some(ms)
    }

    /// 网格上的下一格开盘时刻。
    ///
    /// - 入参：`ms` 当前格的开盘 epoch 毫秒。
    /// - 加工：定长周期直接加步长；`1M` 走自然月进位。
    /// - 出参：下一格的开盘毫秒。重建索引时从真实数据的首格反复调用它生成网格，
    ///   所以网格自动锚在正确的相位上（周线锚周一、月线锚 1 日），不像上游写死频率别名。
    pub fn advance(self, ms: i64) -> i64 {
        match self.step_millis() {
            Some(step) => ms + step,
            None => add_months(ms, 1),
        }
    }
}
