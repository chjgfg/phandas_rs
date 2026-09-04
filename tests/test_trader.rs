//! `trader` 模块的离线测试。跑法：`cargo test --features trader`
//!
//! 本机连不上 OKX（`www.okx.com` DNS 解析失败），所以这里只测**不需要网络**的部分：
//! 签名、凭证、合约换算。签名的期望值由 Python 标准库 `hmac` / `hashlib` / `base64` 现算
//! （不是从记忆里抄的），算式与 OKX 文档一致：
//!
//! ```python
//! pre = timestamp + method + request_path + body
//! base64.b64encode(hmac.new(secret.encode(), pre.encode(), hashlib.sha256).digest())
//! ```
//!
//! 真发单的验收只能在模拟盘上做，见 `docs/上游能力清单与移植对照.md`。

#![cfg(feature = "trader")]

use phandas_rs::net::Method;
use phandas_rs::trader::{
    auth::{signed_request_at, ENV_KEYS},
    sign, Credentials, Environment, InstrumentSpec,
};

/// OKX 文档示例里那把私钥。
const SECRET: &str = "22582BD0CFF14C41EDBF1AB98506286D";
/// 文档示例里那个时刻。
const TS: &str = "2020-12-08T09:08:57.715Z";

fn creds() -> Credentials {
    Credentials::new("my-api-key-0123456789", SECRET, "my-pass").expect("三项都非空")
}

// ---------------------------------------------------------------------------
// 签名
// ---------------------------------------------------------------------------

#[test]
fn sign_matches_reference_vectors() {
    // GET，requestPath 带 query string，body 为空
    assert_eq!(
        sign(
            SECRET,
            TS,
            Method::Get,
            "/api/v5/account/balance?ccy=BTC",
            ""
        ),
        "HiZhvSfMtWJA3uUIVXV3a/bSXNPCWvYFXoGCVS8V4zY="
    );

    // POST，body 是**顶层 JSON 数组**——批量下单就是这个形状，最容易签错的一处
    let batch =
        r#"[{"instId":"BTC-USDT-SWAP","tdMode":"cross","side":"buy","ordType":"market","sz":"1"}]"#;
    assert_eq!(
        sign(
            SECRET,
            TS,
            Method::Post,
            "/api/v5/trade/batch-orders",
            batch
        ),
        "ksK+seQE9JYiQbEblgI6+kyOkHjWyQKzIi3PfGLu7Z4="
    );

    // 另一把短私钥 + 另一个时刻，确认没有隐式的长度或补齐假设
    assert_eq!(
        sign(
            "secret",
            "2026-09-04T09:45:45.328Z",
            Method::Get,
            "/api/v5/account/positions?instType=SWAP",
            ""
        ),
        "iMw7xMTPtGYO4mjJ01ThTglG+E4dl+fYN/ZECZ54NVQ="
    );
}

#[test]
fn sign_is_sensitive_to_every_component() {
    let base = sign(
        SECRET,
        TS,
        Method::Get,
        "/api/v5/account/balance?ccy=BTC",
        "",
    );
    // 丢掉 query string 就是另一个签名——这正是 Invalid Signature 最常见的成因
    assert_ne!(
        base,
        sign(SECRET, TS, Method::Get, "/api/v5/account/balance", "")
    );
    // 方法、时刻、私钥、body 各改一处都应变
    assert_ne!(
        base,
        sign(
            SECRET,
            TS,
            Method::Post,
            "/api/v5/account/balance?ccy=BTC",
            ""
        )
    );
    assert_ne!(
        base,
        sign(
            SECRET,
            "2020-12-08T09:08:57.716Z",
            Method::Get,
            "/api/v5/account/balance?ccy=BTC",
            ""
        )
    );
    assert_ne!(
        base,
        sign(
            "other",
            TS,
            Method::Get,
            "/api/v5/account/balance?ccy=BTC",
            ""
        )
    );
    assert_ne!(
        base,
        sign(
            SECRET,
            TS,
            Method::Get,
            "/api/v5/account/balance?ccy=BTC",
            "{}"
        )
    );
}

#[test]
fn signed_request_carries_all_auth_headers() {
    let req = signed_request_at(
        &creds(),
        Environment::Demo,
        "https://www.okx.com",
        Method::Get,
        "/api/v5/account/positions?instType=SWAP",
        "",
        TS,
    );
    assert_eq!(req.method, Method::Get);
    assert_eq!(
        req.url,
        "https://www.okx.com/api/v5/account/positions?instType=SWAP"
    );
    assert_eq!(req.body, "");

    let h: Vec<(&str, &str)> = req
        .headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        h,
        vec![
            ("OK-ACCESS-KEY", "my-api-key-0123456789"),
            (
                "OK-ACCESS-SIGN",
                sign(
                    SECRET,
                    TS,
                    Method::Get,
                    "/api/v5/account/positions?instType=SWAP",
                    ""
                )
                .as_str()
            ),
            ("OK-ACCESS-TIMESTAMP", TS),
            ("OK-ACCESS-PASSPHRASE", "my-pass"),
            ("Content-Type", "application/json"),
            ("x-simulated-trading", "1"),
        ]
    );
}

