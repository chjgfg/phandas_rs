//! Factor：因子矩阵，对应 Python 侧 `core.Factor`。
//!
//! 本文件只放矩阵本体、[`Operand`]，以及全部运算子共用的内部机制
//! （`like` / `map_*` / `rolling*` / `align` / `reindex` / `combine` / `infix`）。
//! 具体运算子按家族分散在同目录的 `cs` / `neutralize` / `group` / `ts` / `arith` 中，
//! 它们都是本类型的 `impl` 块，故这些内部机制标为 `pub(crate)`。

use std::collections::{BTreeMap, BTreeSet};

use super::numeric::{count_valid, fmt_num, has_nan, nanmax, nanmean, nanmin, nanstd};

/// 因子矩阵：行 = timestamp（升序），列 = symbol（升序），缺失值为 NaN。
#[derive(Debug, Clone)]
pub struct Factor {
    /// 因子名，随运算自动派生，对应 Python 侧 `Factor.name`。
    pub name: String,
    /// 时间索引，升序去重；长度即矩阵行数。
    pub(crate) timestamps: Vec<String>,
    /// 标的索引，升序去重；长度即矩阵列数。
    pub(crate) symbols: Vec<String>,
    /// 行主序矩阵值，长度 = `timestamps.len() * symbols.len()`，缺失为 NaN。
    pub(crate) values: Vec<f64>,
}

/// 二元运算的右操作数，对应 Python 侧 `Union['Factor', float]`。
///
/// 由 `impl Into<Operand>` 自动转换，故 `close.add(&volume)` 与 `close.add(2.0)` 都可直接写。
#[derive(Debug, Clone, Copy)]
pub enum Operand<'a> {
    /// 另一个因子：参与运算前会按 (timestamp, symbol) 取交集对齐。
    Factor(&'a Factor),
    /// 标量：广播到每个单元格。
    Scalar(f64),
}

impl<'a> From<&'a Factor> for Operand<'a> {
    /// - 入参：`f` 因子引用。
    /// - 加工：包装成 [`Operand::Factor`]，不复制数据。
    /// - 出参：可传给任意二元运算方法的操作数。
    fn from(f: &'a Factor) -> Self {
        Operand::Factor(f)
    }
}

impl From<f64> for Operand<'_> {
    /// - 入参：`v` 标量。
    /// - 加工：包装成 [`Operand::Scalar`]。
    /// - 出参：可传给任意二元运算方法的操作数，运算时广播到每个单元格。
    fn from(v: f64) -> Self {
        Operand::Scalar(v)
    }
}

impl Operand<'_> {
    /// 渲染为因子名中的一段文本。
    ///
    /// - 入参：无（取自身）。
    /// - 加工：因子操作数取其 `name`；标量操作数走 [`fmt_num`]。
    /// - 出参：拼接因子名时使用的字符串。
    fn label(&self) -> String {
        match self {
            Operand::Factor(f) => f.name.clone(),
            Operand::Scalar(v) => fmt_num(*v),
        }
    }
}

impl Factor {
    /// 由完整矩阵构建。`values` 为行主序，长度须等于 `timestamps.len() * symbols.len()`。
    ///
    /// - 入参：`timestamps` 时间索引（须升序去重）；`symbols` 标的索引（须升序去重）；
    ///   `values` 行主序矩阵值；`name` 因子名。
    /// - 加工：仅校验长度是否与 `T × N` 匹配，不重排、不复制。
    /// - 出参：`Ok(Factor)`；长度不匹配时返回说明维度的 `Err`。
    pub fn new(
        timestamps: Vec<String>,
        symbols: Vec<String>,
        values: Vec<f64>,
        name: &str,
    ) -> Result<Factor, String> {
        if values.len() != timestamps.len() * symbols.len() {
            return Err(format!(
                "values 长度 {} 与 {}×{} 不匹配",
                values.len(),
                timestamps.len(),
                symbols.len()
            ));
        }
        Ok(Factor {
            name: name.to_string(),
            timestamps,
            symbols,
            values,
        })
    }

