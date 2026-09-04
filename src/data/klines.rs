//! Binance 现货 K 线：请求构造、分页与响应解析。
//!
//! 只用两个**公开**端点，都不需要 API key：
//!
//! | 端点 | 用途 |
//! |---|---|
//! | `GET /api/v3/klines` | 取 OHLCV，一次最多 1000 根 |
//! | `GET /api/v3/exchangeInfo` | 取全部上市市场，用来判断 `{SYM}USDT` 在不在 |
//!
//! 对应上游经由 ccxt 的 `exchange.fetch_ohlcv(...)` 与 `exchange.load_markets()`。上游把
//! `load_markets()` 放在 per-symbol 循环里，靠 ccxt 的内部缓存兜着；这里显式只取一次。

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;

use crate::net::{HttpRequest, HttpTransport, ReqwestTransport};

use super::timeframe::Timeframe;

/// 单次 `klines` 请求的最大根数，对应上游 `FETCH_BATCH_SIZE`，也正好是该端点的上限。
const BATCH_LIMIT: usize = 1000;

/// 默认 API 根地址。
const DEFAULT_BASE_URL: &str = "https://api.binance.com";

/// 分页之间的默认节流，对应上游 `time.sleep(exchange.rateLimit / 1000)`（ccxt 4.x 给 50 毫秒）。
const DEFAULT_PACE: Duration = Duration::from_millis(50);

/// 默认失败重试次数。走代理时 2 MB 级响应读一半断掉不算罕见，重试一次通常就好。
const DEFAULT_RETRIES: u32 = 2;

/// 一根 K 线。只保留上游用到的前 6 项，后 6 项（收盘时刻、成交额、笔数……）一概丢弃。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kline {
    /// **开盘**时刻的 epoch 毫秒（UTC）。上游取的也是开盘时刻，不是收盘时刻。
    pub open_time: i64,
    /// 开盘价。
    pub open: f64,
    /// 最高价。
    pub high: f64,
    /// 最低价。
    pub low: f64,
    /// 收盘价。
    pub close: f64,
    /// 成交量（基础币计）。
    pub volume: f64,
}

/// Binance 公开端点客户端。
///
/// 对传输层泛型，默认用 [`ReqwestTransport`]；测试里换成返回固定 JSON 的假实现即可离线跑通
/// 整条「构造请求 → 分页 → 解析」链路。
#[derive(Debug, Clone)]
pub struct BinanceClient<T: HttpTransport = ReqwestTransport> {
    transport: T,
    base_url: String,
    pace: Duration,
    retries: u32,
}

impl BinanceClient<ReqwestTransport> {
    /// 用内置的 reqwest 传输层构造。
    ///
    /// - 入参：无。
    /// - 加工：建一个带 30 秒超时的 [`ReqwestTransport`]。
    /// - 出参：`Ok(BinanceClient)`；TLS 后端初始化失败时返回 `Err`。
    pub fn new() -> Result<BinanceClient<ReqwestTransport>, String> {
        Ok(BinanceClient::with_transport(ReqwestTransport::new()?))
    }
}

impl<T: HttpTransport> BinanceClient<T> {
    /// 用调用方自备的传输层构造。
    ///
    /// - 入参：`transport` 任意 [`HttpTransport`] 实现。
    /// - 加工：套上默认根地址与默认节流。
    /// - 出参：[`BinanceClient`]。
    pub fn with_transport(transport: T) -> BinanceClient<T> {
        BinanceClient {
            transport,
            base_url: DEFAULT_BASE_URL.to_string(),
            pace: DEFAULT_PACE,
            retries: DEFAULT_RETRIES,
        }
    }

    /// 换 API 根地址并返回自身，便于链式书写。
    ///
    /// - 入参：`url` 根地址，不含尾斜杠，如 `"https://data-api.binance.vision"`。
    /// - 加工：替换字段。
    /// - 出参：改动后的自身。用于切到镜像站点，或在测试里指向假地址。
    pub fn base_url(mut self, url: impl Into<String>) -> BinanceClient<T> {
        self.base_url = url.into();
        self
    }

    /// 换分页节流间隔并返回自身。
    ///
    /// - 入参：`pace` 每批之间的等待时长；传 [`Duration::ZERO`] 即不等待（测试用）。
    /// - 加工：替换字段。
    /// - 出参：改动后的自身。
    pub fn pace(mut self, pace: Duration) -> BinanceClient<T> {
        self.pace = pace;
        self
    }

    /// 换失败重试次数并返回自身。
    ///
    /// - 入参：`retries` 额外重试次数（`0` 即不重试）。
    /// - 加工：替换字段。
    /// - 出参：改动后的自身。
    pub fn retries(mut self, retries: u32) -> BinanceClient<T> {
        self.retries = retries;
        self
    }

