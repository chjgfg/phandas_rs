//! 自由函数外壳：对应 Python 侧 `operators.py`，便于 `rank(&close)` 式书写。
//!
//! 本文件每个函数都是同名方法的薄壳——第一个参数即接收者，其余参数原样转发，
//! 加工过程与出参完全等同于所链接的方法，故此处只标注"入参 → 出参"，
//! 详细加工步骤请看链接过去的方法文档（避免两处描述漂移）。

use std::collections::BTreeMap;

use super::core::{Factor, Operand};
use super::numeric::Driver;

/// 横截面百分位排名：`factor` 输入因子 → 取值落在 `(0, 1]` 的排名因子。
/// 详见 [`Factor::rank`]。
pub fn rank(factor: &Factor) -> Factor {
    factor.rank()
}

/// 横截面均值广播：`factor` 输入因子 → 同期取值相同的均值因子。详见 [`Factor::cs_mean`]。
pub fn mean(factor: &Factor) -> Factor {
    factor.cs_mean()
}

/// 横截面中位数广播：`factor` 输入因子 → 同期取值相同的中位数因子。
/// 详见 [`Factor::cs_median`]。
pub fn median(factor: &Factor) -> Factor {
    factor.cs_median()
}

/// 横截面去均值：`factor` 输入因子、`use_std` 是否标准化、`limit` 截断幅度
/// → 每期和为 0 的因子。详见 [`Factor::normalize`]。
pub fn normalize(factor: &Factor, use_std: bool, limit: f64) -> Factor {
    factor.normalize(use_std, limit)
}

/// 横截面 z-score：`factor` 输入因子 → 每期均值 0、标准差 1 的因子。
/// 详见 [`Factor::zscore`]。
pub fn zscore(factor: &Factor) -> Factor {
    factor.zscore()
}

/// 横截面分位数映射：`factor` 输入因子、`driver` 目标分布、`sigma` 缩放倍数
/// → 重塑成目标分布形状的因子。详见 [`Factor::quantile`]。
pub fn quantile(factor: &Factor, driver: Driver, sigma: f64) -> Factor {
    factor.quantile(driver, sigma)
}

/// 横截面缩放：`factor` 输入因子、`scale` 目标绝对值和、`longscale` / `shortscale`
/// 分侧规模 → 可直接当权重的因子。详见 [`Factor::scale`]。
pub fn scale(
    factor: &Factor,
    scale: f64,
    longscale: Option<f64>,
    shortscale: Option<f64>,
) -> Factor {
    factor.scale(scale, longscale, shortscale)
}

/// 多空价差组合：`factor` 输入因子、`pct` 单侧入选比例 → 取值仅 `±0.5 / 0` 的因子。
/// 详见 [`Factor::spread`]。
pub fn spread(factor: &Factor, pct: f64) -> Result<Factor, String> {
    factor.spread(pct)
}

/// 转为美元中性权重：`factor` 输入因子 → 多空各 0.5、总和 0 的权重因子。
/// 详见 [`Factor::signal`]。
pub fn signal(factor: &Factor) -> Factor {
    factor.signal()
}

/// 向量投影中性化：`x` 目标因子、`y` 要剔除的风格因子 → 与 `y` 正交的因子。
/// 详见 [`Factor::vector_neut`]。
pub fn vector_neut(x: &Factor, y: &Factor) -> Factor {
    x.vector_neut(y)
}

/// 回归中性化：`y` 因变量因子、`x` 单个自变量因子 → 回归残差因子。
/// 详见 [`Factor::regression_neut`]。
pub fn regression_neut(y: &Factor, x: &Factor) -> Factor {
    y.regression_neut(&[x])
}

/// 多元回归中性化：`y` 因变量因子、`xs` 多个自变量因子 → 回归残差因子。
/// 详见 [`Factor::regression_neut`]。
pub fn regression_neut_multi(y: &Factor, xs: &[&Factor]) -> Factor {
    y.regression_neut(xs)
}