    /// 由 `(timestamp, symbol, value)` 记录构建，缺失组合补 NaN。
    ///
    /// - 入参：`records` 长表记录；`name` 因子名。
    /// - 加工：收集时间戳与标的的去重升序集合 → 分配全 NaN 矩阵 → 逐条写入对应单元格。
    ///   同一 (时间, 标的) 出现多次时后写入者生效。
    /// - 出参：[`Factor`]，形状为 `去重期数 × 去重标的数`。
    pub fn from_records(records: Vec<(String, String, f64)>, name: &str) -> Factor {
        let ts_set: BTreeSet<String> = records.iter().map(|(t, _, _)| t.clone()).collect();
        let sym_set: BTreeSet<String> = records.iter().map(|(_, s, _)| s.clone()).collect();
        let timestamps: Vec<String> = ts_set.into_iter().collect();
        let symbols: Vec<String> = sym_set.into_iter().collect();
        let ts_pos: BTreeMap<&str, usize> =
            timestamps.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        let sym_pos: BTreeMap<&str, usize> =
            symbols.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        let mut values = vec![f64::NAN; timestamps.len() * symbols.len()];
        for (ts, sym, v) in &records {
            values[ts_pos[ts.as_str()] * symbols.len() + sym_pos[sym.as_str()]] = *v;
        }
        Factor {
            name: name.to_string(),
            timestamps,
            symbols,
            values,
        }
    }

    /// 时间期数（矩阵行数）。
    ///
    /// - 入参：无。
    /// - 加工：取时间索引长度。
    /// - 出参：期数。
    pub fn n_periods(&self) -> usize {
        self.timestamps.len()
    }

    /// 标的个数（矩阵列数）。
    ///
    /// - 入参：无。
    /// - 加工：取标的索引长度。
    /// - 出参：标的数。
    pub fn n_symbols(&self) -> usize {
        self.symbols.len()
    }

    /// 因子的时间索引。
    ///
    /// - 入参：无。
    /// - 加工：直接借用，不复制。
    /// - 出参：升序时间戳切片。
    pub fn timestamps(&self) -> &[String] {
        &self.timestamps
    }

    /// 因子的标的索引。
    ///
    /// - 入参：无。
    /// - 加工：直接借用，不复制。
    /// - 出参：升序标的名切片。
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// 行主序原始值。
    ///
    /// - 入参：无。
    /// - 加工：直接借用底层缓冲区。
    /// - 出参：长度为 `期数 × 标的数` 的切片，第 `ti` 期第 `si` 个标的位于下标 `ti * N + si`。
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    /// 按下标取值；越界返回 NaN。
    ///
    /// - 入参：`ti` 期下标；`si` 标的下标。
    /// - 加工：换算成行主序偏移 `ti * N + si` 后读取。
    /// - 出参：该单元格的值；任一下标越界时返回 NaN 而非 panic。
    pub fn at(&self, ti: usize, si: usize) -> f64 {
        if ti >= self.timestamps.len() || si >= self.symbols.len() {
            return f64::NAN;
        }
        self.values[ti * self.symbols.len() + si]
    }

    /// 按 (timestamp, symbol) 取值。
    ///
    /// - 入参：`ts` 时间戳；`symbol` 标的名。
    /// - 加工：在两个索引中线性查找下标后读取单元格。
    /// - 出参：`Some(值)`（值本身仍可能是 NaN）；时间戳或标的不在索引中时返回 `None`。
    pub fn get(&self, ts: &str, symbol: &str) -> Option<f64> {
        let ti = self.timestamps.iter().position(|t| t == ts)?;
        let si = self.symbols.iter().position(|s| s == symbol)?;
        Some(self.values[ti * self.symbols.len() + si])
    }

    /// 某一期的横截面切片。
    ///
    /// - 入参：`ti` 期下标。
    /// - 加工：借用行主序缓冲区中该期对应的连续区间。
    /// - 出参：长度为标的数的切片，顺序与 [`Factor::symbols`] 一致；下标越界会 panic。
    pub fn row(&self, ti: usize) -> &[f64] {
        let n = self.symbols.len();
        &self.values[ti * n..(ti + 1) * n]
    }

    /// 某一标的的完整时间序列（按时间升序）。
    ///
    /// - 入参：`si` 标的下标。
    /// - 加工：按步长 `N` 跨行收集该列的值（矩阵是行主序，列不连续，故需复制）。
    /// - 出参：长度为期数的新向量。
    pub fn series(&self, si: usize) -> Vec<f64> {
        let n = self.symbols.len();
        (0..self.timestamps.len()).map(|ti| self.values[ti * n + si]).collect()
    }

