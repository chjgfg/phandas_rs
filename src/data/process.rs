//! 把抓回来的散格子对齐成一张 [`Panel`]，对应上游 `_process_data`。
//!
//! 上游那段做四件事：以 `close` 为准找**公共起点**、按周期重建**时间网格**、逐列
//! **前向填充**、再把各列**外连接**合起来。这里逐条复刻，只有两处例外，见下。

use std::collections::{BTreeMap, BTreeSet};

use crate::factor::Panel;
use crate::net::{millis_to_date, millis_to_datetime};

use super::timeframe::Timeframe;

/// 一格的键：`(开盘毫秒, 裸标的名)`。
pub(crate) type CellKey = (i64, String);

/// 一列的全部格子。`BTreeMap` 的迭代序是「先按时刻升序、再按标的名」，公共起点的计算靠这个序。
pub(crate) type Column = BTreeMap<CellKey, f64>;

/// 抓取过程中的中间表：列名 → 该列的格子。
///
/// 列名用 `BTreeMap` 存，故最终列序是名字升序——与 [`Panel`] 内部一致（上游保留原始列序，
/// 本仓库的 `Panel` 本来就按列名升序，见 `Panel::to_csv_string` 的说明）。
pub(crate) type Frame = BTreeMap<String, Column>;

/// 把中间表对齐成面板。
///
/// - 入参：`frame` 各列的散格子；`tf` 周期（决定网格步长与时间戳格式）；
///   `user_symbols` 调用方要的标的（用于最后一道过滤，把改名用的辅助符号剔掉）。
/// - 加工：
///   1. 取 `close` 列，算出各标的**首个有效收盘**的时刻，取其**最大值**作为公共起点
///      `common_start`——晚上市的标的会把整篮子的历史砍到它的上市日。
///   2. 末点取全部列里最晚的时刻（上游 `df['timestamp'].max()`）。
///   3. 从 `common_start` 反复调 [`Timeframe::advance`] 生成网格，故相位锚在真实数据上。
///   4. 每列每标的按网格重建索引：网格点上有数据就取，没有就**无界前向填充**。
///   5. 标的集 = 各列出现过的标的的并集 ∩ `user_symbols`，输出是 `网格 × 标的` 的稠密矩阵。
/// - 出参：`Ok(Panel)`；缺 `close` 列、`close` 全为 NaN 或没有任何数据时返回 `Err`。
pub(crate) fn process(
    frame: &Frame,
    tf: Timeframe,
    user_symbols: &[&str],
) -> Result<Panel, String> {
    let close = frame.get("close").ok_or_else(|| {
        "缺少 close 列：对齐要以收盘价为准，请把 binance 源放进 sources".to_string()
    })?;

    // 各标的的首个有效收盘时刻。Column 的迭代序保证同一标的第一次命中就是最早的那格
    let mut first_valid: BTreeMap<&str, i64> = BTreeMap::new();
    for ((ms, sym), v) in close {
        if !v.is_nan() {
            first_valid.entry(sym.as_str()).or_insert(*ms);
        }
    }
    let common_start = first_valid
        .values()
        .copied()
        .max()
        .ok_or_else(|| "close 列没有任何有效值，无法确定公共起点".to_string())?;

    let end = frame
        .values()
        .flat_map(|c| c.keys().map(|(ms, _)| *ms))
        .max()
        .ok_or_else(|| "没有抓到任何数据".to_string())?;

    let mut grid = Vec::new();
    let mut t = common_start;
    while t <= end {
        grid.push(t);
        t = tf.advance(t);
    }
    if grid.is_empty() {
        return Err("时间网格为空：公共起点晚于最末数据点".to_string());
    }

    // 标的集：各列出现过的标的的并集，再按调用方要的那批过滤
    let wanted: BTreeSet<&str> = user_symbols.iter().copied().collect();
    let symbols: Vec<String> = frame
        .values()
        .flat_map(|c| c.keys().map(|(_, s)| s.as_str()))
        .filter(|s| wanted.contains(s))
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .map(str::to_string)
        .collect();
    if symbols.is_empty() {
        return Err("过滤后一个标的都不剩，检查 symbols 入参".to_string());
    }

    // 逐列逐标的按网格重建索引 + 前向填充
    let names: Vec<&str> = frame.keys().map(String::as_str).collect();
    let filled: Vec<Vec<Vec<f64>>> = names
        .iter()
        .map(|name| {
            let col = &frame[*name];
            symbols
                .iter()
                .map(|sym| reindex_ffill(col, sym, &grid))
                .collect()
        })
        .collect();

    let stamp = |ms: i64| {
        if tf.is_intraday() {
            millis_to_datetime(ms)
        } else {
            millis_to_date(ms)
        }
    };
    let mut records = Vec::with_capacity(grid.len() * symbols.len());
    for (gi, &ms) in grid.iter().enumerate() {
        for (si, sym) in symbols.iter().enumerate() {
            let vals: Vec<f64> = (0..names.len()).map(|ci| filled[ci][si][gi]).collect();
            records.push((stamp(ms), sym.clone(), vals));
        }
    }
    Panel::from_records(&names, records)
}

/// 把一列里某个标的的取值按网格重排并前向填充，对应上游 `reindex(full_range).ffill()`。
///
/// - 入参：`col` 该列的全部格子；`sym` 标的名；`grid` 时间网格（升序）。
/// - 加工：逐个网格点查值——查得到就用它并记为「最近有效值」，查不到（或是 NaN）就沿用最近
///   有效值。**不在网格上的时刻直接被丢弃**，这正是 `reindex` 的语义，也是上游 `1w` 整块出 NaN
///   的根因（那边网格相位错了，一个点都对不上）；本仓库网格起点取自真实数据，不会错位。
/// - 出参：与 `grid` 等长的取值向量；网格开头若还没出现过有效值则保持 NaN。
///
/// **无界前向填充**：没有 `limit`，停牌 / 退市的标的会把最后一个价格一路铺到样本末尾，
/// 成交量也照填（不置 0）。这是上游行为，如实保留——改了会动下游所有数字。
fn reindex_ffill(col: &Column, sym: &str, grid: &[i64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(grid.len());
    let mut last = f64::NAN;
    for &g in grid {
        let v = col.get(&(g, sym.to_string())).copied().unwrap_or(f64::NAN);
        if !v.is_nan() {
            last = v;
        }
        out.push(last);
    }
    out
}
