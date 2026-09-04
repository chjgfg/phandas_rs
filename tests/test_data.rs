//! `data` 模块的测试。跑法：`cargo test --features data`
//! `data` 模块的测试。跑法：`cargo test --features data --test test_data`
//!
//! 本机连不上交易所（`api.binance.com` 超时、`www.okx.com` DNS 失败），所以这里全部用
//! [`MockTransport`] 塞固定响应，覆盖「构造请求 → 分页 → 解析 → 对齐」整条链路。
//! 真正联网的验收在 `tests/test_data_live.rs`，全部标 `#[ignore]`。

#![cfg(feature = "data")]

use std::sync::{Arc, Mutex};

use phandas_rs::data::klines::BinanceClient;
use phandas_rs::data::{fetch_with, market_id, symbol, timeframe::Timeframe, Source};
use phandas_rs::net::{
    date_to_millis, millis_to_date, millis_to_datetime, HttpRequest, HttpResponse, HttpTransport,
};

/// 假传输层：按 `(URL, 该 URL 的第几次调用)` 交给闭包决定响应体，并记录全部收到的 URL。
#[derive(Clone)]
struct MockTransport {
    seen: Arc<Mutex<Vec<String>>>,
    #[allow(clippy::type_complexity)]
    handler: Arc<dyn Fn(&str, usize) -> String + Send + Sync>,
}

impl MockTransport {
    fn new(handler: impl Fn(&str, usize) -> String + Send + Sync + 'static) -> MockTransport {
        MockTransport {
            seen: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(handler),
        }
    }

    /// 至今收到的全部 URL，按顺序。
    fn urls(&self) -> Vec<String> {
        self.seen.lock().expect("mock 锁未被毒化").clone()
    }
}

impl HttpTransport for MockTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let nth = {
            let mut seen = self.seen.lock().expect("mock 锁未被毒化");
            let nth = seen.iter().filter(|u| **u == req.url).count();
            seen.push(req.url.clone());
            nth
        };
        Ok(HttpResponse {
            status: 200,
            body: (self.handler)(&req.url, nth),
        })
    }
}

/// 一天的毫秒数。
const DAY: i64 = 86_400_000;