    /// 展开为长表记录，对应 Python 侧 `to_df()` 的 (timestamp, symbol, factor) 结构。
    ///
    /// - 入参：无。
    /// - 加工：按"先时间、后标的"的顺序遍历全部单元格，逐个组装三元组（NaN 也照样输出）。
    /// - 出参：长度为 `期数 × 标的数` 的记录向量。
    pub fn to_records(&self) -> Vec<(String, String, f64)> {
        let n = self.symbols.len();
        let mut out = Vec::with_capacity(self.values.len());

        for (ti, ts) in self.timestamps.iter().enumerate() {
            for (si, sym) in self.symbols.iter().enumerate() {
                out.push((ts.clone(), sym.clone(), self.values[ti * n + si]));
            }
        }
        out
    }

    /// 重命名并返回自身，便于链式书写：`f.rename("my alpha")`。
    ///
    /// - 入参：`name` 新因子名。
    /// - 加工：仅替换 `name` 字段，数值与索引不动。
    /// - 出参：改名后的自身（消耗所有权，避免多余克隆）。
    pub fn rename(mut self, name: &str) -> Factor {
        self.name = name.to_string();
        self
    }

    /// 导出 CSV 文本，对应 Python 侧 `to_csv`。
    ///
    /// - 入参：无。
    /// - 加工：写入表头 `timestamp,symbol,factor` → 逐条展开长表记录，NaN 写成空字段。
    /// - 出参：完整 CSV 字符串（不落盘，落盘用 [`super::to_csv`]）。
    pub fn to_csv_string(&self) -> String {
        let mut s = String::from("timestamp,symbol,factor\n");
        for (ts, sym, v) in self.to_records() {
            if v.is_nan() {
                s.push_str(&format!("{ts},{sym},\n"));
            } else {
                s.push_str(&format!("{ts},{sym},{v}\n"));
            }
        }
        s
    }

    /// 指定期（默认最新一期）的权重映射，对应 Python 侧 `to_weights`。
    ///
    /// - 入参：`ts` 目标时间戳；传 `None` 表示取最后一期。
    /// - 加工：定位该期的行 → 逐标的取值 → **丢弃 NaN**（无持仓视为不入表）。
    /// - 出参：`标的 → 权重` 映射；时间戳不存在或因子为空时返回空映射。
    pub fn to_weights(&self, ts: Option<&str>) -> BTreeMap<String, f64> {
        let ti = match ts {
            Some(t) => self.timestamps.iter().position(|x| x == t),
            None => self.timestamps.len().checked_sub(1),
        };
        let mut out = BTreeMap::new();
        if let Some(ti) = ti {
            for (si, sym) in self.symbols.iter().enumerate() {
                let v = self.at(ti, si);
                if !v.is_nan() {
                    out.insert(sym.clone(), v);
                }
            }
        }
        out
    }

    /// 概要信息，对应 Python 侧 `Factor.info()`。
    ///
    /// - 入参：无。
    /// - 加工：统计单元格总数与有效数、期数、标的数、时间范围，并对全部有效值求
    ///   均值 / 标准差 / 最小 / 最大。
    /// - 出参：多行可打印字符串。
    pub fn info(&self) -> String {
        let valid = count_valid(&self.values);

        let range = match (self.timestamps.first(), self.timestamps.last()) {
            (Some(a), Some(b)) => format!("{a} to {b}"),
            _ => "empty".to_string(),
        };
        format!(
            "Factor '{}': {} cells ({} valid, {} NaN)\n  symbols={}, periods={}, range={}\n  \
             mean={:.6}, std={:.6}, min={:.6}, max={:.6}",
            self.name,
            self.values.len(),
            valid,
            self.values.len() - valid,
            self.symbols.len(),
            self.timestamps.len(),
            range,
            nanmean(&self.values),
            nanstd(&self.values, 1),
            nanmin(&self.values),
            nanmax(&self.values),
        )
    }

    // ---- 内部通用机制：整个模块的运算子都建立在下面这几个方法之上 ----

    /// 以相同索引承载新值。
    ///
    /// - 入参：`values` 新的行主序矩阵值（长度须与自身一致）；`name` 新因子名。
    /// - 加工：克隆自身的时间与标的索引，套上新值。
    /// - 出参：形状不变、取值替换后的新 [`Factor`]。
    pub(crate) fn like(&self, values: Vec<f64>, name: String) -> Factor {
        Factor {
            name,
            timestamps: self.timestamps.clone(),
            symbols: self.symbols.clone(),
            values,
        }
    }

