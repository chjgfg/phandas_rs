//! 因子构造模块：Python 版 phandas 中 `Factor` / `Panel` 与全部因子运算子的 Rust 实现。
//!
//! # 设计
//!
//! - **单模块多文件**：Python 版分散在 `core.py`（Factor 与运算实现）、`panel.py`（行情容器）、
//!   `operators.py`（自由函数外壳）、`constants.py`（常量与分组定义）中的因子构造能力，
//!   全部收敛到本模块；内部按职责分文件，对外仍是 `factor` 一个模块。
//! - **稠密矩阵表示**：行 = timestamp（升序去重），列 = symbol（升序去重），值为 `f64`，
//!   缺失位置为 `NaN`。Python 侧 `groupby('timestamp')` 的横截面运算对应"逐行"；
//!   `groupby('symbol').rolling(...)` 的时序运算对应"逐列"。
//! - **二元运算按 (timestamp, symbol) 取交集**，等价于 Python 侧 `pd.merge(how='inner')`。
//! - **零外部依赖**：分位数反函数、最小二乘、条件数估计、CSV 解析均在本模块内实现。
//!
//! # 与 Python 版的已知偏差
//!
//! - `norm.ppf` 采用 Acklam 有理逼近，相对误差约 `1e-9`（scipy 为机器精度）。
//! - 时间戳按字符串字典序排序，要求输入为 ISO-8601（`2023-05-03` 或 `2023-05-03 00:00:00`）。
//! - 并列值的 `ts_rank` 采用稳定排序；numpy `argsort` 默认快排不稳定，并列时名次可能不同。
//! - Python 侧在"全为 NaN"或"合并结果为空"时抛异常，此处一律返回全 NaN / 空因子。
//! - `ts_step` 沿完整时间轴计数；Python 侧 `cumcount` 按各 symbol 自身首行起算，
//!   面板不规整（各 symbol 起始期不同）时两者会有偏差。
//! - `ts_corr` / `ts_covariance` / `ts_autocorr` 的取值与 Python 侧不同：上游用
//!   `merged.groupby('symbol').apply(...).values` 做位置赋值，而闭包返回的是
//!   MultiIndex 的 `Series`，走不到 pandas 的 transform 对齐路径，于是各组结果按分组
//!   顺序拼接后被塞回按 (timestamp, symbol) 排序的行上——标的数大于 1 时结果会串格。
//!   此处按定义正确计算，未复刻该错位。
//!
//! # 如实保留的 Python 侧异常行为
//!
//! - `ts_decay_linear` 的权重方向与 `ts_decay_exp_window` 相反：前者最早一期权重最大。
//! - `ts_skewness` 由 `ts_mean` 与 `ts_sum` 复合而成，需 `2 × window - 1` 期才有首个有效值，
//!   且偏离项取"当期值 − 当期滚动均值"，并非窗口内的标准样本偏度。
//! - `spread` 用 `np.argsort` 的"NaN 排在末尾"语义挑多头，当期存在 NaN 时多头名额会被 NaN 占用。
//! - `ts_regression` 的 `rettype = 8` 在单自变量下会索引越界；Python 侧抛 `IndexError`，
//!   此处返回 NaN。
//! - [`Factor::scalar_div`]（即 `2.0 / &f`）的除零判据是精确等零，与 [`Factor::divide`]
//!   的 `|y| > 1e-10` 不一致——这是 Python 侧 `__rtruediv__` 与 `__truediv__` 本身的不一致。
//!
//! # 文件划分
//!
//! 公开路径不受拆分影响，一律为 `phandas_rs::factor::*`。
//!
//! | 文件 | 内容 |
//! |---|---|
//! | `constants.rs` | 数值阈值与内置分组定义 |
//! | `numeric.rs` | 跳过 NaN 的统计量、名次百分位、分位数反函数、[`Driver`] |
//! | `linalg.rs` | 特征值、条件数、线性方程求解、求逆、最小二乘 |
//! | `panel.rs` | [`Panel`] 行情容器与 CSV 解析 |
//! | `core.rs` | [`Factor`] 矩阵本体、[`Operand`] 与全部运算子共用的内部机制 |
//! | `cs.rs` | 横截面运算子 |
//! | `neutralize.rs` | 向量投影 / 回归中性化 |
//! | `group.rs` | 分组运算子 |
//! | `ts.rs` | 时序运算子 |
//! | `arith.rs` | 一元数学变换、二元运算与比较，含标量在左的 `scalar_*` |
//! | `ops.rs` | `+ - * /` 与取负的运算符重载，含 `2.0 - &f` 这类标量在左的写法 |
//! | `display.rs` | 单行摘要与矩阵表格渲染 |
//! | `operators.rs` | 自由函数外壳 |

mod arith;
mod constants;
mod core;
mod cs;
mod display;
mod group;
mod linalg;
mod neutralize;
mod numeric;
mod ops;
mod operators;
mod panel;
mod ts;

#[cfg(test)]
mod tests;

pub use self::constants::{group_definitions, EPSILON, MATRIX_COND_THRESHOLD, TOLERANCE_FLOAT};
pub use self::core::{Factor, Operand};
pub use self::numeric::Driver;
pub use self::operators::*;
pub use self::panel::Panel;
