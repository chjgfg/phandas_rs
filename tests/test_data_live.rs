//! 真正联网的验收测试，全部标 `#[ignore]`。
//!
//! ```text
//! cargo test --features data --test test_data_live -- --ignored --nocapture --test-threads=1
//! ```
//!
//! 只打 Binance 的**公开**端点，不需要 API key。
//!
//! # 代理
//!
//! 每个测试开头都调 `setup()`，它从仓库根的 `.env` 读环境变量（文件不存在就跳过）。
//! 本地放一份：
//!
//! ```text
//! HTTPS_PROXY=http://127.0.0.1:7897
//! ```
//!
//! 服务器上不放这个文件即直连，两边同一份代码。`.env` 已在 `.gitignore` 里。
//! 真实环境变量优先于 `.env`，所以 `HTTPS_PROXY= cargo test ...` 能临时压过它。
//!
//! 断言刻意只钉「结构」不钉具体数值——行情每天都在变，能钉的是：列齐、时间戳单调去重、
//! 网格步长正确、区间端点符合上游口径。数值正确性由 `tests/test_data.rs` 的固定响应保证。

#![cfg(feature = "data")]

use std::sync::Once;

use phandas_rs::analysis::{analyze, IcMethod};
use phandas_rs::data::{fetch_data, Source, Timeframe};

/// 读一次 `.env`。用 `Once` 是因为 `set_var` 改的是进程全局状态，并行跑时不该重复写。
fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let n = phandas_rs::net::load_default_dotenv();
        let proxy = std::env::var("HTTPS_PROXY").unwrap_or_else(|_| "(未设置，直连)".to_string());
        println!("[setup] .env 写入 {n} 条，HTTPS_PROXY = {proxy}");
    });
}

/// 断言时间戳严格升序且无重复。
fn assert_monotonic(ts: &[String]) {
    for w in ts.windows(2) {
        assert!(w[0] < w[1], "时间戳应严格升序：{} 之后是 {}", w[0], w[1]);
    }
}

#[tokio::test]
#[ignore = "需要联网访问 api.binance.com，默认不跑"]
async fn live_daily_panel_has_expected_shape() {
    setup();
    let panel = fetch_data(
        &["ETH", "SOL"],
        Timeframe::D1,
        Some("2024-01-01"),
        Some("2024-01-31"),
        &[],
    )
    .await
    .expect("抓取成功");

    println!("{}", panel.info());
    assert_eq!(
        panel.column_names(),
        ["close", "high", "low", "open", "volume"]
    );
    assert_eq!(panel.symbols(), ["ETH", "SOL"]);
    assert_monotonic(panel.timestamps());
    // end_date 是当日 00:00:00Z 且过滤为闭区间，日线正好含当天那根
    assert_eq!(
        panel.timestamps().first().map(String::as_str),
        Some("2024-01-01")
    );
    assert_eq!(
        panel.timestamps().last().map(String::as_str),
        Some("2024-01-31")
    );
    assert_eq!(panel.timestamps().len(), 31, "1 月 31 天，日线每天一根");

    let close = panel.factor("close").expect("close");
    for v in close.values() {
        assert!(v.is_finite() && *v > 0.0, "收盘价应为正的有限数，得 {v}");
    }
}

#[tokio::test]
#[ignore = "需要联网访问 api.binance.com，默认不跑"]
async fn live_hourly_end_date_keeps_only_midnight_bar() {
    setup();
    // 上游口径：end_date 解释为当日 00:00:00Z，故日内周期下最后一天只留 00:00 那一根
    let panel = fetch_data(
        &["ETH"],
        Timeframe::H1,
        Some("2024-01-01"),
        Some("2024-01-03"),
        &[],
    )
    .await
    .expect("抓取成功");

    assert_monotonic(panel.timestamps());
    // 日内周期的时间戳带时分秒
    assert_eq!(
        panel.timestamps().first().map(String::as_str),
        Some("2024-01-01 00:00:00")
    );
    assert_eq!(
        panel.timestamps().last().map(String::as_str),
        Some("2024-01-03 00:00:00")
    );
    assert_eq!(
        panel.timestamps().len(),
        49,
        "两整天 48 根 + 第三天 00:00 一根"
    );
}

#[tokio::test]
#[ignore = "需要联网访问 api.binance.com，默认不跑"]
async fn live_weekly_grid_is_not_all_nan() {
    setup();
    // 上游把 1w 映射到 pandas 的 W（W-SUN），与 Binance 的周一开盘错位，整个面板出 NaN。
    // 这里应当拿到正常数据，且每格相隔 7 天、都落在周一。
    let panel = fetch_data(
        &["ETH"],
        Timeframe::W1,
        Some("2024-01-01"),
        Some("2024-03-01"),
        &[],
    )
    .await
    .expect("抓取成功");

    println!("{}", panel.info());
    let close = panel.factor("close").expect("close");
    assert!(
        close.values().iter().all(|v| v.is_finite()),
        "周线不该整块出 NaN：{:?}",
        close.values()
    );
    assert!(panel.timestamps().len() >= 8, "两个月至少 8 根周线");
}

#[tokio::test]
#[ignore = "需要联网访问 api.binance.com，默认不跑"]
async fn live_multi_source_and_downstream_chain() {
    setup();
    // 抓完直接接进现有的因子 / 评价链路，验证 Panel 能无缝喂下去
    let panel = fetch_data(
        &["ETH", "SOL", "ARB"],
        Timeframe::D1,
        Some("2024-01-01"),
        Some("2024-06-30"),
        &[Source::Binance, Source::Vwap, Source::Calendar],
    )
    .await
    .expect("抓取成功");

    println!("{}", panel.info());
    assert!(panel.column_names().contains(&"vwap"));
    assert!(panel.column_names().contains(&"dayofweek"));

    let close = panel.factor("close").expect("close");
    let alpha = close.rank().signal();
    let report = analyze(&[&alpha], &close, Some(&[1, 7])).expect("非空因子表");
    println!("{}", report.summary());
    let ic = report.ic(IcMethod::Spearman);
    let h1 = ic[0].for_horizon(1).expect("h=1");
    assert!(
        h1.ic_series.len() > 100,
        "半年日线应有上百期有效 IC，得 {}",
        h1.ic_series.len()
    );
}

#[tokio::test]
#[ignore = "需要联网访问 api.binance.com，默认不跑"]
async fn live_start_date_none_returns_recent_window_only() {
    setup();
    // 上游行为：start_date=None 时只拿最近 1000 根，不是全量历史
    let panel = fetch_data(&["ETH"], Timeframe::D1, None, None, &[])
        .await
        .expect("抓取成功");
    println!("{}", panel.info());
    assert!(
        panel.timestamps().len() <= 1000,
        "start_date=None 只给最近 1000 根，得 {}",
        panel.timestamps().len()
    );
}
