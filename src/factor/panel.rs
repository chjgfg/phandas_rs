//! Panel：多列行情容器，对应 Python 侧 `panel.Panel`。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::core::Factor;
use super::numeric::count_valid;

/// 极简 CSV 行切分：不处理引号包裹的字段。
///
/// - 入参：`line` 一行原始文本。
/// - 加工：去掉行尾的 `\r` / `\n` 后按逗号切分。
/// - 出参：字段切片向量（借用原字符串，不复制）。
fn split_csv_line(line: &str) -> Vec<&str> {
    line.trim_end_matches(['\r', '\n']).split(',').collect()
}

/// 多列行情容器：共享同一 (timestamp × symbol) 索引的若干数值列。
#[derive(Debug, Clone)]
pub struct Panel {
    /// 时间索引，升序去重；所有列共享。
    timestamps: Vec<String>,
    /// 标的索引，升序去重；所有列共享。
    symbols: Vec<String>,
    /// 每列长度均为 `timestamps.len() * symbols.len()`，行主序。
    columns: BTreeMap<String, Vec<f64>>,
}

impl Panel {
    /// 由 `(timestamp, symbol, 各列值)` 记录构建。`column_names` 给出值列的顺序与名称。
    ///
    /// - 入参：`column_names` 数值列名（决定每条记录中值的顺序）；
    ///   `records` 长表记录，每条含时间戳、标的与与列数等长的值向量。
    /// - 加工：扫描全部记录收集时间戳与标的的去重升序集合 → 按 `T × N` 分配全 NaN 矩阵
    ///   → 把每条记录的值写入对应单元格。缺失的 (时间, 标的) 组合保持 NaN。
    /// - 出参：`Ok(Panel)`；列名为空、或某条记录的值个数与列数不符时返回 `Err`。
    pub fn from_records(
        column_names: &[&str],
        records: Vec<(String, String, Vec<f64>)>,
    ) -> Result<Panel, String> {
        if column_names.is_empty() {
            return Err("至少需要一个数值列".to_string());
        }
        let mut ts_set: BTreeSet<String> = BTreeSet::new();
        let mut sym_set: BTreeSet<String> = BTreeSet::new();
        for (ts, sym, vals) in &records {
            if vals.len() != column_names.len() {
                return Err(format!(
                    "记录 ({ts}, {sym}) 的值个数 {} 与列数 {} 不一致",
                    vals.len(),
                    column_names.len()
                ));
            }
            ts_set.insert(ts.clone());
            sym_set.insert(sym.clone());
        }

        let timestamps: Vec<String> = ts_set.into_iter().collect();
        let symbols: Vec<String> = sym_set.into_iter().collect();
        let ts_pos: BTreeMap<&str, usize> =
            timestamps.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        let sym_pos: BTreeMap<&str, usize> =
            symbols.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();

        let cell_count = timestamps.len() * symbols.len();
        let mut columns: BTreeMap<String, Vec<f64>> = column_names
            .iter()
            .map(|c| ((*c).to_string(), vec![f64::NAN; cell_count]))
            .collect();

        for (ts, sym, vals) in &records {
            let offset = ts_pos[ts.as_str()] * symbols.len() + sym_pos[sym.as_str()];
            for (name, value) in column_names.iter().zip(vals.iter()) {
                columns.get_mut(*name).expect("列已预建")[offset] = *value;
            }
        }

        Ok(Panel { timestamps, symbols, columns })
    }

    /// 解析 CSV 文本。首行须为表头，且包含 `timestamp` 与 `symbol` 两列；
    /// 其余列按 `f64` 解析，空串或非法值记为 NaN。
    ///
    /// - 入参：`text` 完整 CSV 文本。
    /// - 加工：跳过空行 → 解析表头定位 `timestamp` / `symbol` 列 → 其余列视为数值列
    ///   → 逐行切分并解析成记录 → 交给 [`Panel::from_records`] 装配矩阵。
    /// - 出参：`Ok(Panel)`；文本为空、缺少必需列、或某行字段数不足时返回 `Err`。
    pub fn from_csv_str(text: &str) -> Result<Panel, String> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let header = lines.next().ok_or_else(|| "CSV 为空".to_string())?;
        let cols = split_csv_line(header);
        let ts_idx = cols
            .iter()
            .position(|c| c.trim() == "timestamp")
            .ok_or_else(|| "Data must have 'timestamp' and 'symbol' columns".to_string())?;
        let sym_idx = cols
            .iter()
            .position(|c| c.trim() == "symbol")
            .ok_or_else(|| "Data must have 'timestamp' and 'symbol' columns".to_string())?;