/// 生成分组因子：`factor` 提供索引、`mapping` 标的到组号的映射 → 组号因子。
/// 详见 [`Factor::group_map`]。
pub fn group(factor: &Factor, mapping: &BTreeMap<String, f64>) -> Factor {
    factor.group_map(mapping, None)
}

/// 由内置分组名生成分组因子：`factor` 提供索引、`mapping` 方案名 → 组号因子。
/// 详见 [`Factor::group_named`]。
pub fn group_by_name(factor: &Factor, mapping: &str) -> Result<Factor, String> {
    factor.group_named(mapping)
}

/// 组内去均值：`x` 输入因子、`group` 组号因子 → 每组和为 0 的因子。
/// 详见 [`Factor::group_neutralize`]。
pub fn group_neutralize(x: &Factor, group: &Factor) -> Factor {
    x.group_neutralize(group)
}

/// 组内均值广播：`x` 输入因子、`group` 组号因子 → 同组取值相同的均值因子。
/// 详见 [`Factor::group_mean`]。
pub fn group_mean(x: &Factor, group: &Factor) -> Factor {
    x.group_mean(group)
}

/// 组内中位数广播：`x` 输入因子、`group` 组号因子 → 同组取值相同的中位数因子。
/// 详见 [`Factor::group_median`]。
pub fn group_median(x: &Factor, group: &Factor) -> Factor {
    x.group_median(group)
}

/// 组内百分位排名：`x` 输入因子、`group` 组号因子 → 取值落在 `(0, 1]` 的组内排名因子。
/// 详见 [`Factor::group_rank`]。
pub fn group_rank(x: &Factor, group: &Factor) -> Factor {
    x.group_rank(group)
}

/// 组内 min-max 缩放：`x` 输入因子、`group` 组号因子 → 取值落在 `[0, 1]` 的因子。
/// 详见 [`Factor::group_scale`]。
pub fn group_scale(x: &Factor, group: &Factor) -> Factor {
    x.group_scale(group)
}

/// 组内 z-score：`x` 输入因子、`group` 组号因子 → 每组均值 0、标准差 1 的因子。
/// 详见 [`Factor::group_zscore`]。
pub fn group_zscore(x: &Factor, group: &Factor) -> Factor {
    x.group_zscore(group)
}

/// 组内按绝对值和归一：`x` 输入因子、`group` 组号因子、`scale` 每组目标规模
/// → 每组绝对值和为 `scale` 的因子。详见 [`Factor::group_normalize`]。
pub fn group_normalize(x: &Factor, group: &Factor, scale: f64) -> Factor {
    x.group_normalize(group, scale)
}

/// 前移 `window` 期：`factor` 输入因子、`window` 前移期数 → 滞后因子。
/// 详见 [`Factor::ts_delay`]。
pub fn ts_delay(factor: &Factor, window: usize) -> Factor {
    factor.ts_delay(window)
}

/// 与 `window` 期前之差：`factor` 输入因子、`window` 回看期数 → 绝对变动因子。
/// 详见 [`Factor::ts_delta`]。
pub fn ts_delta(factor: &Factor, window: usize) -> Factor {
    factor.ts_delta(window)
}

/// 时间序号：`factor` 提供索引、`start` 首期序号 → 时间坐标因子。详见 [`Factor::ts_step`]。
pub fn ts_step(factor: &Factor, start: i64) -> Factor {
    factor.ts_step(start)
}

/// 滚动求和：`factor` 输入因子、`window` 窗口期数 → 窗口和因子。详见 [`Factor::ts_sum`]。
pub fn ts_sum(factor: &Factor, window: usize) -> Factor {
    factor.ts_sum(window)
}

/// 滚动乘积：`factor` 输入因子、`window` 窗口期数 → 窗口连乘因子。
/// 详见 [`Factor::ts_product`]。
pub fn ts_product(factor: &Factor, window: usize) -> Factor {
    factor.ts_product(window)
}

