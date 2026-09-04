//! `OkxTrader`：OKX v5 REST 客户端，对应上游 `trader.py` 的 `OKXTrader`。
//!
//! 端点一览（全部 `/api/v5` 之下）：
//!
//! | 方法 | 端点 |
//! |---|---|
//! | [`OkxTrader::account_config`] | `GET /account/config` |
//! | [`OkxTrader::set_position_mode`] | `POST /account/set-position-mode` |
//! | [`OkxTrader::positions`] | `GET /account/positions` |
//! | [`OkxTrader::ticker`] | `GET /market/ticker` |
//! | [`OkxTrader::instrument`] | `GET /account/instruments` |
//! | [`OkxTrader::set_leverage`] | `POST /account/set-leverage` |
//! | [`OkxTrader::balance`] | `GET /account/balance` |
//! | [`OkxTrader::place_orders`] | `POST /trade/batch-orders` |
//! | [`OkxTrader::close_position`] | `POST /trade/close-position` |
//!
//! 上游的 `convert_coin_contract`（`GET /public/convert-contract-coin`）不移植——换算改在本地
//! 做，见 [`super::contract`]。上游的 `place_order` 单发接口也不移植：批量接口能发一张，
//! 少一条几乎重复的代码路径。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::net::{HttpTransport, Method, ReqwestTransport};

use super::auth::{signed_request, Credentials, Environment, BASE_URL};
use super::contract::InstrumentSpec;
use super::json::s_of;
use super::types::{
    AccountConfig, Balance, MarginMode, OrderAck, OrderRequest, Position, PositionMode, Ticker,
};

/// `POST /trade/batch-orders` 单次最多带多少张单。
///
/// 上游超了就返回 `{'status':'error','msg':'Too many orders'}`，而调用方拿这个 dict 当「全部
/// 失败」处理——于是 **21 个标的一单都发不出去**。本仓库按这个上限切批。
pub const BATCH_LIMIT: usize = 20;

/// 每次请求之间的默认节流。对应上游 `time.sleep(0.25)`。
const DEFAULT_PACE: Duration = Duration::from_millis(250);

/// 券商返佣标记，上游每张新单都带。原样保留。
const BROKER_TAG: &str = "82eebde453a2BCDE";

/// 生成客户端自定义单号，形如 `t1725442500000a3f9c1`。
///
/// - 入参：无。
/// - 加工：`t` + 13 位毫秒 + 6 位 base36（由纳秒与一个进程内自增序号混出来）。
/// - 出参：长度 20 的字符串，字符集是字母数字，满足 OKX 对 `clOrdId` 的要求。
///
/// 用序号而不是随机数是为了不引 `rand`；混进纳秒则是为了跨进程也基本不撞。
/// 注意**每次调用都是新号**，所以重发一张单不会被交易所去重——上游同样如此，
/// 真要幂等得由调用方自己记账。
fn client_order_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut mixed = (now.subsec_nanos() as u64) ^ (seq << 20).wrapping_add(seq);
    let mut tail = String::with_capacity(6);
    for _ in 0..6 {
        let d = (mixed % 36) as u32;
        mixed /= 36;
        tail.push(char::from_digit(d, 36).unwrap_or('0'));
    }
    format!("t{}{tail}", now.as_millis())
}

/// 把键值对拼成 JSON 对象文本。
///
/// - 入参：`fields` 有序的键值对，值已是最终字符串。
/// - 加工：按插入顺序拼 `{"k":"v",...}`，并转义值里的 `"` 与 `\`。
/// - 出参：JSON 文本。**签名签的就是这份字节**，所以不能再序列化一遍（见 [`super::auth`]）。
fn json_object(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(k, v)| {
            let esc = v.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{k}\":\"{esc}\"")
        })
        .collect();
    format!("{{{}}}", body.join(","))
}