        let value_cols: Vec<(usize, String)> = cols
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != ts_idx && *i != sym_idx)
            .map(|(i, c)| (i, c.trim().to_string()))
            .collect();
        let names: Vec<&str> = value_cols.iter().map(|(_, n)| n.as_str()).collect();

        let mut records: Vec<(String, String, Vec<f64>)> = Vec::new();
        for (lineno, line) in lines.enumerate() {
            let fields = split_csv_line(line);
            let need = value_cols.iter().map(|(i, _)| *i).max().unwrap_or(0).max(ts_idx).max(sym_idx);
            if fields.len() <= need {
                return Err(format!("第 {} 行字段不足：{line}", lineno + 2));
            }
            let vals = value_cols
                .iter()
                .map(|(i, _)| fields[*i].trim().parse::<f64>().unwrap_or(f64::NAN))
                .collect();
            records.push((
                fields[ts_idx].trim().to_string(),
                fields[sym_idx].trim().to_string(),
                vals,
            ));
        }
        Panel::from_records(&names, records)
    }

    /// 读取 CSV 文件，对应 Python 侧 `Panel.from_csv`。
    ///
    /// - 入参：`path` CSV 文件路径。
    /// - 加工：整文件读入字符串 → 交给 [`Panel::from_csv_str`] 解析。
    /// - 出参：`Ok(Panel)`；读文件失败或解析失败时返回带路径信息的 `Err`。
    pub fn from_csv<P: AsRef<std::path::Path>>(path: P) -> Result<Panel, String> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| format!("读取 {} 失败：{e}", path.as_ref().display()))?;
        Panel::from_csv_str(&text)
    }

    /// 取出一列作为因子，对应 Python 侧 `panel['close']`。
    ///
    /// - 入参：`column` 列名，如 `"close"` / `"volume"`。
    /// - 加工：查到该列的数值矩阵 → 连同面板的时间戳与标的索引克隆成独立 [`Factor`]。
    /// - 出参：`Ok(Factor)`，其 `name` 即列名，可直接参与后续运算；列不存在时返回 `Err`。
    pub fn factor(&self, column: &str) -> Result<Factor, String> {
        let values = self
            .columns
            .get(column)
            .ok_or_else(|| format!("Column '{column}' not found"))?;
        Factor::new(
            self.timestamps.clone(),
            self.symbols.clone(),
            values.clone(),
            column,
        )
    }

    /// 数值列名（不含 timestamp / symbol），对应 Python 侧 `Panel.columns`。
    ///
    /// - 入参：无。
    /// - 加工：读取内部列表的键。
    /// - 出参：升序排列的列名切片向量。
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.keys().map(String::as_str).collect()
    }

    /// 面板包含的标的列表。
    ///
    /// - 入参：无。
    /// - 加工：直接借用内部索引，不复制。
    /// - 出参：升序去重的标的名切片。
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// 面板包含的时间戳列表。
    ///
    /// - 入参：无。
    /// - 加工：直接借用内部索引，不复制。
    /// - 出参：升序去重的时间戳切片。
    pub fn timestamps(&self) -> &[String] {
        &self.timestamps
    }

    /// 单元格总数（timestamp × symbol），对应 Python 侧 `len(panel)` 的长表行数。
    ///
    /// - 入参：无。
    /// - 加工：期数乘标的数。
    /// - 出参：矩阵单元格个数（含 NaN 占位）。
    pub fn len(&self) -> usize {
        self.timestamps.len() * self.symbols.len()
    }

    /// 面板是否为空。
    ///
    /// - 入参：无。
    /// - 加工：判断 [`Panel::len`] 是否为 0。
    /// - 出参：无任何单元格时为 `true`。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 按时间区间切片（闭区间，字符串字典序比较），对应 Python 侧 `slice_time`。
    ///
    /// - 入参：`start` / `end` 起止时间戳，任一为 `None` 表示该侧不限制。
    /// - 加工：筛出落在区间内的时间下标 → 按下标把每列的对应行段拼接成新矩阵。
    /// - 出参：新 [`Panel`]，标的索引不变、时间索引收窄；无匹配时期数为 0。
    pub fn slice_time(&self, start: Option<&str>, end: Option<&str>) -> Panel {
        let keep: Vec<usize> = self
            .timestamps
            .iter()
            .enumerate()
            .filter(|(_, ts)| {
                start.is_none_or(|s| ts.as_str() >= s) && end.is_none_or(|e| ts.as_str() <= e)
            })
            .map(|(i, _)| i)
            .collect();
        let n = self.symbols.len();
        let columns = self
            .columns
            .iter()
            .map(|(name, vals)| {
                let sliced = keep
                    .iter()
                    .flat_map(|&i| vals[i * n..(i + 1) * n].iter().copied())
                    .collect();
                (name.clone(), sliced)
            })
            .collect();
        Panel {
            timestamps: keep.iter().map(|&i| self.timestamps[i].clone()).collect(),
            symbols: self.symbols.clone(),
            columns,
        }
    }

    /// 按标的过滤，对应 Python 侧 `slice_symbols`。
    ///
    /// - 入参：`symbols` 要保留的标的名列表。
    /// - 加工：筛出命中的列下标 → 逐期按这些下标重新拼列。
    /// - 出参：新 [`Panel`]，时间索引不变、标的索引收窄为命中项（仍保持升序）。
    pub fn slice_symbols(&self, symbols: &[&str]) -> Panel {
        let keep: Vec<usize> = self
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| symbols.contains(&s.as_str()))
            .map(|(i, _)| i)
            .collect();

        let n = self.symbols.len();
        let columns = self
            .columns
            .iter()
            .map(|(name, vals)| {
                let mut out = Vec::with_capacity(self.timestamps.len() * keep.len());
                for ti in 0..self.timestamps.len() {
                    for &sj in &keep {
                        out.push(vals[ti * n + sj]);
                    }
                }
                (name.clone(), out)
            })
            .collect();
        Panel {
            timestamps: self.timestamps.clone(),
            symbols: keep.iter().map(|&i| self.symbols[i].clone()).collect(),
            columns,
        }
    }

    /// 概要信息，对应 Python 侧 `Panel.info()`。
    ///
    /// - 入参：无。
    /// - 加工：汇总行数、列数、标的数、期数、时间范围，并逐列统计 NaN 个数。
    /// - 出参：多行可打印字符串（不直接输出，由调用方决定去向）。
    pub fn info(&self) -> String {
        let range = match (self.timestamps.first(), self.timestamps.last()) {
            (Some(a), Some(b)) => format!("{a} to {b}"),
            _ => "empty".to_string(),
        };
        let nans: Vec<String> = self
            .columns
            .iter()
            .map(|(name, vals)| format!("{name}: {}", vals.len() - count_valid(vals)))
            .collect();
        format!(
            "Panel: {} rows, {} columns\n  symbols={}, periods={}, range={}\n  NaN: {{{}}}",
            self.len(),
            self.columns.len(),
            self.symbols.len(),
            self.timestamps.len(),
            range,
            nans.join(", ")
        )
    }
}

impl fmt::Display for Panel {
    /// - 入参：`f` 格式化器。
    /// - 加工：拼接行数、列数、标的数、期数与时间范围成单行摘要。
    /// - 出参：形如 `Panel(5742 rows, 5 cols, 6 symbols, 957 periods, ...)` 的一行文本。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let range = match (self.timestamps.first(), self.timestamps.last()) {
            (Some(a), Some(b)) => format!("{a} to {b}"),
            _ => "empty".to_string(),
        };

        write!(
            f,
            "Panel({} rows, {} cols, {} symbols, {} periods, {})",
            self.len(),
            self.columns.len(),
            self.symbols.len(),
            self.timestamps.len(),
            range
        )
    }
}