#[test]
fn live_and_demo_differ_only_in_the_simulated_header() {
    let mk = |env| {
        signed_request_at(
            &creds(),
            env,
            "https://www.okx.com",
            Method::Post,
            "/api/v5/trade/order",
            "{}",
            TS,
        )
    };
    let (demo, live) = (mk(Environment::Demo), mk(Environment::Live));
    // 同一个域名、同一个签名——OKX 靠头区分环境，不是靠 URL
    assert_eq!(demo.url, live.url);
    assert_eq!(demo.headers[1], live.headers[1]);
    assert_eq!(demo.headers[5].1, "1");
    assert_eq!(live.headers[5].1, "0");
    assert!(Environment::Live.is_live() && !Environment::Demo.is_live());
}

// ---------------------------------------------------------------------------
// 凭证
// ---------------------------------------------------------------------------

#[test]
fn credentials_reject_blank_fields() {
    assert!(Credentials::new("", "s", "p")
        .expect_err("空 key")
        .contains("api_key"));
    assert!(Credentials::new("k", "  ", "p")
        .expect_err("空 secret")
        .contains("secret_key"));
    assert!(Credentials::new("k", "s", "\t")
        .expect_err("空 pass")
        .contains("passphrase"));
    // 两端空白会被去掉
    let c = Credentials::new("  k  ", "s", "p").expect("去空白后非空");
    assert_eq!(c.api_key(), "k");
}

#[test]
fn credentials_debug_never_leaks_secrets() {
    let c = Credentials::new("abcdefgh12345678", "super-secret-key", "my-passphrase").expect("ok");
    let s = format!("{c:?}");
    // 私钥与 passphrase 一个字都不能出现
    assert!(!s.contains("super-secret-key"), "泄露了 secret_key：{s}");
    assert!(!s.contains("my-passphrase"), "泄露了 passphrase：{s}");
    assert!(s.contains("<redacted>"), "{s}");
    // api_key 只留前 4 字符 + 长度
    assert!(s.contains("abcd…(16)"), "{s}");
    assert!(!s.contains("abcdefgh12345678"), "api_key 不该全量输出：{s}");
}

#[test]
fn credentials_from_env_reads_the_three_keys() {
    // 这三个变量名没有别的测试用，串行与否都不冲突
    for (k, v) in ENV_KEYS.iter().zip(["env-key", "env-secret", "env-pass"]) {
        std::env::set_var(k, v);
    }
    let c = Credentials::from_env().expect("三个变量都设了");
    assert_eq!(c.api_key(), "env-key");
    assert_eq!(
        ENV_KEYS,
        ["OKX_API_KEY", "OKX_SECRET_KEY", "OKX_PASSPHRASE"]
    );

    std::env::remove_var("OKX_SECRET_KEY");
    let err = Credentials::from_env().expect_err("缺一个就该报错");
    assert!(
        err.contains("OKX_SECRET_KEY"),
        "错误信息要指名缺哪个：{err}"
    );
    for k in ENV_KEYS {
        std::env::remove_var(k);
    }
}

// ---------------------------------------------------------------------------
// 合约换算
// ---------------------------------------------------------------------------