/// 滚动均值：`factor` 输入因子、`window` 窗口期数 → 移动平均因子。
/// 详见 [`Factor::ts_mean`]。
pub fn ts_mean(factor: &Factor, window: usize) -> Factor {
    factor.ts_mean(window)
}

/// 滚动中位数：`factor` 输入因子、`window` 窗口期数 → 移动中位数因子。
/// 详见 [`Factor::ts_median`]。
pub fn ts_median(factor: &Factor, window: usize) -> Factor {
    factor.ts_median(window)
}

/// 滚动标准差：`factor` 输入因子、`window` 窗口期数 → 波动率因子。
/// 详见 [`Factor::ts_std_dev`]。
pub fn ts_std_dev(factor: &Factor, window: usize) -> Factor {
    factor.ts_std_dev(window)
}

/// 滚动最小值：`factor` 输入因子、`window` 窗口期数 → 窗口下沿因子。
/// 详见 [`Factor::ts_min`]。
pub fn ts_min(factor: &Factor, window: usize) -> Factor {
    factor.ts_min(window)
}

/// 滚动最大值：`factor` 输入因子、`window` 窗口期数 → 窗口上沿因子。
/// 详见 [`Factor::ts_max`]。
pub fn ts_max(factor: &Factor, window: usize) -> Factor {
    factor.ts_max(window)
}

/// 窗口内最大值距当期的期数：`factor` 输入因子、`window` 窗口期数
/// → 取值落在 `[0, window - 1]` 的因子。详见 [`Factor::ts_arg_max`]。
pub fn ts_arg_max(factor: &Factor, window: usize) -> Factor {
    factor.ts_arg_max(window)
}

/// 窗口内最小值距当期的期数：`factor` 输入因子、`window` 窗口期数
/// → 取值落在 `[0, window - 1]` 的因子。详见 [`Factor::ts_arg_min`]。
pub fn ts_arg_min(factor: &Factor, window: usize) -> Factor {
    factor.ts_arg_min(window)
}

/// 窗口内 NaN 个数：`factor` 输入因子、`window` 窗口期数 → 缺失计数因子。
/// 详见 [`Factor::ts_count_nans`]。
pub fn ts_count_nans(factor: &Factor, window: usize) -> Factor {
    factor.ts_count_nans(window)
}

/// 当期值与滚动均值之差：`factor` 输入因子、`window` 均值窗口期数 → 偏离因子。
/// 详见 [`Factor::ts_av_diff`]。
pub fn ts_av_diff(factor: &Factor, window: usize) -> Factor {
    factor.ts_av_diff(window)
}

/// 滚动百分位排名：`factor` 输入因子、`window` 窗口期数
/// → 当期在自身历史中的名次因子。详见 [`Factor::ts_rank`]。
pub fn ts_rank(factor: &Factor, window: usize) -> Factor {
    factor.ts_rank(window)
}

/// 滚动 min-max 缩放：`factor` 输入因子、`window` 窗口期数、`constant` 偏移量
/// → 相对区间位置因子。详见 [`Factor::ts_scale`]。
pub fn ts_scale(factor: &Factor, window: usize, constant: f64) -> Factor {
    factor.ts_scale(window, constant)
}

/// 滚动 z-score：`factor` 输入因子、`window` 窗口期数 → 时序标准化因子。
/// 详见 [`Factor::ts_zscore`]。
pub fn ts_zscore(factor: &Factor, window: usize) -> Factor {
    factor.ts_zscore(window)
}

/// 滚动分位数映射：`factor` 输入因子、`window` 窗口期数、`driver` 目标分布
/// → 重塑分布形状的因子。详见 [`Factor::ts_quantile`]。
pub fn ts_quantile(factor: &Factor, window: usize, driver: Driver) -> Factor {
    factor.ts_quantile(window, driver)
}

