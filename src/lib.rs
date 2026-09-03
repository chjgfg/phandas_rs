//! phandas_rs：Python 版 phandas 的 Rust 移植。
//!
//! - [`factor`]：因子构造。`Factor` / `Panel` 与 70 多个横截面、时序、分组运算子，
//!   对应上游 `core.py` / `operators.py` / `panel.py` / `constants.py`。
//! - [`backtest`]：事件驱动回测。目标市值、手续费、净值与 11 项绩效指标，
//!   对应上游 `backtest.py`。
//!
//! 全程零外部依赖：分位数反函数与 CDF、最小二乘、条件数估计、CSV 解析、日期运算
//! 都在仓库内实现。上游的行情抓取、绘图与实盘下单未移植，它们各自绑定
//! ccxt / matplotlib / python-okx。

pub mod backtest;
pub mod factor;
