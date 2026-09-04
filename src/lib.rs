//! phandas_rs：Python 版 phandas 的 Rust 移植。
//!
//! - [`factor`]：因子构造。`Factor` / `Panel` 与 70 多个横截面、时序、分组运算子，
//!   对应上游 `core.py` / `operators.py` / `panel.py` / `constants.py`。
//! - [`backtest`]：事件驱动回测。目标市值、手续费、净值与 11 项绩效指标，
//!   对应上游 `backtest.py`。
//! - [`analysis`]：因子评价。横截面 IC、相关矩阵与覆盖率 / 换手率 / 自相关等描述统计，
//!   对应上游 `analysis.py`。
//!
//! 以上三个模块**零外部依赖**：分位数反函数与 CDF、最小二乘、条件数估计、CSV 解析、
//! 日期运算、相关系数都在仓库内实现。
//!
//! # 可选的联网模块
//!
//! 抓行情与实盘下单绕不开 HTTP 与 TLS，故放在可选 feature 后面——不开就不引入任何依赖：
//!
//! | feature | 模块 | 对应上游 | 说明 |
//! |---|---|---|---|
//! | `data` | `data` | `data.py` | Binance 行情抓取，直接打 REST API，不用 ccxt |
//! | `trader` | `trader` | `trader.py` | OKX 永续下单，直接打 v5 REST API，不用 python-okx |
//!
//! 两者共用 [`net`] 里的 HTTP 传输层与 UTC 时间换算。公开 API 是 `async` 的，运行时由调用方提供
//! （`reqwest` 要求 tokio）。开启这两个 feature 会把 MSRV 抬到 1.85。
//!
//! 上游的绘图（`plot.py`）与 MCP server（`mcp_server.py`）未移植：前者绑定 matplotlib，
//! 本仓库改为把净值、回撤、换手率都暴露出来由调用方作图；后者是反射自身的胶水层。

pub mod analysis;
pub mod backtest;
pub mod factor;

#[cfg(feature = "_net")]
pub mod net;

#[cfg(feature = "data")]
pub mod data;