/// 造一个 `exchangeInfo` 响应，只含给定的市场 id。
fn exchange_info(ids: &[&str]) -> String {
    let items: Vec<String> = ids
        .iter()
        .map(|s| format!(r#"{{"symbol":"{s}"}}"#))
        .collect();
    format!(r#"{{"symbols":[{}]}}"#, items.join(","))
}

/// 造一根 K 线的 JSON。价量按 Binance 的实际形态写成**字符串**，时间写成数字。
fn kline(open_time: i64, close: f64, volume: f64) -> String {
    format!(
        r#"[{open_time},"{o:.1}","{h:.1}","{l:.1}","{close:.1}","{volume:.1}",{ct},"0",0,"0","0","0"]"#,
        o = close - 1.0,
        h = close + 2.0,
        l = close - 2.0,
        ct = open_time + DAY - 1,
    )
}

/// 造一段连续日线的 JSON 数组：从 `start` 起 `n` 根，收盘价 `base + i`。
fn daily_klines(start: i64, n: usize, base: f64) -> String {
    let rows: Vec<String> = (0..n)
        .map(|i| kline(start + i as i64 * DAY, base + i as f64, 100.0 + i as f64))
        .collect();
    format!("[{}]", rows.join(","))
}

/// 取 URL 里某个查询参数的值。
fn param(url: &str, key: &str) -> Option<String> {
    url.split(['?', '&'])
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// 纯函数
// ---------------------------------------------------------------------------

#[test]
fn timeframe_parse_covers_every_binance_interval() {
    // 16 个合法 interval 一个不少，且往返一致
    let all = [
        "1s", "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d",
        "1w", "1M",
    ];
    for s in all {
        let tf = Timeframe::parse(s).unwrap_or_else(|e| panic!("{s} 应合法：{e}"));
        assert_eq!(tf.as_str(), s);
    }
    // 大小写敏感：1m 是分钟、1M 是月
    assert_ne!(
        Timeframe::parse("1m").unwrap(),
        Timeframe::parse("1M").unwrap()
    );
    // 上游对未知取值静默回落成日线并丢数据，这里必须报错
    for bad in ["1y", "7d", "10m", "", "1D", "1H"] {
        assert!(Timeframe::parse(bad).is_err(), "{bad} 应被拒");
    }
}

#[test]
fn timeframe_grid_matches_binance_phase() {
    // 定长周期直接加步长
    assert_eq!(Timeframe::D1.step_millis(), Some(DAY));
    assert_eq!(Timeframe::H4.step_millis(), Some(4 * 3_600_000));
    // 周线是 7 天，不是 pandas 的 W-SUN——上游那个映射让周线整块出 NaN
    assert_eq!(Timeframe::W1.step_millis(), Some(7 * DAY));
    let monday = date_to_millis("2024-01-01").unwrap(); // 2024-01-01 是周一
    assert_eq!(millis_to_date(Timeframe::W1.advance(monday)), "2024-01-08");

    // 月线没有固定步长，走自然月进位
    assert_eq!(Timeframe::Mon1.step_millis(), None);
    let jan = date_to_millis("2024-01-01").unwrap();
    assert_eq!(millis_to_date(Timeframe::Mon1.advance(jan)), "2024-02-01");
    let dec = date_to_millis("2024-12-01").unwrap();
    assert_eq!(millis_to_date(Timeframe::Mon1.advance(dec)), "2025-01-01");
    // 闰年 2 月
    let jan31 = date_to_millis("2024-01-31").unwrap();
    assert_eq!(millis_to_date(Timeframe::Mon1.advance(jan31)), "2024-02-29");

    // 日内与日级的时间戳格式不同
    assert!(Timeframe::H1.is_intraday() && Timeframe::M1.is_intraday());
    assert!(!Timeframe::D1.is_intraday() && !Timeframe::W1.is_intraday());
}

#[test]
fn time_conversions_are_utc_and_round_trip() {
    let ms = date_to_millis("2024-03-15").unwrap();
    assert_eq!(millis_to_date(ms), "2024-03-15");
    assert_eq!(millis_to_datetime(ms), "2024-03-15 00:00:00");
    assert_eq!(
        millis_to_datetime(ms + 3_600_000 + 61_000),
        "2024-03-15 01:01:01"
    );
    // 带时间的入参：时间部分一律忽略，对齐上游 parse8601(f'{date}T00:00:00Z')
    assert_eq!(date_to_millis("2024-03-15 08:30:00"), Some(ms));
    assert_eq!(date_to_millis("2024-03-15T08:30:00"), Some(ms));
    // 非法日期
    assert_eq!(date_to_millis("2023-02-29"), None);
    assert_eq!(date_to_millis("not-a-date"), None);
    // 1970 之前也成立
    assert_eq!(millis_to_date(-DAY), "1969-12-31");
}

#[test]
fn symbol_mapping_and_rename_table() {
    assert_eq!(market_id("ETH"), "ETHUSDT");
    // 上游把计价币种写死 USDT；传进带斜杠的名字会拼出查不到的 id，于是该标的被跳过
    assert_eq!(market_id("ETH/USDT"), "ETH/USDTUSDT");

    let pol = symbol::rename_for("POL").expect("POL 有历史改名");
    assert_eq!(pol.old_symbol, "MATIC");
    assert_eq!(pol.cutoff_date, "2024-09-01");
    assert!(symbol::rename_for("ETH").is_none());
    assert_eq!(symbol::SYMBOL_RENAMES.len(), 1);
}

#[test]
fn source_parse_rejects_unknown() {
    assert_eq!(Source::parse("binance").unwrap(), Source::Binance);
    assert_eq!(Source::parse("vwap").unwrap(), Source::Vwap);
    // 上游对未知源发 warning 后静默跳过，这里拒掉
    let err = Source::parse("bybit").expect_err("未知源应报错");
    assert!(err.contains("bybit") && err.contains("binance"));
}

// ---------------------------------------------------------------------------
// 分页
// ---------------------------------------------------------------------------

#[tokio::test]
async fn klines_pagination_advances_cursor_and_stops_on_short_batch() {
    let start = date_to_millis("2024-01-01").unwrap();
    let mock = MockTransport::new(move |url, _| {
        if url.contains("exchangeInfo") {
            return exchange_info(&["ETHUSDT"]);
        }
        match param(url, "startTime") {
            // 首批满 1000 根 → 应继续要下一批
            None => daily_klines(start, 1000, 10.0),
            // 第二批只有 3 根，不满 1000 → 停
            Some(_) => daily_klines(start + 1000 * DAY, 3, 1010.0),
        }
    });
    let client = BinanceClient::with_transport(mock.clone()).pace(std::time::Duration::ZERO);
    let ks = client
        .klines("ETHUSDT", Timeframe::D1, None, None)
        .await
        .expect("抓取成功");

    assert_eq!(ks.len(), 1003, "1000 + 3 根");
    assert_eq!(ks[0].open_time, start);
    assert_eq!(ks[0].close, 10.0);
    // 游标推进到「上一批最后一根 + 1 毫秒」
    let urls = mock.urls();
    assert_eq!(urls.len(), 2, "满批后再要一次，不满即停：{urls:?}");
    assert_eq!(
        param(&urls[1], "startTime"),
        Some((start + 999 * DAY + 1).to_string())
    );
    assert_eq!(param(&urls[0], "limit"), Some("1000".to_string()));
    assert_eq!(param(&urls[0], "interval"), Some("1d".to_string()));
}

#[tokio::test]
async fn klines_terminates_even_if_server_ignores_end_time() {
    // 回归上游那个死循环：`end_date` 之后还有 ≥1000 根时，上游的游标不推进、两个 break 都不触发。
    // 这里让假服务端**无视 endTime**，每次都回满满 1000 根越过区间末点的数据。
    let start = date_to_millis("2024-01-01").unwrap();
    let end = date_to_millis("2024-01-05").unwrap();
    let mock = MockTransport::new(move |url, _| {
        if url.contains("exchangeInfo") {
            return exchange_info(&["ETHUSDT"]);
        }
        // 全部落在 end 之后，且批是满的
        daily_klines(end + DAY, 1000, 50.0)
    });
    let client = BinanceClient::with_transport(mock.clone()).pace(std::time::Duration::ZERO);
    let ks = client
        .klines("ETHUSDT", Timeframe::D1, Some(start), Some(end))
        .await
        .expect("必须返回，不能挂死");

    assert!(ks.is_empty(), "越过 end 的数据应被滤掉");
    assert_eq!(mock.urls().len(), 1, "发现越过 end 就停，不再重发");
    // endTime 确实发给了服务端
    assert_eq!(param(&mock.urls()[0], "endTime"), Some(end.to_string()));
}

#[tokio::test]
async fn klines_reports_binance_business_error() {
    let mock = MockTransport::new(|_, _| r#"{"code":-1121,"msg":"Invalid symbol."}"#.to_string());
    let client = BinanceClient::with_transport(mock);
    let err = client
        .klines("NOPEUSDT", Timeframe::D1, None, None)
        .await
        .expect_err("业务错误码应变成 Err");
    assert!(
        err.contains("-1121") && err.contains("Invalid symbol"),
        "{err}"
    );
}

// ---------------------------------------------------------------------------
// fetch_with：端到端
// ---------------------------------------------------------------------------

/// 两个标的、各 5 根日线的假服务端；`AAA` 从 01-01 起、`BBB` 晚两天才上市。
fn two_symbol_mock() -> MockTransport {
    let d0 = date_to_millis("2024-01-01").unwrap();
    MockTransport::new(move |url, nth| {
        if url.contains("exchangeInfo") {
            return exchange_info(&["AAAUSDT", "BBBUSDT", "BTCUSDT", "ETHUSDT"]);
        }
        if nth > 0 {
            return "[]".to_string();
        }
        match param(url, "symbol").as_deref() {
            Some("AAAUSDT") => daily_klines(d0, 5, 10.0),
            Some("BBBUSDT") => daily_klines(d0 + 2 * DAY, 3, 20.0),
            Some("BTCUSDT") => daily_klines(d0, 5, 60000.0),
            Some("ETHUSDT") => daily_klines(d0, 5, 3000.0),
            _ => "[]".to_string(),
        }
    })
}

#[tokio::test]
async fn fetch_with_builds_panel_and_truncates_to_latest_listing() {
    let client = BinanceClient::with_transport(two_symbol_mock()).pace(std::time::Duration::ZERO);
    let panel = fetch_with(&client, &["AAA", "BBB"], Timeframe::D1, None, None, &[])
        .await
        .expect("抓取成功");

    assert_eq!(
        panel.column_names(),
        ["close", "high", "low", "open", "volume"]
    );
    assert_eq!(panel.symbols(), ["AAA", "BBB"]);
    // common_start = 各标的首个有效收盘的最大值 = BBB 的上市日 01-03，之前的全丢
    assert_eq!(
        panel.timestamps(),
        ["2024-01-03", "2024-01-04", "2024-01-05"]
    );

    let close = panel.factor("close").expect("close");
    // AAA 的 01-03 是它的第 3 根，收盘 12.0
    assert_eq!(close.get("2024-01-03", "AAA"), Some(12.0));
    assert_eq!(close.get("2024-01-03", "BBB"), Some(20.0));
    assert_eq!(close.get("2024-01-05", "BBB"), Some(22.0));
}

#[tokio::test]
async fn fetch_with_forward_fills_gaps_without_limit() {
    // BBB 中间缺 01-03，网格上该格由前一格无界前向填充（上游行为，价量都填）
    let d0 = date_to_millis("2024-01-01").unwrap();
    let mock = MockTransport::new(move |url, nth| {
        if url.contains("exchangeInfo") {
            return exchange_info(&["AAAUSDT", "BBBUSDT"]);
        }
        if nth > 0 {
            return "[]".to_string();
        }
        match param(url, "symbol").as_deref() {
            Some("AAAUSDT") => daily_klines(d0, 4, 10.0),
            // 只给 01-01、01-02、01-04：01-03 是空洞
            Some("BBBUSDT") => format!(
                "[{},{},{}]",
                kline(d0, 20.0, 200.0),
                kline(d0 + DAY, 21.0, 210.0),
                kline(d0 + 3 * DAY, 23.0, 230.0)
            ),
            _ => "[]".to_string(),
        }
    });
    let client = BinanceClient::with_transport(mock).pace(std::time::Duration::ZERO);
    let panel = fetch_with(&client, &["AAA", "BBB"], Timeframe::D1, None, None, &[])
        .await
        .expect("抓取成功");

    let close = panel.factor("close").expect("close");
    let vol = panel.factor("volume").expect("volume");
    assert_eq!(close.get("2024-01-02", "BBB"), Some(21.0));
    assert_eq!(
        close.get("2024-01-03", "BBB"),
        Some(21.0),
        "空洞由前一格填上"
    );
    assert_eq!(
        vol.get("2024-01-03", "BBB"),
        Some(210.0),
        "成交量也照填，不置 0"
    );
    assert_eq!(close.get("2024-01-04", "BBB"), Some(23.0));
}

#[tokio::test]
async fn fetch_with_merges_multiple_sources() {
    let client = BinanceClient::with_transport(two_symbol_mock()).pace(std::time::Duration::ZERO);
    let panel = fetch_with(
        &client,
        &["AAA", "BBB"],
        Timeframe::D1,
        Some("2024-01-01"),
        Some("2024-01-05"),
        &[Source::Binance, Source::Benchmark, Source::Calendar],
    )
    .await
    .expect("抓取成功");

    // 三个源的列合起来，列名升序（Panel 内部就是这个序）
    assert_eq!(
        panel.column_names(),
        [
            "BTC_close",
            "ETH_close",
            "close",
            "day",
            "dayofmonth_position",
            "dayofweek",
            "high",
            "is_week_end",
            "low",
            "month",
            "open",
            "volume",
            "year",
        ]
    );

    // benchmark：BTC / ETH 的收盘价广播到每个标的，同一时刻两个标的拿到同一个值
    let btc = panel.factor("BTC_close").expect("BTC_close");
    assert_eq!(btc.get("2024-01-03", "AAA"), Some(60002.0));
    assert_eq!(btc.get("2024-01-03", "BBB"), Some(60002.0));

    // calendar：2024-01-03 是周三 → dayofweek = 3；上旬 → dayofmonth_position = 1；非周末
    let dow = panel.factor("dayofweek").expect("dayofweek");
    let pos = panel
        .factor("dayofmonth_position")
        .expect("dayofmonth_position");
    let wend = panel.factor("is_week_end").expect("is_week_end");
    assert_eq!(dow.get("2024-01-03", "AAA"), Some(3.0));
    assert_eq!(pos.get("2024-01-03", "AAA"), Some(1.0));
    assert_eq!(wend.get("2024-01-03", "AAA"), Some(0.0));
    // 2024-01-06 是周六，但它在 common_start 之后、end 之内吗？end=01-05，故不在网格里
    assert_eq!(
        panel.timestamps().last().map(String::as_str),
        Some("2024-01-05")
    );
}

#[tokio::test]
async fn calendar_alone_fails_without_close() {
    // 对齐必须以 close 为准，单用 calendar 上游会在 pivot_table 里抛未捕获的 KeyError
    let mock = MockTransport::new(|_, _| exchange_info(&["AAAUSDT"]));
    let client = BinanceClient::with_transport(mock);
    let err = fetch_with(
        &client,
        &["AAA"],
        Timeframe::D1,
        Some("2024-01-01"),
        Some("2024-01-05"),
        &[Source::Calendar],
    )
    .await
    .expect_err("缺 close 应报错而不是 panic");
    assert!(err.contains("close"), "{err}");
}

#[tokio::test]
async fn calendar_requires_both_dates() {
    let mock = MockTransport::new(|_, _| exchange_info(&["AAAUSDT"]));
    let client = BinanceClient::with_transport(mock);
    let err = fetch_with(
        &client,
        &["AAA"],
        Timeframe::D1,
        None,
        None,
        &[Source::Calendar],
    )
    .await
    .expect_err("日历源缺日期应报错");
    assert!(err.contains("Calendar requires both"), "{err}");
}

#[tokio::test]
async fn unlisted_symbol_is_skipped() {
    // BBB 不在 exchangeInfo 里 → 整个跳过（上游发 warning 后 return None）
    let d0 = date_to_millis("2024-01-01").unwrap();
    let mock = MockTransport::new(move |url, nth| {
        if url.contains("exchangeInfo") {
            return exchange_info(&["AAAUSDT"]);
        }
        if nth > 0 {
            return "[]".to_string();
        }
        daily_klines(d0, 3, 10.0)
    });
    let client = BinanceClient::with_transport(mock.clone()).pace(std::time::Duration::ZERO);
    let panel = fetch_with(&client, &["AAA", "BBB"], Timeframe::D1, None, None, &[])
        .await
        .expect("抓取成功");
    assert_eq!(panel.symbols(), ["AAA"]);
    // 没给 BBB 发过 klines 请求（exchangeInfo 的存在性探测里会带上它的 id，那是正常的）
    assert!(
        !mock
            .urls()
            .iter()
            .any(|u| u.contains("/klines") && u.contains("BBBUSDT")),
        "未上市的标的不该发 klines 请求：{:?}",
        mock.urls()
    );
}

#[tokio::test]
async fn rename_splices_old_and_new_symbol() {
    // POL 在 2024-09-01 之前叫 MATIC：老段用 MATIC 拉、改写成 POL 后与新段拼接。
    // 上游还会连带把其余标的一起重抓一遍（老段的符号表是 ['MATIC'] + 其余），这里一并核对。
    let cutoff = date_to_millis("2024-09-01").unwrap();
    let mock = MockTransport::new(move |url, nth| {
        if url.contains("exchangeInfo") {
            return exchange_info(&["MATICUSDT", "POLUSDT", "AAAUSDT"]);
        }
        if nth > 0 {
            return "[]".to_string();
        }
        let sym = param(url, "symbol").unwrap_or_default();
        let end = param(url, "endTime").and_then(|s| s.parse::<i64>().ok());
        let old_leg = end == Some(cutoff - 1);
        match (sym.as_str(), old_leg) {
            // 老段：分界日前两天
            ("MATICUSDT", true) => daily_klines(cutoff - 2 * DAY, 2, 40.0),
            ("AAAUSDT", true) => daily_klines(cutoff - 2 * DAY, 2, 10.0),
            // 新段：分界日起两天
            ("POLUSDT", false) => daily_klines(cutoff, 2, 50.0),
            ("AAAUSDT", false) => daily_klines(cutoff, 2, 12.0),
            _ => "[]".to_string(),
        }
    });
    let client = BinanceClient::with_transport(mock.clone()).pace(std::time::Duration::ZERO);
    let panel = fetch_with(
        &client,
        &["POL", "AAA"],
        Timeframe::D1,
        Some("2024-08-01"),
        None,
        &[],
    )
    .await
    .expect("抓取成功");

    // 输出里只有 POL，没有 MATIC——辅助符号被最后一道 user_symbols 过滤剔掉
    assert_eq!(panel.symbols(), ["AAA", "POL"]);
    let close = panel.factor("close").expect("close");
    // 老段的 MATIC 数据挂到了 POL 名下
    assert_eq!(close.get("2024-08-30", "POL"), Some(40.0));
    assert_eq!(close.get("2024-08-31", "POL"), Some(41.0));
    // 新段
    assert_eq!(close.get("2024-09-01", "POL"), Some(50.0));
    assert_eq!(close.get("2024-09-02", "POL"), Some(51.0));

    // 老段确实用旧符号发过请求，且带着 cutoff - 1 的 endTime
    let urls = mock.urls();
    assert!(
        urls.iter().any(|u| u.contains("/klines")
            && u.contains("MATICUSDT")
            && param(u, "endTime") == Some((cutoff - 1).to_string())),
        "老段应用 MATIC + endTime=cutoff-1：{urls:?}"
    );
    // 其余标的也被两段各抓一次（上游行为）
    assert_eq!(
        urls.iter()
            .filter(|u| u.contains("/klines") && u.contains("AAAUSDT"))
            .count(),
        2,
        "非改名标的也是两段拼起来的：{urls:?}"
    );
}

#[tokio::test]
async fn listed_symbols_narrows_the_query_and_falls_back_on_invalid_id() {
    // 快路径：一次 ?symbols=[...] 问全部候选，响应只含这几个市场（上游走全量 2 MB 的表，
    // 走代理时容易读一半就断）
    let ok = MockTransport::new(|_, _| exchange_info(&["AAAUSDT", "BBBUSDT"]));
    let client = BinanceClient::with_transport(ok.clone());
    let got = client
        .listed_symbols(&["AAAUSDT".to_string(), "BBBUSDT".to_string()])
        .await
        .expect("快路径成功");
    assert_eq!(got.len(), 2);
    assert_eq!(ok.urls().len(), 1, "只发一次请求");
    let u = &ok.urls()[0];
    assert!(u.contains("symbols="), "应按 id 收窄查询：{u}");
    assert!(u.contains("%5B%22AAAUSDT%22"), "JSON 数组要百分号编码：{u}");

    // 慢路径：候选里有不存在的 id 时，Binance 对整个请求回 -1121，退到逐个问
    let fallback = MockTransport::new(|url, _| {
        if url.contains("symbols=") {
            return r#"{"code":-1121,"msg":"Invalid symbol."}"#.to_string();
        }
        match param(url, "symbol").as_deref() {
            Some("AAAUSDT") => exchange_info(&["AAAUSDT"]),
            _ => r#"{"code":-1121,"msg":"Invalid symbol."}"#.to_string(),
        }
    });
    let client = BinanceClient::with_transport(fallback.clone()).pace(std::time::Duration::ZERO);
    let got = client
        .listed_symbols(&["AAAUSDT".to_string(), "ZZZUSDT".to_string()])
        .await
        .expect("慢路径也要成功");
    assert_eq!(
        got.iter().map(String::as_str).collect::<Vec<_>>(),
        ["AAAUSDT"],
        "下架的 id 被判为未上市，不该让整次抓取失败"
    );
    // 1 次批量 + 2 次逐个
    assert_eq!(fallback.urls().len(), 3, "{:?}", fallback.urls());
}

#[tokio::test]
async fn transport_errors_are_retried_business_errors_are_not() {
    // 传输层抖动（走代理时 `error decoding response body` 很常见）应重试
    let flaky = Arc::new(Mutex::new(0u32));
    let counter = flaky.clone();
    struct Flaky {
        hits: Arc<Mutex<u32>>,
        body: String,
    }
    impl HttpTransport for Flaky {
        async fn send(&self, _req: HttpRequest) -> Result<HttpResponse, String> {
            let mut n = self.hits.lock().expect("锁未毒化");
            *n += 1;
            if *n < 3 {
                return Err("error decoding response body".to_string());
            }
            Ok(HttpResponse {
                status: 200,
                body: self.body.clone(),
            })
        }
    }
    let client = BinanceClient::with_transport(Flaky {
        hits: counter,
        body: exchange_info(&["AAAUSDT"]),
    })
    .pace(std::time::Duration::ZERO);
    let got = client
        .listed_symbols(&["AAAUSDT".to_string()])
        .await
        .expect("前两次失败后第三次应成功");
    assert_eq!(got.len(), 1);
    assert_eq!(*flaky.lock().expect("锁未毒化"), 3, "默认重试 2 次");

    // 业务错误码不重试：重试也不会变
    let biz = MockTransport::new(|_, _| r#"{"code":-1100,"msg":"Illegal chars."}"#.to_string());
    let client = BinanceClient::with_transport(biz.clone());
    assert!(client
        .klines("AAAUSDT", Timeframe::D1, None, None)
        .await
        .is_err());
    assert_eq!(biz.urls().len(), 1, "业务错误只发一次");
}

#[tokio::test]
async fn empty_symbols_and_no_data_are_errors() {
    let mock = MockTransport::new(|_, _| exchange_info(&["AAAUSDT"]));
    let client = BinanceClient::with_transport(mock);
    assert!(fetch_with(&client, &[], Timeframe::D1, None, None, &[])
        .await
        .is_err());

    // 有市场但一根 K 线都没有
    let empty = MockTransport::new(|url, _| {
        if url.contains("exchangeInfo") {
            exchange_info(&["AAAUSDT"])
        } else {
            "[]".to_string()
        }
    });
    let client = BinanceClient::with_transport(empty);
    let err = fetch_with(&client, &["AAA"], Timeframe::D1, None, None, &[])
        .await
        .expect_err("无数据应报错");
    assert!(
        err.contains("No data fetched") || err.contains("close"),
        "{err}"
    );
}

#[tokio::test]
async fn bad_date_is_rejected() {
    let mock = MockTransport::new(|_, _| exchange_info(&["AAAUSDT"]));
    let client = BinanceClient::with_transport(mock);
    let err = fetch_with(
        &client,
        &["AAA"],
        Timeframe::D1,
        Some("2023-02-29"),
        None,
        &[],
    )
    .await
    .expect_err("非法日期应报错");
    assert!(err.contains("start_date"), "{err}");
}