/// OKX v5 客户端。
///
/// 对传输层泛型，默认用 [`ReqwestTransport`]；测试里换成返回固定 JSON 的假实现，就能把
/// 「请求报文长什么样」逐字段验一遍而不碰真实账户。
#[derive(Debug, Clone)]
pub struct OkxTrader<T: HttpTransport = ReqwestTransport> {
    transport: T,
    creds: Credentials,
    env: Environment,
    base_url: String,
    inst_type: String,
    pace: Duration,
}

impl OkxTrader<ReqwestTransport> {
    /// 用内置的 reqwest 传输层构造。
    ///
    /// - 入参：`creds` 凭证；`env` 运行环境——**必须显式给**，没有默认值。
    /// - 加工：建一个带 30 秒超时的 [`ReqwestTransport`]。
    /// - 出参：`Ok(OkxTrader)`；TLS 后端初始化失败时返回 `Err`。
    pub fn new(
        creds: Credentials,
        env: Environment,
    ) -> Result<OkxTrader<ReqwestTransport>, String> {
        Ok(OkxTrader::with_transport(
            ReqwestTransport::new()?,
            creds,
            env,
        ))
    }

    /// 从环境变量读凭证再构造，变量名见 [`super::auth::ENV_KEYS`]。
    ///
    /// - 入参：`env` 运行环境。
    /// - 加工：[`Credentials::from_env`] 之后转发给 [`OkxTrader::new`]。
    /// - 出参：`Ok(OkxTrader)`；缺环境变量时返回指名缺哪个的 `Err`。
    pub fn from_env(env: Environment) -> Result<OkxTrader<ReqwestTransport>, String> {
        OkxTrader::new(Credentials::from_env()?, env)
    }
}

impl<T: HttpTransport> OkxTrader<T> {
    /// 用调用方自备的传输层构造。
    ///
    /// - 入参：`transport` 任意 [`HttpTransport`] 实现；`creds` 凭证；`env` 运行环境。
    /// - 加工：套上默认根地址、`SWAP` 合约类型与默认节流。
    /// - 出参：[`OkxTrader`]。
    pub fn with_transport(transport: T, creds: Credentials, env: Environment) -> OkxTrader<T> {
        OkxTrader {
            transport,
            creds,
            env,
            base_url: BASE_URL.to_string(),
            inst_type: "SWAP".to_string(),
            pace: DEFAULT_PACE,
        }
    }

    /// 换 API 根地址并返回自身。测试指向假地址、或需要走镜像时用。
    pub fn base_url(mut self, url: impl Into<String>) -> OkxTrader<T> {
        self.base_url = url.into();
        self
    }

    /// 换合约类型并返回自身。默认 `SWAP`（永续），对应上游 `inst_type='SWAP'`。
    pub fn inst_type(mut self, inst_type: impl Into<String>) -> OkxTrader<T> {
        self.inst_type = inst_type.into();
        self
    }

    /// 换请求之间的节流间隔并返回自身；传 [`Duration::ZERO`] 即不等待（测试用）。
    pub fn pace(mut self, pace: Duration) -> OkxTrader<T> {
        self.pace = pace;
        self
    }

    /// 当前运行环境。实盘时调用方可以据此在日志里加重警示。
    pub fn environment(&self) -> Environment {
        self.env
    }

