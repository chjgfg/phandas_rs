//! 回测模块：Python 版 phandas 中 `backtest.py` 的 Rust 实现。
//!
//! # 设计
//!
//! - **事件驱动、T+1 调仓**：第 `i` 期用第 `i-1` 期的因子值算目标市值，按第 `i` 期的
//!   成交价因子（上游惯例用 `open`）成交。
//! - **一天一条净值**，记录点在当日成交之前，与上游第一次 `update_market_value` 对齐。
//! - **零外部依赖**：上游用到的 `scipy.stats.{linregress, skew, kurtosis, norm}` 在此
//!   分别由 `stats::pearson_r`、`stats::skew`、`stats::kurtosis_pearson` 与
//!   `factor::numeric::norm_cdf` 顶替，均为本仓库自实现。
//! - **不含绘图**：上游 `plot_equity` 走 matplotlib，此处改为把净值、回撤、换手率与
//!   等权基准都暴露出来，由调用方决定怎么画。
//!
//! # 文件划分
//!
//! | 文件 | 内容 |
//! |---|---|
//! | `date.rs` | 民用日期 ↔ 天数序号，供年化与回撤时长使用（公开模块） |
//! | `stats.rs` | pandas / scipy 口径的分位数、相关系数、有偏偏度与峰度（公开模块） |
//! | `metrics.rs` | [`Metrics`]、[`DrawdownPeriod`] 与绩效指标计算 |
//! | `portfolio.rs` | [`Portfolio`]、[`Trade`]：现金、持仓、流水与市价重估 |
//! | `engine.rs` | [`Backtester`]、[`Neutralization`]、[`backtest`] |
//!
//! # 可以单独用的构件
//!
//! 不跑完整回测也能用其中几块：[`performance_metrics`] 直接评价外部给来的收益率序列、
//! [`identify_drawdown_periods`] 单独识别回撤区间、[`psr`] 单独算概率化夏普、
//! [`Portfolio`] 驱动自定义的回测循环，[`date`] 与 [`stats`] 两个模块则是通用工具。
//!
//! # 与 Python 版的已知偏差
//!
//! - **`full_rebalance = true` 只记一条净值**。上游每个交易日调 `update_market_value`
//!   两次（清仓前后各一次），使 `history` 同一天出现两条记录：收益率序列长度翻倍，
//!   年化波动、夏普、索提诺全部失真；`turnover` 用 `pd.DataFrame` 对齐重复索引还会直接
//!   抛异常，连带 `summary()` 在该模式下不可用。此处视为同一天内的一次调仓。
//!   因此 `full_rebalance = true` 时数字与上游不同；`false`（默认）应逐格一致。
//! - **[`Neutralization::parse`] 对未知字符串返回 `Err`**。上游只特判 `"none"`，
//!   其余任何字符串（含拼错的）都静默按市场中性处理。
//! - **没有 `plot_equity`**；`summary` 与 `drawdown_report` 返回 `String` 而不是直接打印，
//!   与 [`crate::factor::Factor::info`] 的风格一致。
//! - `norm_cdf` 用 Hart 有理逼近，绝对误差约 `1e-15`（scipy 为机器精度）；
//!   偏度与峰度按定义直算，极端样本上可能与 scipy 有末位差异。
//!
//! # 如实保留的 Python 侧行为
//!
//! - 建逐期缓存时，**当期只要有任一标的为 NaN 就整期丢弃**，价格与策略两侧都如此。
//! - **前一期策略数据被丢弃时目标市值为空，当期会清空全部持仓。**
//! - 目标市值用清仓**之前**的净值计算，而下单时比对的是清仓**之后**的持仓。
//! - 年化按日历天数（`365`）折算，不是交易日；无风险利率默认 `0.03`。

pub mod date;
mod engine;
mod metrics;
mod portfolio;
pub mod stats;

pub use self::engine::{backtest, Backtester, Neutralization};
pub use self::metrics::{identify_drawdown_periods, performance_metrics, psr, DrawdownPeriod, Metrics};
pub use self::portfolio::{Portfolio, Trade};