    /// 发一次 GET 并把响应体按 JSON 解析，顺带把两种错误形态统一成 `Err`。
    ///
    /// - 入参：`url` 完整 URL（含 query string）。
    /// - 加工：交给传输层，**传输层失败时按 `retries` 重试**（每次退避 `pace` 的 4 倍）——
    ///   连接重置、响应体读一半断掉这类抖动在走代理的环境里很常见，重试一次通常就好；
    ///   非 2xx 与 Binance 的 `{"code":..,"msg":..}` 业务错误**不重试**，它们重试也不会变。
    ///   拿到响应体后解析成 [`Value`]。
    /// - 出参：`Ok(Value)`；重试用尽后仍传输失败、状态码非 2xx、JSON 不合法或带业务错误码时
    ///   返回 `Err`。
    async fn get_json(&self, url: String) -> Result<Value, String> {
        let mut attempt = 0;
        let resp = loop {
            match self.transport.send(HttpRequest::get(url.clone())).await {
                Ok(r) => break r,
                Err(e) if attempt < self.retries => {
                    attempt += 1;
                    let backoff = self.pace * 4 * attempt;
                    if !backoff.is_zero() {
                        tokio::time::sleep(backoff).await;
                    }
                    let _ = e;
                }
                Err(e) => return Err(format!("{e}（已重试 {attempt} 次）")),
            }
        };
        let v: Value = serde_json::from_str(&resp.body).map_err(|e| {
            format!(
                "解析 {url} 的响应失败：{e}；原文前 200 字节：{:.200}",
                resp.body
            )
        })?;
        // Binance 的业务错误是个带 code / msg 的对象，正常数据则是数组或带具体字段的对象
        if let (Some(code), Some(msg)) = (
            v.get("code").and_then(Value::as_i64),
            v.get("msg").and_then(Value::as_str),
        ) {
            return Err(format!("Binance 返回错误 {code}：{msg}（{url}）"));
        }
        if !resp.is_success() {
            return Err(format!("请求 {url} 返回 HTTP {}", resp.status));
        }
        Ok(v)
    }