    /// 发一个签名请求，返回 `(code, msg, data)` 三件套，**不判 `code`**。
    ///
    /// - 入参：`method` 方法；`request_path` 含 query string 的路径；`body` 请求体。
    /// - 加工：签名组装 → 传输 → 解析 OKX 的信封 `{"code":..,"msg":..,"data":[..]}`。
    /// - 出参：`Ok((code, msg, data))`；传输失败或响应不是合法 JSON 时返回 `Err`。
    ///
    /// 批量下单要用这条而不是 [`OkxTrader::call`]：部分成功时顶层 `code` 是 `"2"`，
    /// 但 `data[]` 里逐单的 `sCode` 才是真结果。上游只判顶层，于是这些细节全丢了。
    async fn raw(
        &self,
        method: Method,
        request_path: &str,
        body: &str,
    ) -> Result<(String, String, Vec<Value>), String> {
        let req = signed_request(
            &self.creds,
            self.env,
            &self.base_url,
            method,
            request_path,
            body,
        );
        let resp = self.transport.send(req).await?;
        let v: Value = serde_json::from_str(&resp.body).map_err(|e| {
            format!(
                "解析 {request_path} 的响应失败：{e}；HTTP {}；原文前 200 字节：{:.200}",
                resp.status, resp.body
            )
        })?;
        let data = v
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok((
            s_of(&v, "code").to_string(),
            s_of(&v, "msg").to_string(),
            data,
        ))
    }

    /// 发一个签名请求并要求顶层 `code == "0"`。
    ///
    /// - 入参：同 [`OkxTrader::raw`]。
    /// - 加工：走 `raw` 后校验 `code`。
    /// - 出参：`Ok(data 数组)`；`code` 非 `"0"` 时返回带 code 与 msg 的 `Err`。
    async fn call(
        &self,
        method: Method,
        request_path: &str,
        body: &str,
    ) -> Result<Vec<Value>, String> {
        let (code, msg, data) = self.raw(method, request_path, body).await?;
        if code != "0" {
            return Err(format!("OKX 返回错误 {code}：{msg}（{request_path}）"));
        }
        Ok(data)
    }

    /// 按 `pace` 节流。
    async fn throttle(&self) {
        if !self.pace.is_zero() {
            tokio::time::sleep(self.pace).await;
        }
    }

    /// 读账户配置。`GET /api/v5/account/config`
    ///
    /// - 入参：无。
    /// - 加工：取 `data[0]` 解析成 [`AccountConfig`]。
    /// - 出参：`Ok(AccountConfig)`；`data` 为空时返回 `Err`。
    pub async fn account_config(&self) -> Result<AccountConfig, String> {
        let data = self.call(Method::Get, "/api/v5/account/config", "").await?;
        let first = data
            .first()
            .ok_or_else(|| "account/config 没有返回任何数据".to_string())?;
        Ok(AccountConfig::from_json(first))
    }

    /// 校验账户配置能不能跑再平衡。
    ///
    /// - 入参：`auto_fix` 为 `true` 时，持仓模式不是单向就**自动改成 `net_mode`**。
    /// - 加工：读配置 → 需要时改持仓模式并重读 → 走 [`AccountConfig::check`]。
    /// - 出参：`Ok(AccountConfig)` 表示两条前置条件都满足；否则 `Err` 里逐条列出问题。
    ///
    /// **`auto_fix` 默认应传 `false`。** 上游这个参数默认 `True`，而 `rebalance_portfolio`
    /// 一进来就调它——等于「跑一次再平衡」的副作用是**改账户设置**。本仓库把方向反过来：
    /// 要改就显式传 `true`。
    pub async fn validate_account_config(&self, auto_fix: bool) -> Result<AccountConfig, String> {
        let mut cfg = self.account_config().await?;
        if auto_fix && !cfg.is_net_mode() {
            self.set_position_mode(PositionMode::Net).await?;
            cfg = self.account_config().await?;
        }
        let bad = cfg.check();
        if !bad.is_empty() {
            return Err(format!("账户配置不满足再平衡要求：{}", bad.join("；")));
        }
        Ok(cfg)
    }

    /// 设持仓模式。`POST /api/v5/account/set-position-mode`
    ///
    /// - 入参：`mode` 目标持仓模式。
    /// - 加工：`{"posMode":"net_mode"}` 发出去。
    /// - 出参：`Ok(())`；OKX 拒绝时返回 `Err`（有持仓时改不了，这是交易所的限制）。
    pub async fn set_position_mode(&self, mode: PositionMode) -> Result<(), String> {
        let body = json_object(&[("posMode", mode.as_str().to_string())]);
        self.call(Method::Post, "/api/v5/account/set-position-mode", &body)
            .await?;
        Ok(())
    }