    /// 逐元素映射。
    ///
    /// - 入参：`name` 结果因子名；`f` 单值变换闭包。
    /// - 加工：对每个单元格独立调用 `f`（NaN 也会传入，由闭包自行决定如何处理）。
    /// - 出参：形状与索引不变的新因子。
    pub(crate) fn map_values(&self, name: String, f: impl Fn(f64) -> f64) -> Factor {
        self.like(self.values.iter().copied().map(f).collect(), name)
    }

    /// 逐行（横截面）映射，对应 Python 侧 `groupby('timestamp')`。
    /// 闭包接收当期全部标的值，须返回等长结果。
    ///
    /// - 入参：`name` 结果因子名；`f` 接收一期横截面、返回同长度结果的闭包。
    /// - 加工：按期切出连续行 → 交给 `f` → 结果顺序拼接成新矩阵。
    /// - 出参：形状与索引不变的新因子。debug 构建下会断言 `f` 的返回长度。
    pub(crate) fn map_rows(&self, name: String, f: impl Fn(&[f64]) -> Vec<f64>) -> Factor {
        let n = self.symbols.len();
        let mut out = Vec::with_capacity(self.values.len());
        for ti in 0..self.timestamps.len() {
            let row = f(&self.values[ti * n..(ti + 1) * n]);
            debug_assert_eq!(row.len(), n, "横截面运算须返回等长结果");
            out.extend(row);
        }
        self.like(out, name)
    }

    /// 逐列（单标的时间序列）映射，对应 Python 侧 `groupby('symbol')`。
    /// 闭包接收该标的按时间升序的完整序列，须返回等长结果。
    ///
    /// - 入参：`name` 结果因子名；`f` 接收一条完整时间序列、返回同长度结果的闭包。
    /// - 加工：逐标的抽出列（跨行复制）→ 交给 `f` → 按步长写回新矩阵。
    /// - 出参：形状与索引不变的新因子。debug 构建下会断言 `f` 的返回长度。
    pub(crate) fn map_series(&self, name: String, f: impl Fn(&[f64]) -> Vec<f64>) -> Factor {
        let n = self.symbols.len();
        let t = self.timestamps.len();
        let mut out = vec![f64::NAN; self.values.len()];
        for si in 0..n {
            let col = self.series(si);
            let res = f(&col);
            debug_assert_eq!(res.len(), t, "时序运算须返回等长结果");
            for (ti, v) in res.into_iter().enumerate() {
                out[ti * n + si] = v;
            }
        }
        self.like(out, name)
    }

    /// 滚动窗口，语义对齐 pandas `rolling(window, min_periods).apply(...)`：
    /// 起始若干期为不足窗口的部分窗口；窗口内非 NaN 数量小于 `min_periods` 时直接输出 NaN。
    ///
    /// - 入参：`window` 窗口期数；`min_periods` 窗口内所需的最少有效值个数；
    ///   `name` 结果因子名；`f` 对单个窗口求一个标量的闭包。
    /// - 加工：逐标的沿时间推进，第 `i` 期取 `[i - window + 1, i]`（前期不足则截断）
    ///   → 先数有效值，不达 `min_periods` 直接给 NaN 且**不调用** `f` → 否则调用 `f`。
    /// - 出参：形状与索引不变的新因子。
    pub(crate) fn rolling(
        &self,
        window: usize,
        min_periods: usize,
        name: String,
        f: impl Fn(&[f64]) -> f64,
    ) -> Factor {
        self.map_series(name, |col| {
            (0..col.len())
                .map(|i| {
                    let start = (i + 1).saturating_sub(window);
                    let w = &col[start..=i];
                    if count_valid(w) < min_periods {
                        f64::NAN
                    } else {
                        f(w)
                    }
                })
                .collect()
        })
    }

    /// 完整窗口滚动：窗口必须填满且不含 NaN，否则输出 NaN。
    /// 对应 Python 侧 `_apply_rolling` + `min_periods=window` 的组合语义。
    ///
    /// - 入参：`window` 窗口期数；`name` 结果因子名；`f` 对单个干净窗口求标量的闭包。
    /// - 加工：以 `min_periods = window` 调用 [`Factor::rolling`]，并额外再挡一层
    ///   "窗口未填满或含 NaN"的情况。
    /// - 出参：形状与索引不变的新因子；前 `window - 1` 期必为 NaN。
    pub(crate) fn rolling_full(&self, window: usize, name: String, f: impl Fn(&[f64]) -> f64) -> Factor {
        self.rolling(window, window, name, move |w| {
            if w.len() < window || has_nan(w) {
                f64::NAN
            } else {
                f(w)
            }
        })
    }

