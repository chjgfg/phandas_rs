//! 四个数据源，对应上游 `fetch_binance` / `fetch_benchmark` / `fetch_calendar` / `fetch_vwap`。
//!
//! 每个源往中间表 `Frame` 里写若干列，最后由 `process` 统一对齐成 `Panel`。

use std::collections::{BTreeMap, BTreeSet};

use crate::net::time::{floor_to_day, split_millis, weekday_mon0};
use crate::net::HttpTransport;

use super::klines::{BinanceClient, Kline};
use super::process::Frame;
use super::symbol::{market_id, rename_for};
use super::timeframe::Timeframe;

/// 从一根 K 线里取一列的取值器。
type Pick = fn(&Kline) -> f64;

/// OHLCV 五列的列名与取值器，顺序固定。
const OHLCV: [(&str, Pick); 5] = [
    ("open", |k| k.open),
    ("high", |k| k.high),
    ("low", |k| k.low),
    ("close", |k| k.close),
    ("volume", |k| k.volume),
];

/// 往中间表里写一格。
///
/// - 入参：`frame` 中间表；`col` 列名；`ms` 开盘毫秒；`sym` 裸标的名；`v` 取值。
/// - 加工：列不存在时先建列，再写格子。**同键重复写以后写者为准**。
/// - 出参：无。
///
/// 上游这里是 `pivot_table` 的默认 `aggfunc='mean'`，重复的 `(时刻, 标的)` 会被**取平均**。
/// 本仓库后写覆盖：分页游标按「上一批最后一根 + 1 毫秒」推进，正常路径下不会产生重复键，
/// 上游那个平均只是 pandas 的副产物而非本意。
fn put(frame: &mut Frame, col: &str, ms: i64, sym: &str, v: f64) {
    frame
        .entry(col.to_string())
        .or_default()
        .insert((ms, sym.to_string()), v);
}

/// 逐个标的抓 OHLCV，写进一张新的中间表。
///
/// - 入参：`client` Binance 客户端；`symbols` 要抓的裸标的名（去重后）；`tf` 周期；
///   `start` / `end` 区间毫秒；`listed` 已上市的市场 id 集合。
/// - 加工：对每个标的先查 `{SYM}USDT` 在不在 `listed`——不在就**整个跳过**（上游发一条
///   warning 后 `return None`，本仓库同样跳过，只是不打日志）；在则拉全量 K 线写进五列。
/// - 出参：`Ok(中间表)`；任一标的的请求失败即整体 `Err`。
///
/// 注意与上游的一处差异：上游把**单个标的的任何异常**都吞成 warning 并丢掉该标的
/// （`except Exception: warnings.warn(...); return None`），于是一次网络抖动会静默少一个标的、
/// 且因为 `common_start` 的存在还可能连带改变整篮子的起点。本仓库让错误冒出来，
/// 由调用方决定重试还是放弃。
async fn ohlcv<T: HttpTransport>(
    client: &BinanceClient<T>,
    symbols: &[&str],
    tf: Timeframe,
    start: Option<i64>,
    end: Option<i64>,
    listed: &BTreeSet<String>,
) -> Result<Frame, String> {
    let mut frame = Frame::new();
    for sym in symbols {
        if !listed.contains(&market_id(sym)) {
            continue;
        }
        for k in client.klines(&market_id(sym), tf, start, end).await? {
            for (col, get) in OHLCV {
                put(&mut frame, col, k.open_time, sym, get(&k));
            }
        }
    }
    Ok(frame)
}

