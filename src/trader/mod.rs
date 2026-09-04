//! 实盘下单模块：Python 版 phandas 中 `trader.py` 的 Rust 实现。
//!
//! 只在开启 `trader` feature 时编译。**不用 `python-okx`、不用 `rich`**，直接打 OKX v5 REST。
//!
//! # 文件划分
//!
//! | 文件 | 内容 |
//! |---|---|
//! | `auth.rs` | v5 签名、[`Credentials`]、[`Environment`] |
//! | `contract.rs` | [`InstrumentSpec`]：合约规格与「名义额 ↔ 张数」换算 |
//! | `client.rs` | [`OkxTrader`]：签名请求、九个端点、批量切分 |
//! | `rebalance.rs` | [`Rebalancer`] / [`RebalancePlan`] / [`RebalanceReport`]：权重 → 市价单 |
//! | `types.rs` | 响应里的数据结构：[`AccountConfig`] / [`Position`] / [`Ticker`] / [`Balance`] / [`OrderRequest`] / [`OrderAck`] |
//! | `json.rs` | OKX「数值也是字符串、空串表示不适用」的字段提取（私有） |
//!
//! # 安全设计
//!
//! 这个模块会动真钱，故几处刻意做得比上游严：
//!
//! - **[`Environment`] 没有 `Default`**，必须把 `Live` 显式写出来。代码里「不小心打到实盘」
//!   不是一个能默默发生的状态。
//! - **[`Credentials`] 的 `Debug` 打码**：`secret_key` / `passphrase` 完全不输出，`api_key`
//!   只留前 4 字符。整个客户端 `{:?}` 进日志也不泄露。
//! - **凭证可从环境变量读**（[`Credentials::from_env`]），配合
//!   [`crate::net::load_default_dotenv`] 放进 `.env`（已在 `.gitignore` 里）。上游只能从构造
//!   参数传，写脚本时容易硬编码进源码。
//! - **取整方向向下**：见 [`contract`] 的模块文档，上游把方向交给了一个没写清的服务端默认值。
//!
//! # 与上游的口径差异
//!
//! - `get_positions` 在 `code != '0'` 时上游返回空字典，于是「API 失败」与「无持仓」不可区分，
//!   再平衡会把它当空仓、照目标全额建仓。本仓库返回 `Err`。
//! - `minSz` 上游从不校验；本仓库在本地拦下并在计划里标注原因。
//! - `state` 上游读了不用；本仓库在建计划时会看是否 `live`。

pub mod auth;
pub mod client;
pub mod contract;
mod json;
pub mod rebalance;
pub mod types;

pub use self::auth::{sign, signed_request, signed_request_at, Credentials, Environment, BASE_URL};
pub use self::client::{OkxTrader, BATCH_LIMIT};
pub use self::contract::InstrumentSpec;
pub use self::rebalance::{
    rebalance, Action, Confirm, Leg, RebalancePlan, RebalanceReport, Rebalancer, DEFAULT_SUFFIX,
    MIN_TRADE_VALUE,
};
pub use self::types::{
    AccountConfig, Balance, MarginMode, OrderAck, OrderRequest, OrderSide, Position, PositionMode,
    Ticker, MIN_NOTIONAL_USD,
};