/// 滚动超额峰度：`factor` 输入因子、`window` 窗口期数 → 尖峰厚尾程度因子。
/// 详见 [`Factor::ts_kurtosis`]。
pub fn ts_kurtosis(factor: &Factor, window: usize) -> Factor {
    factor.ts_kurtosis(window)
}

/// 滚动样本偏度：`factor` 输入因子、`window` 窗口期数 → 分布偏斜方向因子。
/// 详见 [`Factor::ts_skewness`]。
pub fn ts_skewness(factor: &Factor, window: usize) -> Factor {
    factor.ts_skewness(window)
}

/// 缺失值回填：`factor` 输入因子、`window` 回看窗口、`k` 取倒数第几个有效值
/// → 缺口被历史值填上的因子。详见 [`Factor::ts_backfill`]。
pub fn ts_backfill(factor: &Factor, window: usize, k: usize) -> Result<Factor, String> {
    factor.ts_backfill(window, k)
}

/// 指数衰减加权均值：`factor` 输入因子、`window` 窗口期数、`factor_arg` 衰减系数、
/// `nan` 缺失处理方式 → 平滑因子。详见 [`Factor::ts_decay_exp_window`]。
pub fn ts_decay_exp_window(
    factor: &Factor,
    window: usize,
    factor_arg: f64,
    nan: bool,
) -> Result<Factor, String> {
    factor.ts_decay_exp_window(window, factor_arg, nan)
}

/// 线性衰减加权均值：`factor` 输入因子、`window` 窗口期数、`dense` 缺失处理方式
/// → 加权平滑因子。详见 [`Factor::ts_decay_linear`]（注意其权重方向）。
pub fn ts_decay_linear(factor: &Factor, window: usize, dense: bool) -> Factor {
    factor.ts_decay_linear(window, dense)
}

/// 滚动相关系数：`factor1` / `factor2` 两个因子、`window` 窗口期数
/// → 取值落在 `[-1, 1]` 的相关性因子。详见 [`Factor::ts_corr`]。
pub fn ts_corr(factor1: &Factor, factor2: &Factor, window: usize) -> Factor {
    factor1.ts_corr(factor2, window)
}

/// 滚动协方差：`factor1` / `factor2` 两个因子、`window` 窗口期数 → 保留量纲的共动因子。
/// 详见 [`Factor::ts_covariance`]。
pub fn ts_covariance(factor1: &Factor, factor2: &Factor, window: usize) -> Factor {
    factor1.ts_covariance(factor2, window)
}

/// 滚动线性回归：`y` 因变量、`x` 自变量、`window` 窗口期数、`lag` 自变量滞后、
/// `rettype` 输出选择 → 含义随 `rettype` 变化的因子。详见 [`Factor::ts_regression`]。
pub fn ts_regression(
    y: &Factor,
    x: &Factor,
    window: usize,
    lag: usize,
    rettype: i32,
) -> Factor {
    y.ts_regression(&[x], window, lag, rettype)
}

/// 滚动多元线性回归：`y` 因变量、`xs` 多个自变量、`window` 窗口期数、`lag` 滞后、
/// `rettype` 输出选择 → 含义随 `rettype` 变化的因子。详见 [`Factor::ts_regression`]。
pub fn ts_regression_multi(
    y: &Factor,
    xs: &[&Factor],
    window: usize,
    lag: usize,
    rettype: i32,
) -> Factor {
    y.ts_regression(xs, window, lag, rettype)
}

/// 滚动变异系数：`factor` 输入因子、`window` 窗口期数 → 无量纲相对波动因子。
/// 详见 [`Factor::ts_cv`]。
pub fn ts_cv(factor: &Factor, window: usize) -> Factor {
    factor.ts_cv(window)
}