/// 造一份 instrument 响应项的 JSON 文本；`skip` 里的字段不写出来。
/// OKX 的数值字段全是**字符串**，这里照那个形态造。
fn spec_json_full(ct_val: &str, lot_sz: &str, min_sz: &str, skip: &[&str]) -> String {
    let fields = [
        ("instId", "BTC-USDT-SWAP"),
        ("state", "live"),
        ("ctVal", ct_val),
        ("ctMult", "1"),
        ("lotSz", lot_sz),
        ("minSz", min_sz),
        ("tickSz", "0.1"),
        ("maxMktSz", "1000"),
    ];
    let body: Vec<String> = fields
        .iter()
        .filter(|(k, _)| !skip.contains(k))
        .map(|(k, v)| format!("\"{k}\":\"{v}\""))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// 完整的一项。
fn spec_json(ct_val: &str, lot_sz: &str, min_sz: &str) -> String {
    spec_json_full(ct_val, lot_sz, min_sz, &[])
}

/// 缺某个字段的一项，用来验「字段缺失」的分支。
fn spec_json_without(field: &str) -> String {
    spec_json_full("0.01", "0.1", "0.1", &[field])
}

#[test]
fn spec_parses_okx_string_numbers() {
    let s = InstrumentSpec::from_json_str(&spec_json("0.01", "0.1", "0.1")).expect("可解析");
    assert_eq!(s.inst_id, "BTC-USDT-SWAP");
    assert!(s.is_live());
    assert_eq!(s.ct_val, 0.01);
    assert_eq!(s.ct_mult, 1.0);
    assert_eq!(s.lot_sz, 0.1);
    assert_eq!(s.min_sz, 0.1);
    assert_eq!(s.max_mkt_sz, 1000.0);
    assert_eq!(s.size_decimals, 1, "小数位数取自 lotSz");
    // 一张 = 0.01 BTC；BTC 报 60000 时单张名义额 600
    assert_eq!(s.coin_per_contract(), 0.01);
    assert_eq!(s.notional_per_contract(60_000.0), 600.0);

    // ctMult 缺失按 1 处理
    let v = spec_json_without("ctMult");
    assert!(!v.contains("ctMult"), "辅助函数没删掉字段：{v}");
    assert_eq!(InstrumentSpec::from_json_str(&v).expect("ok").ct_mult, 1.0);

    // ctVal / lotSz 非正就不能静默当 0 用
    assert!(InstrumentSpec::from_json_str(&spec_json("0", "0.1", "0.1")).is_err());
    assert!(InstrumentSpec::from_json_str(&spec_json("0.01", "0", "0.1")).is_err());
}

#[test]
fn contracts_round_down_never_up() {
    // 一张 = 0.01 BTC，价 60000 → 单张名义额 600；lotSz = 0.1 张 = 60 USD 一格
    let s = InstrumentSpec::from_json_str(&spec_json("0.01", "0.1", "0.1")).expect("ok");

    // 1000 USD / 600 = 1.6667 张 → 向下对齐到 1.6 张
    let c = s
        .contracts_for_notional(1000.0, 60_000.0)
        .expect("够 minSz");
    assert!((c - 1.6).abs() < 1e-12, "得 {c}");
    // 实际成交名义额小于目标，绝不超出
    let got = s.notional_of(c, 60_000.0);
    assert!(got <= 1000.0 + 1e-9, "向下取整不该超出目标：{got}");
    assert!((got - 960.0).abs() < 1e-9, "得 {got}");

    // 正好落在格子上时不该少掉一格：0.3 / 0.1 在二进制里是 2.9999…，裸 floor 会给 0.2
    let c = s
        .contracts_for_notional(180.0, 60_000.0)
        .expect("正好 0.3 张");
    assert!((c - 0.3).abs() < 1e-12, "得 {c}，裸 floor 会错成 0.2");
}

#[test]
fn min_size_is_rejected_locally_with_a_reason() {
    let s = InstrumentSpec::from_json_str(&spec_json("0.01", "0.1", "0.5")).expect("ok");
    // 目标 1.2 USD，单张 600 USD → 向下取整是 0 张，远低于 minSz
    let err = s
        .contracts_for_notional(1.2, 60_000.0)
        .expect_err("上游要等交易所拒单才知道，这里本地就拦下");
    assert!(err.contains("最小下单量"), "{err}");
    assert!(err.contains("BTC-USDT-SWAP"), "错误要指名是哪个合约：{err}");

    // 非正的价格与名义额也拦下（NaN 也算）
    assert!(s.contracts_for_notional(1000.0, 0.0).is_err());
    assert!(s.contracts_for_notional(1000.0, f64::NAN).is_err());
    assert!(s.contracts_for_notional(0.0, 60_000.0).is_err());
    assert!(s.contracts_for_notional(f64::NAN, 60_000.0).is_err());
}

#[test]
fn format_size_avoids_float_noise() {
    let s = InstrumentSpec::from_json_str(&spec_json("0.01", "0.1", "0.1")).expect("ok");
    // 直接 {} 打 0.1 + 0.2 会出 0.30000000000000004，OKX 直接拒
    assert_eq!(s.format_size(0.1 + 0.2), "0.3");
    assert_eq!(s.format_size(1.6), "1.6");

    // 整数步长的合约只发整数
    let whole = InstrumentSpec::from_json_str(&spec_json("1", "1", "1")).expect("ok");
    assert_eq!(whole.size_decimals, 0);
    assert_eq!(whole.format_size(3.0), "3");

    // 细步长
    let fine = InstrumentSpec::from_json_str(&spec_json("0.001", "0.0001", "0.0001")).expect("ok");
    assert_eq!(fine.size_decimals, 4);
    assert_eq!(fine.format_size(0.0123), "0.0123");
}

#[test]
fn market_cap_is_checked_unlike_upstream() {
    let s = InstrumentSpec::from_json_str(&spec_json("0.01", "0.1", "0.1")).expect("ok");
    assert!(!s.exceeds_market_cap(1000.0));
    assert!(s.exceeds_market_cap(1000.1), "上游完全不看 maxMktSz");

    // 响应没给 maxMktSz 时一律判否
    let v = spec_json_without("maxMktSz");
    let no_cap = InstrumentSpec::from_json_str(&v).expect("ok");
    assert!(!no_cap.exceeds_market_cap(1e12));
}

#[test]
fn non_live_instrument_is_visible() {
    // 上游读了 state 却从不校验；这里至少能问出来
    let v = spec_json("0.01", "0.1", "0.1").replace("\"live\"", "\"suspend\"");
    assert!(!InstrumentSpec::from_json_str(&v).expect("ok").is_live());
}

// ---------------------------------------------------------------------------
// 客户端与再平衡：用假传输层把请求报文逐字段验一遍，不碰真实账户
// ---------------------------------------------------------------------------

use std::sync::{Arc, Mutex};

use phandas_rs::net::{HttpRequest, HttpResponse, HttpTransport};
use phandas_rs::trader::{
    Action, Confirm, MarginMode, OkxTrader, OrderRequest, OrderSide, PositionMode, Rebalancer,
};

/// 记录下来的一次请求。
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    url: String,
    body: String,
}