    /// 在给定的候选市场 id 里，挑出**确实已上市**的那些。
    ///
    /// - 入参：`ids` 候选市场 id（如 `["ETHUSDT", "SOLUSDT"]`）。
    /// - 加工：先走快路径——`GET /api/v3/exchangeInfo?symbols=[...]` 一次性问全部候选，
    ///   响应只含这几个市场，几百字节。若其中有任何一个不存在，Binance 会对**整个请求**回
    ///   `-1121 Invalid symbol`，此时退到慢路径：逐个 `?symbol=XXX` 问一遍，`-1121` 即视为
    ///   未上市。慢路径只在真有下架标的时才走。
    /// - 出参：`Ok(已上市的 id 集合)`。**不按 `status == "TRADING"` 过滤**——ccxt 的
    ///   `exchange.symbols` 同样把非交易状态的市场列出来，与上游保持一致。
    ///
    /// 上游经由 ccxt 的 `load_markets()` 拉的是**全量**市场表（约 2 MB）。那份响应在走代理的
    /// 环境里很容易读一半就断（实测报 `error decoding response body`），而这里只需要判断几个
    /// 标的在不在，故收窄成按 id 查询。
    pub async fn listed_symbols(&self, ids: &[String]) -> Result<BTreeSet<String>, String> {
        if ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        let list = ids
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(",");
        let url = format!(
            "{}/api/v3/exchangeInfo?symbols={}",
            self.base_url,
            percent_encode(&format!("[{list}]"))
        );
        match self.get_json(url).await {
            Ok(v) => parse_symbols(&v),
            // 候选里有不存在的 id → 整个请求被拒，逐个问
            Err(e) if e.contains("-1121") => {
                let mut out = BTreeSet::new();
                for id in ids {
                    let url = format!("{}/api/v3/exchangeInfo?symbol={id}", self.base_url);
                    match self.get_json(url).await {
                        Ok(v) => out.extend(parse_symbols(&v)?),
                        Err(e) if e.contains("-1121") => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(out)
            }
            Err(e) => Err(e),
        }
    }

    /// 取一个市场在 `[start, end]` 区间内的全部 K 线，自动分页。
    ///
    /// - 入参：`market_id` Binance 市场 id（如 `"ETHUSDT"`）；`tf` 周期；
    ///   `start` / `end` 区间端点的 epoch 毫秒，语义是**开盘时刻**的闭区间，`None` 表示不限。
    ///   `start` 传 `None` 时只会拿到**最近 1000 根**——这是上游行为，如实保留。
    /// - 加工：游标从 `start` 起，每批最多 1000 根；`end` 同时作为 `endTime` 发给服务端**并**
    ///   在本地再滤一遍（服务端过滤的是开盘还是收盘时刻在文档上不够明确，本地兜一道
    ///   保证闭区间语义与上游一致）。每批取回后游标推进到"最后一根开盘时刻 + 1 毫秒"，
    ///   拿到空批、批不满 1000、或最后一根已越过 `end` 时停。批之间按 `pace` 节流。
    /// - 出参：`Ok(按时间升序的 K 线)`；任一批请求失败即整体 `Err`（不做重试）。
    ///
    /// # 与上游的差异：修掉一个死循环
    ///
    /// 上游把 `until` 只在客户端过滤，且游标推进写在 `if batch:` 里：
    ///
    /// ```text
    /// batch = [c for c in batch if c[0] <= until]   # 游标越过 until 后这里全滤空
    /// if original_batch_len < FETCH_BATCH_SIZE: break
    /// if batch: cursor = batch[-1][0] + 1           # batch 为空 → 游标不动
    /// ```
    ///
    /// 于是当 `end_date` 之后还有 ≥1000 根 K 线时，两个 break 都不触发、游标又不前进，
    /// 同一个请求无限重发。阈值：`1d` 约 2.7 年、`1h` 约 41 天、`1m` 约 16.7 小时。
    /// 这里把 `endTime` 交给服务端并按"批不满 / 越过 end"停，行为等价且不会挂。
    pub async fn klines(
        &self,
        market_id: &str,
        tf: Timeframe,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Result<Vec<Kline>, String> {
        let mut out: Vec<Kline> = Vec::new();
        let mut cursor = start;
        loop {
            let mut url = format!(
                "{}/api/v3/klines?symbol={market_id}&interval={}&limit={BATCH_LIMIT}",
                self.base_url,
                tf.as_str()
            );
            if let Some(c) = cursor {
                url.push_str(&format!("&startTime={c}"));
            }
            if let Some(e) = end {
                url.push_str(&format!("&endTime={e}"));
            }

            let batch = parse_klines(&self.get_json(url).await?)?;
            if batch.is_empty() {
                break;
            }
            let full = batch.len() >= BATCH_LIMIT;
            let last = batch[batch.len() - 1].open_time;
            // 服务端已按 endTime 过滤，这里再滤一道以确保"开盘时刻 <= end"的闭区间语义
            out.extend(
                batch
                    .into_iter()
                    .filter(|k| end.is_none_or(|e| k.open_time <= e)),
            );

            if !full || end.is_some_and(|e| last >= e) {
                break;
            }
            cursor = Some(last + 1);
            if !self.pace.is_zero() {
                tokio::time::sleep(self.pace).await;
            }
        }
        Ok(out)
    }
}

/// 从 `exchangeInfo` 响应里取出 `symbols[].symbol`。
///
/// - 入参：`v` 已解析的响应。
/// - 加工：定位 `symbols` 数组，逐项取 `symbol` 字段。
/// - 出参：`Ok(市场 id 集合)`；响应里没有 `symbols` 数组时返回 `Err`。
fn parse_symbols(v: &Value) -> Result<BTreeSet<String>, String> {
    let arr = v
        .get("symbols")
        .and_then(Value::as_array)
        .ok_or_else(|| "exchangeInfo 响应里没有 symbols 数组".to_string())?;
    Ok(arr
        .iter()
        .filter_map(|s| s.get("symbol").and_then(Value::as_str))
        .map(str::to_string)
        .collect())
}

/// 极简百分号编码：只转义 query string 里会引起歧义的那几个字符。
///
/// - 入参：`s` 待编码的原文（这里只会是 `["ETHUSDT","SOLUSDT"]` 这种 JSON 数组字面量）。
/// - 加工：字母、数字与 `-_.~` 原样保留，其余按 `%XX` 编码。
/// - 出参：可直接拼进 URL 的字符串。为这一个用途引一个 `urlencoding` 依赖不值当。
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 把 JSON 里的一格取成 `f64`。Binance 的价量都是**字符串**、时间是数字，两种都要认。
///
/// - 入参：`v` JSON 值。
/// - 加工：字符串走 `parse`，数字直接取。
/// - 出参：`Some(数值)`；其余类型或解析失败返回 `None`。
fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// 解析 `klines` 端点的响应。
///
/// - 入参：`v` 已解析成 [`Value`] 的响应，形如
///   `[[开盘时刻, "开", "高", "低", "收", "量", ...], ...]`。
/// - 加工：逐行取前 6 项；任一行结构不符即整体报错——价格数据错一格，下游的因子与回测
///   就全错，宁可失败也不要静默塞 NaN。
/// - 出参：`Ok(K 线向量)`，顺序即响应顺序（Binance 按开盘时刻升序返回）。
fn parse_klines(v: &Value) -> Result<Vec<Kline>, String> {
    let rows = v
        .as_array()
        .ok_or_else(|| format!("klines 响应不是数组：{:.200}", v))?;
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let cells = row
            .as_array()
            .ok_or_else(|| format!("klines 第 {i} 行不是数组"))?;
        if cells.len() < 6 {
            return Err(format!(
                "klines 第 {i} 行只有 {} 项，至少要 6 项",
                cells.len()
            ));
        }
        let bad = |field: &str| format!("klines 第 {i} 行的 {field} 解析失败");
        out.push(Kline {
            open_time: cells[0].as_i64().ok_or_else(|| bad("openTime"))?,
            open: as_f64(&cells[1]).ok_or_else(|| bad("open"))?,
            high: as_f64(&cells[2]).ok_or_else(|| bad("high"))?,
            low: as_f64(&cells[3]).ok_or_else(|| bad("low"))?,
            close: as_f64(&cells[4]).ok_or_else(|| bad("close"))?,
            volume: as_f64(&cells[5]).ok_or_else(|| bad("volume"))?,
        });
    }
    Ok(out)
}
