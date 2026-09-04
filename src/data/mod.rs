//! 行情抓取模块：Python 版 phandas 中 `data.py` 的 Rust 实现。
//!
//! 只在开启 `data` feature 时编译。**不用 ccxt**，直接打 Binance 的公开 REST 端点。
//!
//! # 文件划分
//!
//! | 文件 | 内容 |
//! |---|---|
//! | `timeframe.rs` | [`Timeframe`]：16 个合法周期与它们的时间网格 |
//! | `symbol.rs` | 裸标的名 ↔ Binance 市场 id、历史改名表（MATIC→POL） |
//! | `klines.rs` | [`BinanceClient`]：`klines` / `exchangeInfo` 两个端点与分页 |
//!
//! # 与 Python 版的已知偏差
//!
//! 上游这个模块的 bug 比别处多，以下几条**刻意不复现**：
//!
//! - **分页死循环**。`end_date` 之后还有 ≥1000 根 K 线时，上游的游标不再推进且两个 break
//!   都不触发，同一请求无限重发（`1d` 约 2.7 年、`1h` 约 41 天就够）。详见
//!   [`BinanceClient::klines`]。
//! - **`'1w'` 整块出 NaN**。上游把 `1w` 映射到 pandas 的 `W`（即 `W-SUN`），而 Binance 周线开在
//!   周一 `00:00:00Z`，重建索引一根都对不上。本仓库的网格起点取自真实数据，相位自然正确。
//! - **未列入映射表的周期静默丢数据**。`2h` / `6h` / `8h` / `12h` / `3d` / `3m` / `1s` 都是合法
//!   Binance interval，上游回落成日线频率后重建索引只留午夜那根。[`Timeframe::parse`] 覆盖全部
//!   16 个取值，未知取值直接 `Err`。
//! - **`load_markets()` 在 per-symbol 循环里**。上游靠 ccxt 的内部缓存兜着，这里显式只取一次。
//! - **`output_path` 参数**不移植：`Panel::to_csv(path)` 已经有了，调用方自己写一行即可
//!   （上游那个 `os.path.dirname('panel.csv')` 返回空串导致 `makedirs` 抛
//!   `FileNotFoundError` 的 bug 也就跟着消失）。
//!
//! # 如实保留的上游行为
//!
//! 这几条改了会动结果，照抄：
//!
//! - **`start_date = None` 只给最近 1000 根**，不是全量历史。
//! - **`end_date` 是当日 `00:00:00Z`，不是当日收盘**。日线正好含当天那根；日内则只留
//!   `end_date` 当天 00:00 那一根。
//! - **计价币种写死 USDT**，无法请求 USDC / BTC 计价的市场。
//! - **不按 `status == "TRADING"` 过滤市场**，与 ccxt 的 `exchange.symbols` 一致。
//! - **单个标的抓取失败就整个跳过**（上游发 warning 后 `return None`）。

pub mod klines;
mod process;
mod sources;
pub mod symbol;
pub mod timeframe;

use std::collections::BTreeSet;

use crate::factor::Panel;
use crate::net::time::floor_to_day;
use crate::net::{date_to_millis, HttpTransport};

use self::process::{process, Frame};

pub use self::klines::{BinanceClient, Kline};
pub use self::symbol::{market_id, SymbolRename, SYMBOL_RENAMES};
pub use self::timeframe::Timeframe;

/// 数据源，对应上游 `fetch_data(sources=[...])` 里那四个字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Source {
    /// Binance OHLCV：`open` / `high` / `low` / `close` / `volume` 五列。上游默认，也是
    /// **唯一提供 `close` 的源**，而对齐必须以 `close` 为准，故它实际上是必选的。
    #[default]
    Binance,
    /// BTC 与 ETH 的收盘价广播到每个标的：`BTC_close` / `ETH_close` 两列。
    Benchmark,
    /// 日历特征，纯计算不联网：`year` / `month` / `day` / `dayofweek` /
    /// `dayofmonth_position` / `is_week_end` 六列。
    Calendar,
    /// 成交量加权均价：`vwap` 一列。
    Vwap,
}

impl Source {
    /// 由字符串解析。
    ///
    /// - 入参：`s` 源名。
    /// - 加工：与上游那四个字符串精确匹配。
    /// - 出参：`Ok(Source)`；未知取值返回带清单的 `Err`——上游发一条 warning 后**静默跳过**
    ///   该源，这里拒掉。
    pub fn parse(s: &str) -> Result<Source, String> {
        match s {
            "binance" => Ok(Source::Binance),
            "benchmark" => Ok(Source::Benchmark),
            "calendar" => Ok(Source::Calendar),
            "vwap" => Ok(Source::Vwap),
            other => Err(format!(
                "Unknown source: {other}. Available: ['binance', 'benchmark', 'calendar', 'vwap']"
            )),
        }
    }
}