/// 假传输层：按「路径含某个子串」路由到预置响应，并记录全部请求。
#[derive(Clone)]
struct Mock {
    seen: Arc<Mutex<Vec<Seen>>>,
    #[allow(clippy::type_complexity)]
    route: Arc<dyn Fn(&str, &str, usize) -> String + Send + Sync>,
}

impl Mock {
    fn new(route: impl Fn(&str, &str, usize) -> String + Send + Sync + 'static) -> Mock {
        Mock {
            seen: Arc::new(Mutex::new(Vec::new())),
            route: Arc::new(route),
        }
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("锁未毒化").clone()
    }
    fn hits(&self, needle: &str) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|s| s.url.contains(needle))
            .collect()
    }
}

impl HttpTransport for Mock {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let nth = {
            let mut g = self.seen.lock().expect("锁未毒化");
            let nth = g.iter().filter(|s| s.url == req.url).count();
            g.push(Seen {
                method: req.method.as_str().to_string(),
                url: req.url.clone(),
                body: req.body.clone(),
            });
            nth
        };
        Ok(HttpResponse {
            status: 200,
            body: (self.route)(&req.url, &req.body, nth),
        })
    }
}

/// OKX 成功信封。
fn ok_env(data: &str) -> String {
    format!(r#"{{"code":"0","msg":"","data":[{data}]}}"#)
}

fn trader(mock: Mock) -> OkxTrader<Mock> {
    OkxTrader::with_transport(mock, creds(), Environment::Demo).pace(std::time::Duration::ZERO)
}

/// 账户配置：现货与合约 + 单向持仓。
const CFG_OK: &str = r#"{"acctLv":"2","posMode":"net_mode","uid":"42"}"#;

#[tokio::test]
async fn positions_api_failure_is_an_error_not_an_empty_book() {
    // 上游在 code != '0' 时返回空字典，于是「接口失败」与「确实空仓」不可区分——
    // 再平衡会当空仓、照目标全额建仓。这是本模块最要紧的一处修正。
    let mock = Mock::new(|_, _, _| r#"{"code":"50011","msg":"Rate limit","data":[]}"#.to_string());
    let err = trader(mock)
        .positions(None)
        .await
        .expect_err("API 失败必须硬失败");
    assert!(err.contains("50011") && err.contains("Rate limit"), "{err}");
}

#[tokio::test]
async fn positions_filters_dust_and_signs_the_query() {
    let data = concat!(
        // 有效多头
        r#"{"instId":"ETH-USDT-SWAP","posSide":"net","pos":"3","notionalUsd":"1200","markPx":"4000","avgPx":"3900","upl":"12","lever":"5"},"#,
        // 有效空头：notionalUsd 是绝对值，符号要按张数补
        r#"{"instId":"BTC-USDT-SWAP","posSide":"net","pos":"-2","notionalUsd":"1200","markPx":"60000","avgPx":"","upl":"-5","lever":"3"},"#,
        // 张数为 0 → 丢
        r#"{"instId":"SOL-USDT-SWAP","posSide":"net","pos":"0","notionalUsd":"10","markPx":"150"},"#,
        // 名义额小于 0.01 → 丢
        r#"{"instId":"OP-USDT-SWAP","posSide":"net","pos":"1","notionalUsd":"0.001","markPx":"2"}"#
    );
    let mock = Mock::new(move |_, _, _| ok_env(data));
    let ps = trader(mock.clone()).positions(None).await.expect("ok");

    assert_eq!(ps.len(), 2, "两条 dust 应被滤掉");
    // 按 inst_id 排序，可复现
    assert_eq!(ps[0].inst_id, "BTC-USDT-SWAP");
    assert_eq!(ps[0].notional_usd, -1200.0, "空头名义额取负");
    assert_eq!(
        ps[0].avg_px, 0.0,
        "空串 avgPx 按 0，上游裸 float() 会抛异常"
    );
    assert_eq!(ps[0].base_symbol(), "BTC");
    assert_eq!(ps[1].notional_usd, 1200.0);
    assert_eq!(ps[1].leverage, 5.0);

    // 请求带上了 instType，且路径与签名一致
    let hit = &mock.hits("positions")[0];
    assert_eq!(hit.method, "GET");
    assert!(
        hit.url.ends_with("/api/v5/account/positions?instType=SWAP"),
        "{}",
        hit.url
    );
    assert_eq!(hit.body, "");
}

#[tokio::test]
async fn batch_orders_chunk_at_twenty_and_keep_per_order_codes() {
    // 上游超 20 单直接返回 error，调用方当「全部失败」——21 个标的一单都发不出去。
    // 这里应切成 20 + 5 两批，且顶层 code = "2"（部分成功）时逐单 sCode 不能丢。
    let mock = Mock::new(|_, body, _| {
        let n = body.matches("\"instId\"").count();
        let items: Vec<String> = (0..n)
            .map(|i| {
                // 第 3 张单失败，其余成功
                if i == 2 {
                    r#"{"clOrdId":"c","ordId":"","sCode":"51008","sMsg":"Insufficient margin"}"#
                        .to_string()
                } else {
                    format!(r#"{{"clOrdId":"c{i}","ordId":"o{i}","sCode":"0","sMsg":""}}"#)
                }
            })
            .collect();
        // 顶层 code 用 "2" 表示部分成功
        format!(
            r#"{{"code":"2","msg":"partial","data":[{}]}}"#,
            items.join(",")
        )
    });
    let t = trader(mock.clone());
    let spec = InstrumentSpec::from_json_str(&spec_json("0.01", "0.1", "0.1")).expect("ok");
    let orders: Vec<OrderRequest> = (0..25)
        .map(|_| OrderRequest::market(&spec, OrderSide::Buy, 1.0, false))
        .collect();

    let acks = t
        .place_orders(&orders, PositionMode::Net)
        .await
        .expect("部分失败不该是整体 Err");
    assert_eq!(acks.len(), 25, "两批的结果都要收回来");
    let batches = mock.hits("batch-orders");
    assert_eq!(batches.len(), 2, "25 张切成 20 + 5");
    assert_eq!(batches[0].body.matches("\"instId\"").count(), 20);
    assert_eq!(batches[1].body.matches("\"instId\"").count(), 5);

    // 逐单 sCode 被保留下来
    assert!(acks[2].code == "51008" && !acks[2].is_success());
    assert_eq!(acks[2].msg, "Insufficient margin");
    assert!(acks[0].is_success() && acks[1].is_success());
}

#[tokio::test]
async fn batch_order_body_is_a_top_level_array_with_the_right_fields() {
    let mock = Mock::new(|_, _, _| ok_env(r#"{"clOrdId":"c","ordId":"o","sCode":"0","sMsg":""}"#));
    let t = trader(mock.clone());
    let spec = InstrumentSpec::from_json_str(&spec_json("0.01", "0.1", "0.1")).expect("ok");
    // 0.1 + 0.2 会有浮点噪声，必须被格式化成 "0.3"
    let o = OrderRequest::market(&spec, OrderSide::Sell, 0.1 + 0.2, true);
    t.place_orders(&[o], PositionMode::Net).await.expect("ok");

    let b = &mock.hits("batch-orders")[0];
    assert_eq!(b.method, "POST");
    // 顶层是数组——签名签的就是这份字节
    assert!(
        b.body.starts_with('[') && b.body.ends_with(']'),
        "{}",
        b.body
    );
    for expect in [
        r#""instId":"BTC-USDT-SWAP""#,
        r#""tdMode":"cross""#,
        r#""side":"sell""#,
        r#""ordType":"market""#,
        r#""sz":"0.3""#,
        r#""posSide":"net""#,
        r#""reduceOnly":"true""#,
        r#""tag":"82eebde453a2BCDE""#,
    ] {
        assert!(b.body.contains(expect), "缺 {expect}：{}", b.body);
    }
    assert!(
        b.body.contains(r#""clOrdId":"t"#),
        "要带 clOrdId：{}",
        b.body
    );
    // 双向持仓下 posSide 按方向填
    let mock2 = Mock::new(|_, _, _| ok_env(r#"{"clOrdId":"c","ordId":"o","sCode":"0","sMsg":""}"#));
    let t2 = trader(mock2.clone());
    let o2 = OrderRequest::market(&spec, OrderSide::Buy, 1.0, false);
    t2.place_orders(&[o2], PositionMode::LongShort)
        .await
        .expect("ok");
    assert!(mock2.hits("batch-orders")[0]
        .body
        .contains(r#""posSide":"long""#));
}

#[test]
fn action_close_is_symmetric_for_long_and_short() {
    // 上游 _determine_action 把「平多」判成 flip、「平空」判成 close，于是平多**不带**
    // reduceOnly 而平空带——不对称，且平多那笔有冲成反向仓位的风险。
    assert_eq!(Action::decide(500.0, 0.0), Action::Close, "平多");
    assert_eq!(Action::decide(-500.0, 0.0), Action::Close, "平空");
    assert!(Action::decide(500.0, 0.0).is_reduce_only());
    assert!(Action::decide(-500.0, 0.0).is_reduce_only());

    // 其余分支
    assert_eq!(Action::decide(0.0, 500.0), Action::Open);
    assert_eq!(Action::decide(0.0, -500.0), Action::Open);
    assert_eq!(Action::decide(500.0, 800.0), Action::Add);
    assert_eq!(Action::decide(-500.0, -800.0), Action::Add, "空头加仓");
    assert_eq!(Action::decide(800.0, 500.0), Action::Reduce);
    assert_eq!(Action::decide(-800.0, -500.0), Action::Reduce, "空头减仓");
    assert_eq!(Action::decide(500.0, -500.0), Action::Flip);

    // 小额跳过（上游那个 'none' 分支是死代码，先被这道闸门拦成 skip）
    assert_eq!(Action::decide(0.0, 0.0), Action::Skip);
    assert_eq!(Action::decide(100.0, 100.5), Action::Skip);
    assert!(!Action::decide(100.0, 100.5).is_reduce_only());
    // 正好 1 USD 就动
    assert_eq!(Action::decide(100.0, 101.0), Action::Add);

    // 只有减仓与平仓带 reduceOnly
    assert!(Action::decide(800.0, 500.0).is_reduce_only());
    assert!(!Action::decide(500.0, 800.0).is_reduce_only());
    assert!(!Action::decide(500.0, -500.0).is_reduce_only());
}

/// 建计划用的假服务端：一个持仓（ETH 多 1200）+ 两个合约规格与行情。
fn plan_mock() -> Mock {
    Mock::new(|url, _, _| {
        if url.contains("/account/config") {
            return ok_env(CFG_OK);
        }
        if url.contains("/account/positions") {
            return ok_env(
                r#"{"instId":"ETH-USDT-SWAP","posSide":"net","pos":"3","notionalUsd":"1200","markPx":"4000","avgPx":"3900","upl":"0","lever":"5"}"#,
            );
        }
        if url.contains("/account/instruments") {
            let id = if url.contains("BTC") {
                "BTC"
            } else if url.contains("SOL") {
                "SOL"
            } else {
                "ETH"
            };
            // BTC 一张 0.01 币，SOL / ETH 一张 1 币
            let ct = if id == "BTC" { "0.01" } else { "1" };
            return ok_env(&format!(
                r#"{{"instId":"{id}-USDT-SWAP","state":"live","ctVal":"{ct}","ctMult":"1","lotSz":"0.1","minSz":"0.1","tickSz":"0.1","maxMktSz":"10000"}}"#
            ));
        }
        if url.contains("/market/ticker") {
            let px = if url.contains("BTC") {
                "60000"
            } else if url.contains("SOL") {
                "150"
            } else {
                "4000"
            };
            let id = if url.contains("BTC") {
                "BTC"
            } else if url.contains("SOL") {
                "SOL"
            } else {
                "ETH"
            };
            return ok_env(&format!(
                r#"{{"instId":"{id}-USDT-SWAP","last":"{px}","bidPx":"{px}","askPx":"{px}"}}"#
            ));
        }
        if url.contains("set-leverage") || url.contains("batch-orders") {
            return ok_env(r#"{"clOrdId":"c","ordId":"o","sCode":"0","sMsg":""}"#);
        }
        ok_env("")
    })
}

#[tokio::test]
async fn plan_is_read_only_deterministic_and_closes_dropped_symbols() {
    let mock = plan_mock();
    let t = trader(mock.clone());
    // 目标里有 BTC 与 SOL，没有 ETH——ETH 当前持有 1200，应被平掉
    let plan = Rebalancer::new(
        [("BTC".to_string(), 0.5), ("SOL".to_string(), -0.3)],
        10_000.0,
    )
    .expect("ok")
    .plan(&t)
    .await
    .expect("建计划成功");

    // 建计划**绝不发单**
    assert!(
        mock.hits("batch-orders").is_empty() && mock.hits("set-leverage").is_empty(),
        "plan 阶段不该有任何写请求：{:?}",
        mock.seen().iter().map(|s| &s.url).collect::<Vec<_>>()
    );

    // 按标的名排序，可复现（上游迭代 set(...)，每个进程顺序都不同）
    let names: Vec<&str> = plan.legs.iter().map(|l| l.symbol.as_str()).collect();
    assert_eq!(names, ["BTC", "ETH", "SOL"]);

    let btc = &plan.legs[0];
    assert_eq!(btc.action, Action::Open);
    assert_eq!(btc.target_usd, 5000.0);
    assert_eq!(btc.current_usd, 0.0);
    // 单张 60000 × 0.01 = 600 USD；5000 / 600 = 8.33 张 → 向下对齐到 8.3 张 = 4980 USD
    assert_eq!(btc.order.as_ref().expect("有单").size, "8.3");
    assert!(
        (btc.order_usd - 4980.0).abs() < 1e-6,
        "得 {}",
        btc.order_usd
    );
    assert!(!btc.order.as_ref().expect("有单").reduce_only);

    // 持有但不在目标里 → 平仓，且带 reduceOnly（上游平多这一支不带）
    let eth = &plan.legs[1];
    assert_eq!(eth.action, Action::Close);
    assert_eq!(eth.target_usd, 0.0);
    assert_eq!(eth.current_usd, 1200.0);
    let o = eth.order.as_ref().expect("有单");
    assert!(o.reduce_only, "平多必须带 reduceOnly");
    assert_eq!(o.side, OrderSide::Sell);

    // 负权重 → 做空
    let sol = &plan.legs[2];
    assert_eq!(sol.target_usd, -3000.0);
    assert_eq!(sol.order.as_ref().expect("有单").side, OrderSide::Sell);

    // 毛敞口与预算的比：0.5 + 0.3 = 0.8
    assert!((plan.gross_leverage() - 0.8).abs() < 1e-9);
    assert_eq!(plan.pos_mode, PositionMode::Net);
}

#[tokio::test]
async fn plan_rejects_below_min_size_locally_with_a_note() {
    let mock = plan_mock();
    let t = trader(mock);
    // BTC 目标 30 USD，单张 600 USD → 0.05 张，按 lotSz 0.1 向下取整成 0，低于 minSz 0.1
    let plan = Rebalancer::new([("BTC".to_string(), 0.003)], 10_000.0)
        .expect("ok")
        .plan(&t)
        .await
        .expect("ok");
    let btc = plan
        .legs
        .iter()
        .find(|l| l.symbol == "BTC")
        .expect("有 BTC");
    assert_eq!(btc.action, Action::Open, "动作还是开仓");
    assert!(btc.order.is_none(), "但本地就拦下了，不发单");
    let note = btc.note.as_ref().expect("要说明原因");
    assert!(note.contains("最小下单量"), "{note}");
    // 预览里能看到这条备注
    assert!(plan.preview().contains("最小下单量"));
}

#[tokio::test]
async fn execute_sets_leverage_then_sends_and_needs_explicit_confirm() {
    let mock = plan_mock();
    let t = trader(mock.clone());
    let plan = Rebalancer::new([("BTC".to_string(), 0.5)], 10_000.0)
        .expect("ok")
        .leverage(3)
        .plan(&t)
        .await
        .expect("ok");

    // Confirm::Yes 是唯一变体，调用点必须写出来
    let report = plan.execute(&t, Confirm::Yes).await.expect("发单成功");
    assert_eq!(report.acks.len(), 1);
    assert!(report.all_ok() && report.failed() == 0);
    assert!(report.leverage_errors.is_empty());
    assert!(report.summary().contains("ok=1"));

    // 先设杠杆再下单
    let urls: Vec<String> = mock.seen().into_iter().map(|s| s.url).collect();
    let lev = urls
        .iter()
        .position(|u| u.contains("set-leverage"))
        .expect("设过杠杆");
    let ord = urls
        .iter()
        .position(|u| u.contains("batch-orders"))
        .expect("下过单");
    assert!(lev < ord, "顺序应是先设杠杆再下单：{urls:?}");
    let lb = &mock.hits("set-leverage")[0].body;
    assert!(
        lb.contains(r#""lever":"3""#) && lb.contains(r#""mgnMode":"cross""#),
        "{lb}"
    );
    // 全仓不该带 posSide
    assert!(!lb.contains("posSide"), "{lb}");
}

#[tokio::test]
async fn execute_on_an_empty_plan_sends_nothing() {
    let mock = plan_mock();
    let t = trader(mock.clone());
    // 目标与当前一致（ETH 1200 / 10000 = 0.12），差额为 0 → 全部 skip
    let plan = Rebalancer::new([("ETH".to_string(), 0.12)], 10_000.0)
        .expect("ok")
        .plan(&t)
        .await
        .expect("ok");
    assert!(plan.orders().is_empty(), "{}", plan.preview());

    let before = mock.seen().len();
    let report = plan.execute(&t, Confirm::Yes).await.expect("ok");
    assert!(report.acks.is_empty());
    assert_eq!(mock.seen().len(), before, "没单要发就一个请求都不打");
}

#[tokio::test]
async fn plan_refuses_wrong_account_mode() {
    // 账户模式 1（纯现货）+ 双向持仓，两条都不满足
    let mock = Mock::new(|url, _, _| {
        if url.contains("/account/config") {
            ok_env(r#"{"acctLv":"1","posMode":"long_short_mode","uid":"1"}"#)
        } else {
            ok_env("")
        }
    });
    let t = trader(mock.clone());
    let err = Rebalancer::new([("BTC".to_string(), 0.5)], 1000.0)
        .expect("ok")
        .plan(&t)
        .await
        .expect_err("配置不合规应报错");
    assert!(err.contains("acctLv"), "{err}");
    assert!(err.contains("posMode"), "{err}");
    // 校验不通过就不该继续读持仓
    assert!(mock.hits("positions").is_empty(), "{:?}", mock.seen());
}

#[tokio::test]
async fn validate_does_not_touch_account_settings_unless_asked() {
    let mock = Mock::new(|url, _, _| {
        if url.contains("/account/config") {
            ok_env(r#"{"acctLv":"2","posMode":"long_short_mode","uid":"1"}"#)
        } else {
            ok_env("")
        }
    });
    let t = trader(mock.clone());
    // auto_fix = false：只报问题，不改账户
    assert!(t.validate_account_config(false).await.is_err());
    assert!(
        mock.hits("set-position-mode").is_empty(),
        "上游 auto_fix 默认 True，跑一次再平衡的副作用是改账户设置；这里默认不改"
    );

    // 显式要求才改
    let mock2 = Mock::new(|url, _, nth| {
        if url.contains("set-position-mode") {
            return ok_env("");
        }
        if url.contains("/account/config") {
            // 第一次读到双向，改完之后读到单向
            return if nth == 0 {
                ok_env(r#"{"acctLv":"2","posMode":"long_short_mode","uid":"1"}"#)
            } else {
                ok_env(CFG_OK)
            };
        }
        ok_env("")
    });
    let t2 = trader(mock2.clone());
    t2.validate_account_config(true).await.expect("自动修好");
    let body = &mock2.hits("set-position-mode")[0].body;
    assert!(body.contains(r#""posMode":"net_mode""#), "{body}");
}

#[test]
fn rebalancer_validates_inputs() {
    assert!(
        Rebalancer::new(Vec::<(String, f64)>::new(), 1000.0).is_err(),
        "空权重表"
    );
    assert!(
        Rebalancer::new([("BTC".to_string(), 0.5)], 0.0).is_err(),
        "预算非正"
    );
    assert!(Rebalancer::new([("BTC".to_string(), 0.5)], -1.0).is_err());
    assert!(
        Rebalancer::new([("BTC".to_string(), f64::NAN)], 1000.0).is_err(),
        "NaN 权重"
    );
    assert!(Rebalancer::new([("BTC".to_string(), 0.5)], 1000.0).is_ok());
}

#[tokio::test]
async fn preview_flags_gross_exposure_above_budget() {
    let mock = plan_mock();
    let t = trader(mock);
    // 权重毛和 1.5 倍预算：上游对此完全不校验，这里不拦但要在文本里标出来
    let plan = Rebalancer::new(
        [("BTC".to_string(), 1.0), ("SOL".to_string(), -0.5)],
        10_000.0,
    )
    .expect("ok")
    .plan(&t)
    .await
    .expect("ok");
    let text = plan.preview();
    assert!((plan.gross_leverage() - 1.5).abs() < 1e-9);
    assert!(text.contains("已超过 1 倍"), "{text}");
    assert!(
        text.contains("Symbol") && text.contains("OrderUSD"),
        "{text}"
    );
}

#[test]
fn margin_mode_and_side_strings_match_okx() {
    assert_eq!(MarginMode::Cross.as_str(), "cross");
    assert_eq!(MarginMode::Isolated.as_str(), "isolated");
    assert_eq!(MarginMode::default(), MarginMode::Cross);
    assert_eq!(OrderSide::Buy.as_str(), "buy");
    assert_eq!(OrderSide::from_delta(5.0), OrderSide::Buy);
    assert_eq!(OrderSide::from_delta(-5.0), OrderSide::Sell);
    assert_eq!(PositionMode::parse("net_mode"), Some(PositionMode::Net));
    assert_eq!(PositionMode::parse("nonsense"), None);
}