/// 滚动跳跃度：`factor` 输入因子、`window` 窗口期数 → 震荡 / 趋势度量因子。
/// 详见 [`Factor::ts_jumpiness`]。
pub fn ts_jumpiness(factor: &Factor, window: usize) -> Factor {
    factor.ts_jumpiness(window)
}

/// 滚动趋势强度：`factor` 输入因子、`window` 窗口期数 → 取值落在 `[0, 1]` 的 R² 因子。
/// 详见 [`Factor::ts_trend_strength`]。
pub fn ts_trend_strength(factor: &Factor, window: usize) -> Factor {
    factor.ts_trend_strength(window)
}

/// 滚动方差比：`factor` 输入因子、`window` 窗口期数、`k` 长周期差分跨度
/// → 围绕 1 波动的趋势 / 震荡因子。详见 [`Factor::ts_vr`]。
pub fn ts_vr(factor: &Factor, window: usize, k: usize) -> Result<Factor, String> {
    factor.ts_vr(window, k)
}

/// 滚动自相关：`factor` 输入因子、`window` 窗口期数、`lag` 滞后期数
/// → 取值落在 `[-1, 1]` 的自相关因子。详见 [`Factor::ts_autocorr`]。
pub fn ts_autocorr(factor: &Factor, window: usize, lag: usize) -> Result<Factor, String> {
    factor.ts_autocorr(window, lag)
}

/// 滚动反转频率：`factor` 输入因子、`window` 窗口期数
/// → 取值落在 `[0, 1]` 的方向翻转频率因子。详见 [`Factor::ts_reversal_count`]。
pub fn ts_reversal_count(factor: &Factor, window: usize) -> Factor {
    factor.ts_reversal_count(window)
}

/// 自然对数：`factor` 输入因子 → 对数因子（非正值为 NaN）。详见 [`Factor::ln`]。
pub fn ln(factor: &Factor) -> Factor {
    factor.ln()
}

/// 对数：`factor` 输入因子、`base` 底数（`None` 为自然底）→ 对数因子。
/// 详见 [`Factor::log`]。
pub fn log(factor: &Factor, base: Option<f64>) -> Result<Factor, String> {
    factor.log(base)
}

/// 保号对数：`factor` 输入因子 → `sign(x) · ln(1 + |x|)` 因子。详见 [`Factor::s_log_1p`]。
pub fn s_log_1p(factor: &Factor) -> Factor {
    factor.s_log_1p()
}

/// 符号函数：`factor` 输入因子 → 取值仅 `-1 / 0 / 1` 的方向因子。详见 [`Factor::sign`]。
pub fn sign(factor: &Factor) -> Factor {
    factor.sign()
}

/// 平方根：`factor` 输入因子 → 开方因子（负值为 NaN）。详见 [`Factor::sqrt`]。
pub fn sqrt(factor: &Factor) -> Factor {
    factor.sqrt()
}

/// 倒数：`factor` 输入因子 → `1 / x` 因子（零值为 NaN）。详见 [`Factor::inverse`]。
pub fn inverse(factor: &Factor) -> Factor {
    factor.inverse()
}

/// 绝对值：`factor` 输入因子 → 绝对值因子。详见 [`Factor::abs`]。
pub fn abs(factor: &Factor) -> Factor {
    factor.abs()
}

/// 取负：`factor` 输入因子 → 方向翻转后的因子。详见 [`Factor::reverse`]。
pub fn reverse(factor: &Factor) -> Factor {
    factor.reverse()
}

/// 逐元素取较大者：`factor` 左操作数、`other` 右操作数（因子或标量）→ 逐格较大值因子。
/// 详见 [`Factor::maximum`]。
pub fn maximum<'a>(factor: &Factor, other: impl Into<Operand<'a>>) -> Factor {
    factor.maximum(other)
}

/// 逐元素取较小者：`factor` 左操作数、`other` 右操作数（因子或标量）→ 逐格较小值因子。
/// 详见 [`Factor::minimum`]。
pub fn minimum<'a>(factor: &Factor, other: impl Into<Operand<'a>>) -> Factor {
    factor.minimum(other)
}