/// 抓行情并拼成面板，对应上游 `fetch_data` / `fetch_panel_core`。
///
/// - 入参：`symbols` 裸标的名（如 `["ETH", "SOL"]`）；`tf` 周期；`start_date` / `end_date`
///   形如 `"2024-01-01"` 的日期，按当日 `00:00:00Z` 解释，`None` 表示不限；
///   `sources` 数据源，**空切片等同 `[Source::Binance]`**（对应上游 `sources=None`）。
/// - 加工：建一个默认的 [`BinanceClient`] 后转发给 [`fetch_with`]。
/// - 出参：`Ok(Panel)`，可直接 `panel.factor("close")` 接进因子链路。
///
/// 上游的 `output_path` 参数不移植——落盘写 `panel.to_csv(path)` 即可。
pub async fn fetch_data(
    symbols: &[&str],
    tf: Timeframe,
    start_date: Option<&str>,
    end_date: Option<&str>,
    sources: &[Source],
) -> Result<Panel, String> {
    let client = BinanceClient::new()?;
    fetch_with(&client, symbols, tf, start_date, end_date, sources).await
}

/// [`fetch_data`] 的可注入版本：客户端由调用方给。
///
/// - 入参：`client` 已配好的客户端（可换传输层、根地址、节流）；其余同 [`fetch_data`]。
/// - 加工：日期解析成毫秒 → 取一次已上市市场清单 → **按 `sources` 给定的顺序**逐个源抓取，
///   `binance` 抓完后记下它实际返回的最末时刻并截到当日零点，后续源的区间末点改用它
///   （对应上游 `binance_end_date = df['timestamp'].max().strftime('%Y-%m-%d')`）
///   → 各源的列外连接进一张中间表 → 交给 `process` 对齐成面板。
/// - 出参：`Ok(Panel)`；日期非法、一个源都没产出数据、或缺 `close` 列时返回 `Err`。
///
/// **顺序依赖是上游行为**：末点裁剪只在 `binance` 排在其他源**之前**时生效。
/// `sources = [Calendar, Binance]` 的日历部分用的是原始 `end_date`，与
/// `[Binance, Calendar]` 结果不同。如实保留。
pub async fn fetch_with<T: HttpTransport>(
    client: &BinanceClient<T>,
    symbols: &[&str],
    tf: Timeframe,
    start_date: Option<&str>,
    end_date: Option<&str>,
    sources: &[Source],
) -> Result<Panel, String> {
    if symbols.is_empty() {
        return Err("至少要给一个标的".to_string());
    }
    let parse = |d: Option<&str>, what: &str| -> Result<Option<i64>, String> {
        match d {
            None => Ok(None),
            Some(s) => date_to_millis(s)
                .map(Some)
                .ok_or_else(|| format!("{what} 不是合法日期：{s}（要 YYYY-MM-DD）")),
        }
    };
    let start = parse(start_date, "start_date")?;
    let end = parse(end_date, "end_date")?;

    let chosen: &[Source] = if sources.is_empty() {
        &[Source::Binance]
    } else {
        sources
    };

    // 候选市场 id：请求的标的 + 它们的历史旧符号 + benchmark 要的 BTC / ETH。
    // 只问这几个而不是拉全量市场表——后者约 2 MB，走代理时很容易读一半就断。
    let mut candidates: BTreeSet<String> = symbols.iter().map(|s| market_id(s)).collect();
    for s in symbols {
        if let Some(r) = symbol::rename_for(s) {
            candidates.insert(market_id(r.old_symbol));
        }
    }
    if chosen.contains(&Source::Benchmark) {
        candidates.insert(market_id("BTC"));
        candidates.insert(market_id("ETH"));
    }
    let ids: Vec<String> = candidates.into_iter().collect();
    let listed: BTreeSet<String> = client.listed_symbols(&ids).await?;

    let mut frame = Frame::new();
    let mut binance_end: Option<i64> = None;
    for src in chosen {
        let part = match src {
            Source::Binance => {
                let f = sources::binance(client, symbols, tf, start, end, &listed).await?;
                binance_end = f
                    .values()
                    .flat_map(|c| c.keys().map(|(ms, _)| *ms))
                    .max()
                    .map(floor_to_day);
                f
            }
            Source::Benchmark => {
                sources::benchmark(client, symbols, tf, start, binance_end.or(end), &listed).await?
            }
            Source::Calendar => sources::calendar(symbols, tf, start, binance_end.or(end))?,
            Source::Vwap => {
                sources::vwap(client, symbols, tf, start, binance_end.or(end), &listed).await?
            }
        };
        // 外连接：同一格已有值时保留先到的那份（上游 drop 重复列名时也是 keep='first'）
        for (col, cells) in part {
            let target = frame.entry(col).or_default();
            for (key, v) in cells {
                target.entry(key).or_insert(v);
            }
        }
    }
    if frame.is_empty() {
        return Err("No data fetched from any source".to_string());
    }
    process(&frame, tf, symbols)
}