    /// 读持仓。`GET /api/v5/account/positions`
    ///
    /// - 入参：`inst_id` 只看某个合约时给 `Some`，看全部给 `None`。
    /// - 加工：带上 `instType` 查询 → 逐项过 `Position` 的空仓闸门（张数 / 名义额 / 标记价为 0，或名义额不足 0.01 的一律丢掉）。
    /// - 出参：`Ok(持仓列表)`，按 `inst_id` 排序（可复现）；**API 失败返回 `Err`**。
    ///
    /// 上游在 `code != '0'` 时返回空字典，于是「接口失败」和「确实没有持仓」不可区分——
    /// 再平衡会把它当空仓、照目标**全额建仓**。这是本模块最要紧的一处修正。
    pub async fn positions(&self, inst_id: Option<&str>) -> Result<Vec<Position>, String> {
        let mut path = format!("/api/v5/account/positions?instType={}", self.inst_type);
        if let Some(id) = inst_id {
            path.push_str(&format!("&instId={id}"));
        }
        let data = self.call(Method::Get, &path, "").await?;
        let mut out: Vec<Position> = data.iter().filter_map(Position::from_json).collect();
        out.sort_by(|a, b| a.inst_id.cmp(&b.inst_id));
        Ok(out)
    }

    /// 读行情。`GET /api/v5/market/ticker`
    ///
    /// - 入参：`inst_id` 合约 id。
    /// - 加工：取 `data[0]`。
    /// - 出参：`Ok(Ticker)`；`data` 为空时返回 `Err`。这是个公开端点，但照上游一样带签名发
    ///   （多带几个头不影响结果）。
    pub async fn ticker(&self, inst_id: &str) -> Result<Ticker, String> {
        let path = format!("/api/v5/market/ticker?instId={inst_id}");
        let data = self.call(Method::Get, &path, "").await?;
        let first = data
            .first()
            .ok_or_else(|| format!("{inst_id} 没有行情数据"))?;
        Ok(Ticker::from_json(first))
    }

    /// 读合约规格。`GET /api/v5/account/instruments`
    ///
    /// - 入参：`inst_id` 合约 id。
    /// - 加工：取 `data[0]` 解析成 [`InstrumentSpec`]。
    /// - 出参：`Ok(InstrumentSpec)`；`data` 为空或必需字段缺失时返回 `Err`。
    ///
    /// 上游有这个方法（`get_instrument_info`）但**从不调用**——它把张数换算整个甩给了服务端。
    /// 本仓库靠它做本地换算与 `minSz` 拦截。
    pub async fn instrument(&self, inst_id: &str) -> Result<InstrumentSpec, String> {
        let path = format!(
            "/api/v5/account/instruments?instType={}&instId={inst_id}",
            self.inst_type
        );
        let data = self.call(Method::Get, &path, "").await?;
        let first = data
            .first()
            .ok_or_else(|| format!("{inst_id} 查不到合约规格"))?;
        InstrumentSpec::from_json(first)
    }

    /// 读账户权益。`GET /api/v5/account/balance`
    ///
    /// - 入参：无。
    /// - 加工：取 `data[0]`。
    /// - 出参：`Ok(Balance)`；`data` 为空时返回 `Err`。`total_equity` 就是再平衡常用的预算基数。
    pub async fn balance(&self) -> Result<Balance, String> {
        let data = self
            .call(Method::Get, "/api/v5/account/balance", "")
            .await?;
        let first = data
            .first()
            .ok_or_else(|| "account/balance 没有返回任何数据".to_string())?;
        Ok(Balance::from_json(first))
    }