    /// 与另一因子按 (timestamp, symbol) 取交集对齐，对应 Python 侧 `pd.merge(how='inner')`。
    /// 返回共同索引与两侧重排后的值。
    ///
    /// - 入参：`other` 另一因子。
    /// - 加工：索引完全相同时走快路径直接克隆；否则求时间戳与标的的交集（保持自身的升序）
    ///   → 两侧各自 [`Factor::reindex`] 到该交集。
    /// - 出参：`(共同时间戳, 共同标的, 自身重排值, 对方重排值)`；无交集时索引与值均为空。
    pub(crate) fn align(&self, other: &Factor) -> (Vec<String>, Vec<String>, Vec<f64>, Vec<f64>) {
        if self.timestamps == other.timestamps && self.symbols == other.symbols {
            return (
                self.timestamps.clone(),
                self.symbols.clone(),
                self.values.clone(),
                other.values.clone(),
            );
        }
        let ts_other: BTreeSet<&str> = other.timestamps.iter().map(String::as_str).collect();
        let sym_other: BTreeSet<&str> = other.symbols.iter().map(String::as_str).collect();
        let timestamps: Vec<String> = self
            .timestamps
            .iter()
            .filter(|t| ts_other.contains(t.as_str()))
            .cloned()
            .collect();
        let symbols: Vec<String> = self
            .symbols
            .iter()
            .filter(|s| sym_other.contains(s.as_str()))
            .cloned()
            .collect();
        let a = self.reindex(&timestamps, &symbols);
        let b = other.reindex(&timestamps, &symbols);
        (timestamps, symbols, a, b)
    }

    /// 按给定索引重排取值，缺失组合补 NaN。
    ///
    /// - 入参：`timestamps` 目标时间索引；`symbols` 目标标的索引。
    /// - 加工：建立自身索引的查找表 → 按目标索引逐格取值，查不到就填 NaN。
    /// - 出参：长度为 `目标期数 × 目标标的数` 的行主序值向量。
    pub(crate) fn reindex(&self, timestamps: &[String], symbols: &[String]) -> Vec<f64> {
        let ts_pos: BTreeMap<&str, usize> =
            self.timestamps.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        let sym_pos: BTreeMap<&str, usize> =
            self.symbols.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        let n = self.symbols.len();
        let mut out = Vec::with_capacity(timestamps.len() * symbols.len());
        for ts in timestamps {
            for sym in symbols {
                out.push(match (ts_pos.get(ts.as_str()), sym_pos.get(sym.as_str())) {
                    (Some(&ti), Some(&si)) => self.values[ti * n + si],
                    _ => f64::NAN,
                });
            }
        }
        out
    }

    /// 二元运算通用实现：对齐后逐元素计算，并按 `name_of` 生成因子名。
    ///
    /// - 入参：`other` 右操作数（因子或标量）；`name_of` 由右操作数名生成结果名的闭包；
    ///   `f` 二元数值运算闭包。
    /// - 加工：右操作数是因子时先 [`Factor::align`] 取交集再逐格计算；是标量时直接
    ///   把标量喂给每个单元格。
    /// - 出参：新因子——因子对因子的结果形状为交集大小，因子对标量的结果形状不变。
    pub(crate) fn combine<'a>(
        &self,
        other: impl Into<Operand<'a>>,
        name_of: impl FnOnce(&str) -> String,
        f: impl Fn(f64, f64) -> f64,
    ) -> Factor {
        let operand = other.into();
        let name = name_of(&operand.label());
        match operand {
            Operand::Factor(o) => {
                let (timestamps, symbols, a, b) = self.align(o);
                let values = a.iter().zip(b.iter()).map(|(x, y)| f(*x, *y)).collect();
                Factor { name, timestamps, symbols, values }
            }
            Operand::Scalar(v) => self.map_values(name, |x| f(x, v)),
        }
    }

    /// 中缀运算：生成 `(左名 op 右名)` 形式的因子名。
    ///
    /// - 入参：`other` 右操作数；`op` 中缀符号文本；`f` 二元数值运算闭包。
    /// - 加工：套用 [`Factor::combine`]，仅固定命名格式。
    /// - 出参：新因子，名形如 `(close-open)`。
    pub(crate) fn infix<'a>(
        &self,
        other: impl Into<Operand<'a>>,
        op: &str,
        f: impl Fn(f64, f64) -> f64,
    ) -> Factor {
        let me = self.name.clone();
        self.combine(other, |rhs| format!("({me}{op}{rhs})"), f)
    }
}