/// `binance` 源：抓 OHLCV，处理历史改名。对应上游 `fetch_binance`。
///
/// - 入参：`client` 客户端；`symbols` 裸标的名；`tf` 周期；`start` / `end` 区间毫秒；
///   `listed` 已上市市场 id 集合。
/// - 加工：请求里若含有改名过的标的（目前只有 `POL`）且区间跨过分界日，就分两段抓——
///   老段用旧符号（`MATIC`）取 `[start, cutoff - 1]`、新段用原符号表取 `[cutoff, end]`，
///   老段的旧符号改写成新符号后并进来，最后**只对新符号那批**按周期网格补齐并把成交量的
///   空缺置 0。其余情况单段抓完即可。
/// - 出参：`Ok(中间表)`。
///
/// # 照抄的上游行为
///
/// - 老段会**连带把其余标的一起重抓一遍**（上游 `['MATIC'] + [s for s in symbols if s != 'POL']`），
///   所以跨分界日的请求对非改名标的也是两段拼起来的。
/// - 若旧符号已下架（`MATIC/USDT` 查不到），老段拿不到任何数据，于是**整篮子 cutoff 之前的
///   历史全部消失**，且没有任何警告。这是上游的连带效应，如实保留。
/// - 补齐那一步上游把频率写死成日线（`freq='D'`），日内周期下会把改名标的压成一天一根。
///   本仓库按传入的 `tf` 补齐，**这一条修掉了**——写死日线在日内周期下等于毁掉该标的的数据，
///   不像别的口径差异那样只是「不同」。
pub(crate) async fn binance<T: HttpTransport>(
    client: &BinanceClient<T>,
    symbols: &[&str],
    tf: Timeframe,
    start: Option<i64>,
    end: Option<i64>,
    listed: &BTreeSet<String>,
) -> Result<Frame, String> {
    let uniq: Vec<&str> = symbols
        .iter()
        .copied()
        .collect::<BTreeSet<&str>>()
        .into_iter()
        .collect();

    // 上游在循环里直接 return，故只处理第一条命中的改名规则
    let hit = uniq.iter().find_map(|s| {
        let r = rename_for(s)?;
        let cutoff = crate::net::date_to_millis(r.cutoff_date)?;
        (start.is_none_or(|s| s < cutoff)).then_some((r, cutoff))
    });
    let Some((rename, cutoff)) = hit else {
        return ohlcv(client, &uniq, tf, start, end, listed).await;
    };

    let old_syms: Vec<&str> = std::iter::once(rename.old_symbol)
        .chain(uniq.iter().copied().filter(|s| *s != rename.new_symbol))
        .collect();
    let old = ohlcv(client, &old_syms, tf, start, Some(cutoff - 1), listed).await?;
    let new = ohlcv(client, &uniq, tf, Some(cutoff), end, listed).await?;

    Ok(splice_rename(
        old,
        new,
        rename.old_symbol,
        rename.new_symbol,
        tf,
    ))
}

/// 把改名前后的两段拼起来，对应上游 `fetch_binance` 里 `old_data` / `new_data` 的四个分支。
///
/// - 入参：`old` 分界日之前那段（含旧符号）；`new` 分界日及之后那段；`old_sym` / `new_sym`
///   旧、新符号；`tf` 周期（补齐网格用）。
/// - 加工：先把老段里旧符号的格子改挂到新符号名下、并进新段；再**只对新符号那批**按 `tf` 的
///   网格从首格补到末格——网格点缺值时价格类走前向填充、`volume` 置 0（上游
///   `renamed_rows['volume'].fillna(0)`）。
/// - 出参：合并后的中间表。任一段为空时退化成另一段，两段都空则返回空表。
fn splice_rename(old: Frame, new: Frame, old_sym: &str, new_sym: &str, tf: Timeframe) -> Frame {
    let mut merged: Frame = Frame::new();
    for (col, cells) in old.into_iter().chain(new.into_iter()) {
        let target = merged.entry(col).or_default();
        for ((ms, sym), v) in cells {
            let sym = if sym == old_sym {
                new_sym.to_string()
            } else {
                sym
            };
            target.insert((ms, sym), v);
        }
    }

    // 只对新符号补齐网格：取它在各列里出现过的最早与最晚时刻
    let span = merged
        .values()
        .flat_map(|c| c.keys().filter(|(_, s)| s == new_sym).map(|(ms, _)| *ms));
    let (Some(lo), Some(hi)) = (span.clone().min(), span.max()) else {
        return merged;
    };

    let mut grid = Vec::new();
    let mut t = lo;
    while t <= hi {
        grid.push(t);
        t = tf.advance(t);
    }
    for (col, cells) in merged.iter_mut() {
        let zero_fill = col == "volume";
        let mut last = f64::NAN;
        for &g in &grid {
            match cells.get(&(g, new_sym.to_string())) {
                Some(v) if !v.is_nan() => last = *v,
                _ => {
                    let v = if zero_fill { 0.0 } else { last };
                    cells.insert((g, new_sym.to_string()), v);
                }
            }
        }
    }
    merged
}

/// `benchmark` 源：把 BTC 与 ETH 的收盘价广播到每个标的，成 `BTC_close` / `ETH_close` 两列。
/// 对应上游 `fetch_benchmark`。
///
/// - 入参：同 [`binance`]，`symbols` 是要广播到的标的（**不去重**，与上游一致；
///   重复项在本仓库只是覆盖同一格）。
/// - 加工：分别抓 `BTCUSDT` / `ETHUSDT` 的 K 线，只留收盘价，再把每个时刻的两个值写到
///   **每个**请求标的名下。
/// - 出参：`Ok(中间表)`，含 `BTC_close` / `ETH_close` 两列。
///
/// 上游这里的区间**不**跟着请求标的的可用范围收窄，也不参与改名逻辑，如实保留。
pub(crate) async fn benchmark<T: HttpTransport>(
    client: &BinanceClient<T>,
    symbols: &[&str],
    tf: Timeframe,
    start: Option<i64>,
    end: Option<i64>,
    listed: &BTreeSet<String>,
) -> Result<Frame, String> {
    let mut frame = Frame::new();
    for base in ["BTC", "ETH"] {
        if !listed.contains(&market_id(base)) {
            continue;
        }
        let col = format!("{base}_close");
        for k in client.klines(&market_id(base), tf, start, end).await? {
            for sym in symbols {
                put(&mut frame, &col, k.open_time, sym, k.close);
            }
        }
    }
    Ok(frame)
}