    /// 设杠杆。`POST /api/v5/account/set-leverage`
    ///
    /// - 入参：`inst_id` 合约；`lever` 杠杆倍数（1–125）；`mgn_mode` 保证金模式；
    ///   `pos_side` 仅逐仓 + 双向持仓时才需要，其余传 `None`。
    /// - 加工：先本地校验倍数范围（上游同样先本地校验），再发 `{instId, lever, mgnMode}`。
    /// - 出参：`Ok(())`；倍数越界或 OKX 拒绝时返回 `Err`。
    pub async fn set_leverage(
        &self,
        inst_id: &str,
        lever: u32,
        mgn_mode: MarginMode,
        pos_side: Option<&str>,
    ) -> Result<(), String> {
        if !(1..=125).contains(&lever) {
            return Err(format!("杠杆倍数须落在 1–125，给的是 {lever}"));
        }
        let mut fields = vec![
            ("instId", inst_id.to_string()),
            ("lever", lever.to_string()),
            ("mgnMode", mgn_mode.as_str().to_string()),
        ];
        // 只有逐仓 + 双向持仓才要 posSide；全仓带上会被拒
        if mgn_mode == MarginMode::Isolated {
            if let Some(side) = pos_side {
                fields.push(("posSide", side.to_string()));
            }
        }
        self.call(
            Method::Post,
            "/api/v5/account/set-leverage",
            &json_object(&fields),
        )
        .await?;
        Ok(())
    }

    /// 批量下市价单。`POST /api/v5/trade/batch-orders`
    ///
    /// - 入参：`orders` 待发的单；`pos_mode` 账户持仓模式（决定 `posSide` 填 `net` 还是
    ///   `long` / `short`）。
    /// - 加工：**按 [`BATCH_LIMIT`] 切批**，逐批发出、批之间按 `pace` 节流；
    ///   每张单带一个新的 `clOrdId` 与券商标记；用内部的原始调用取回结果，
    ///   **只要有 `data[]` 就逐单解析**，不管顶层 `code` 是 `0` / `1` / `2`。
    /// - 出参：`Ok(逐单结果)`，顺序与入参一致——调用方据 [`OrderAck::is_success`] 逐单判成败。
    ///   只有整批连 `data[]` 都没有（例如签名被拒）才返回 `Err`。
    ///
    /// 两处修了上游：**超 20 单不再是「一单都不发却报全部失败」**，而是切批；
    /// **部分成功不再丢掉逐单的 `sCode` / `sMsg`**。
    pub async fn place_orders(
        &self,
        orders: &[OrderRequest],
        pos_mode: PositionMode,
    ) -> Result<Vec<OrderAck>, String> {
        if orders.is_empty() {
            return Ok(Vec::new());
        }
        let mut acks = Vec::with_capacity(orders.len());
        for (i, chunk) in orders.chunks(BATCH_LIMIT).enumerate() {
            if i > 0 {
                self.throttle().await;
            }
            let items: Vec<String> = chunk
                .iter()
                .map(|o| {
                    let pos_side = match pos_mode {
                        PositionMode::Net => "net".to_string(),
                        // 双向持仓：买入开多、卖出开空
                        PositionMode::LongShort => match o.side {
                            super::types::OrderSide::Buy => "long".to_string(),
                            super::types::OrderSide::Sell => "short".to_string(),
                        },
                    };
                    json_object(&[
                        ("instId", o.inst_id.clone()),
                        ("tdMode", MarginMode::Cross.as_str().to_string()),
                        ("side", o.side.as_str().to_string()),
                        ("ordType", "market".to_string()),
                        ("sz", o.size.clone()),
                        ("posSide", pos_side),
                        ("reduceOnly", o.reduce_only.to_string()),
                        ("clOrdId", client_order_id()),
                        ("tag", BROKER_TAG.to_string()),
                    ])
                })
                .collect();
            // body 是**顶层 JSON 数组**，签名签的就是这份字节
            let body = format!("[{}]", items.join(","));
            let (code, msg, data) = self
                .raw(Method::Post, "/api/v5/trade/batch-orders", &body)
                .await?;
            if data.is_empty() {
                return Err(format!(
                    "批量下单整批被拒：OKX {code}：{msg}（{} 张单）",
                    chunk.len()
                ));
            }
            acks.extend(data.iter().map(OrderAck::from_json));
        }
        Ok(acks)
    }