/// 幂运算：`base` 底数因子、`exponent` 指数（因子或标量）→ 幂因子。详见 [`Factor::power`]。
pub fn power<'a>(base: &Factor, exponent: impl Into<Operand<'a>>) -> Factor {
    base.power(exponent)
}

/// 保号幂运算：`base` 底数因子、`exponent` 指数（因子或标量）→ 保留方向的幂因子。
/// 详见 [`Factor::signed_power`]。
pub fn signed_power<'a>(base: &Factor, exponent: impl Into<Operand<'a>>) -> Factor {
    base.signed_power(exponent)
}

/// 加法：`factor1` 左操作数、`factor2` 右操作数（因子或标量）→ 求和因子。
/// 详见 [`Factor::add`]。
pub fn add<'a>(factor1: &Factor, factor2: impl Into<Operand<'a>>) -> Factor {
    factor1.add(factor2)
}

/// 减法：`factor1` 被减数、`factor2` 减数（因子或标量）→ 差值因子。
/// 详见 [`Factor::subtract`]。
pub fn subtract<'a>(factor1: &Factor, factor2: impl Into<Operand<'a>>) -> Factor {
    factor1.subtract(factor2)
}

/// 乘法：`factor1` 左操作数、`factor2` 右操作数（因子或标量）→ 乘积因子。
/// 详见 [`Factor::multiply`]。
pub fn multiply<'a>(factor1: &Factor, factor2: impl Into<Operand<'a>>) -> Factor {
    factor1.multiply(factor2)
}

/// 除法：`factor1` 被除数、`factor2` 除数（因子或标量）→ 商因子（除零处为 NaN）。
/// 详见 [`Factor::divide`]。
pub fn divide<'a>(factor1: &Factor, factor2: impl Into<Operand<'a>>) -> Factor {
    factor1.divide(factor2)
}

/// 条件选择：`condition` 条件因子、`x` 为真时的取值、`y` 为假时的取值（因子或标量）
/// → 二选一后的因子。详见 [`Factor::where_cond`]。
///
/// 对应 Python 侧 `where(condition, x, y)`（`where` 是 Rust 关键字，故加下划线）。
pub fn where_<'a>(condition: &Factor, x: &Factor, y: impl Into<Operand<'a>>) -> Factor {
    x.where_cond(condition, y)
}

/// 渲染因子矩阵（默认显示 20 期），对应 Python 侧 `show(obj)`。
///
/// - 入参：`factor` 待渲染因子。
/// - 加工：以 `max_rows = 20` 调用 [`Factor::show`]。
/// - 出参：多行可打印字符串。
pub fn show(factor: &Factor) -> String {
    factor.show(20)
}

/// 写出 CSV 文件，对应 Python 侧 `to_csv(obj, path)`。
///
/// - 入参：`factor` 待导出因子；`path` 目标文件路径。
/// - 加工：先由 [`Factor::to_csv_string`] 生成文本，再整体写盘（已存在的文件会被覆盖）。
/// - 出参：`Ok(())`；写入失败时返回带路径信息的 `Err`。
pub fn to_csv<P: AsRef<std::path::Path>>(factor: &Factor, path: P) -> Result<(), String> {
    std::fs::write(path.as_ref(), factor.to_csv_string())
        .map_err(|e| format!("写入 {} 失败：{e}", path.as_ref().display()))
}

/// 展开为长表记录，对应 Python 侧 `to_df(obj)`。
///
/// - 入参：`factor` 待展开因子。
/// - 加工：转发到 [`Factor::to_records`]。
/// - 出参：`(timestamp, symbol, value)` 记录向量，含 NaN 单元格。
pub fn to_records(factor: &Factor) -> Vec<(String, String, f64)> {
    factor.to_records()
}