/// `calendar` 源：纯计算，不联网。对应上游 `fetch_calendar`。
///
/// - 入参：`symbols` 标的名；`tf` 周期（决定网格步长）；`start` / `end` 区间毫秒，**两者都必须给**。
/// - 加工：从 `start` 按 `tf` 的网格走到 `end`，每格对每个标的写六列日历特征。
/// - 出参：`Ok(中间表)`；`start` 或 `end` 缺失时返回 `Err`（上游抛
///   `ValueError("Calendar requires both start_date and end_date")`）。
///
/// 六列的公式照抄上游：
///
/// | 列 | 公式 | 例 |
/// |---|---|---|
/// | `year` / `month` / `day` | 日历分量 | |
/// | `dayofweek` | `weekday + 1` | 周一 1 … 周日 7 |
/// | `dayofmonth_position` | `1 + (day - 1) / 10` | 1–10 → 1，11–20 → 2，21–30 → 3，**31 → 4** |
/// | `is_week_end` | `weekday >= 5` | 周六 / 周日 → 1 |
pub(crate) fn calendar(
    symbols: &[&str],
    tf: Timeframe,
    start: Option<i64>,
    end: Option<i64>,
) -> Result<Frame, String> {
    let (Some(start), Some(end)) = (start, end) else {
        return Err("Calendar requires both start_date and end_date".to_string());
    };
    let mut frame = Frame::new();
    let mut t = start;
    while t <= end {
        let (y, m, d, ..) = split_millis(t);
        let w = weekday_mon0(t);
        for sym in symbols {
            put(&mut frame, "year", t, sym, y as f64);
            put(&mut frame, "month", t, sym, m as f64);
            put(&mut frame, "day", t, sym, d as f64);
            put(&mut frame, "dayofweek", t, sym, (w + 1) as f64);
            put(
                &mut frame,
                "dayofmonth_position",
                t,
                sym,
                (1 + (d - 1) / 10) as f64,
            );
            put(&mut frame, "is_week_end", t, sym, i64::from(w >= 5) as f64);
        }
        t = tf.advance(t);
    }
    Ok(frame)
}

/// `vwap` 源：成交量加权均价。对应上游 `fetch_vwap`。
///
/// - 入参：同 [`binance`]。
/// - 加工：典型价取 `(high + low + close) / 3`，`pv = 典型价 × volume`。
///   **日线**（`tf == D1`）时内部改抓 `1h`，再按 UTC 自然日聚合 `Σpv / Σvolume`，时间戳落在当日
///   零点；**其余周期**按自然日重置做**累计** VWAP（每根 K 线一个值，等于当日到该根为止的
///   加权均价），时间戳即该根的开盘时刻。最后按 `start` 再裁一刀。
/// - 出参：`Ok(中间表)`，含 `vwap` 一列。
///
/// 照抄的上游行为：`Σvolume` 为 0 时不做保护，结果是 `inf` 或 `NaN`。另外日线路径下
/// `end_date` 只放行当天 00:00 那一根小时线，故**最后一天的 VWAP 只由 00:00–01:00 这一根决定**
/// ——这是上游 `until` 取午夜的连带效应。
pub(crate) async fn vwap<T: HttpTransport>(
    client: &BinanceClient<T>,
    symbols: &[&str],
    tf: Timeframe,
    start: Option<i64>,
    end: Option<i64>,
    listed: &BTreeSet<String>,
) -> Result<Frame, String> {
    let daily = tf == Timeframe::D1;
    let fetch_tf = if daily { Timeframe::H1 } else { tf };

    let mut frame = Frame::new();
    for sym in symbols {
        if !listed.contains(&market_id(sym)) {
            continue;
        }
        let bars = client.klines(&market_id(sym), fetch_tf, start, end).await?;
        // 按自然日累计 (Σpv, Σvolume)
        let mut day = i64::MIN;
        let (mut pv, mut vol) = (0.0f64, 0.0f64);
        let mut daily_out: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
        for k in &bars {
            let d = floor_to_day(k.open_time);
            if d != day {
                day = d;
                pv = 0.0;
                vol = 0.0;
            }
            pv += (k.high + k.low + k.close) / 3.0 * k.volume;
            vol += k.volume;
            if daily {
                daily_out.insert(d, (pv, vol));
            } else {
                put(&mut frame, "vwap", k.open_time, sym, pv / vol);
            }
        }
        for (d, (pv, vol)) in daily_out {
            put(&mut frame, "vwap", d, sym, pv / vol);
        }
    }

    if let Some(s) = start {
        for cells in frame.values_mut() {
            cells.retain(|(ms, _), _| *ms >= s);
        }
    }
    Ok(frame)
}