    /// 市价全平一个合约的仓位。`POST /api/v5/trade/close-position`
    ///
    /// - 入参：`inst_id` 合约；`mgn_mode` 保证金模式；`pos_side` 双向持仓时给 `Some`。
    /// - 加工：发 `{instId, mgnMode}`（外加可选的 `posSide`）。
    /// - 出参：`Ok(())`；OKX 拒绝时返回 `Err`。
    ///
    /// 这条不带 `clOrdId` 也不带券商标记，与上游一致。
    pub async fn close_position(
        &self,
        inst_id: &str,
        mgn_mode: MarginMode,
        pos_side: Option<&str>,
    ) -> Result<(), String> {
        let mut fields = vec![
            ("instId", inst_id.to_string()),
            ("mgnMode", mgn_mode.as_str().to_string()),
        ];
        if let Some(side) = pos_side {
            fields.push(("posSide", side.to_string()));
        }
        self.call(
            Method::Post,
            "/api/v5/trade/close-position",
            &json_object(&fields),
        )
        .await?;
        Ok(())
    }

    /// 市价全平所有仓位。
    ///
    /// - 入参：`mgn_mode` 保证金模式；`suffix` 只平合约 id 以此结尾的仓位
    ///   （默认场景传 `Some("-USDT-SWAP")`），传 `None` 表示不筛。
    /// - 加工：读持仓 → 读账户配置定 `posSide` → **按 `inst_id` 排序**后逐个平，
    ///   每个之间按 `pace` 节流。
    /// - 出参：`Ok(每个合约的结果)`——成功给 `Ok(())`，失败给 `Err(原因)`，
    ///   **一个失败不中断其余**。整体读持仓失败才返回外层 `Err`。
    #[allow(clippy::type_complexity)]
    pub async fn close_all_positions(
        &self,
        mgn_mode: MarginMode,
        suffix: Option<&str>,
    ) -> Result<Vec<(String, Result<(), String>)>, String> {
        let positions = self.positions(None).await?;
        let cfg = self.account_config().await?;
        let mut out = Vec::new();
        for (i, p) in positions
            .iter()
            .filter(|p| suffix.is_none_or(|s| p.inst_id.ends_with(s)))
            .enumerate()
        {
            if i > 0 {
                self.throttle().await;
            }
            // 单向持仓不传 posSide；双向持仓才照持仓自身的方向传
            let side = if cfg.is_net_mode() {
                None
            } else {
                Some(p.pos_side.as_str())
            };
            out.push((
                p.inst_id.clone(),
                self.close_position(&p.inst_id, mgn_mode, side).await,
            ));
        }
        Ok(out)
    }

    /// 一次性把要下单的合约规格与最新价都取回来。
    ///
    /// - 入参：`inst_ids` 合约 id 列表。
    /// - 加工：逐个取规格与行情，请求之间按 `pace` 节流。
    /// - 出参：`Ok(合约 id → (规格, 最新价))`；任一个失败即整体 `Err`——建计划阶段就该硬失败，
    ///   总比拿着半份数据去下单好。
    pub async fn specs_and_prices(
        &self,
        inst_ids: &[String],
    ) -> Result<BTreeMap<String, (InstrumentSpec, f64)>, String> {
        let mut out = BTreeMap::new();
        for (i, id) in inst_ids.iter().enumerate() {
            if i > 0 {
                self.throttle().await;
            }
            let spec = self.instrument(id).await?;
            let px = self.ticker(id).await?.last;
            out.insert(id.clone(), (spec, px));
        }
        Ok(out)
    }
}
