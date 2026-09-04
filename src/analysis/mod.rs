//! 因子评价模块：Python 版 phandas 中 `analysis.py` 的 Rust 实现。
//!
//! # 设计
//!
//! - 传入一个**价格因子**与一个或多个**候选因子**，用价格因子造前瞻收益
//!   （`price.shift(-h) / price - 1`，持有期为 `h`），逐期算横截面 IC——
//!   上游对 Spearman 用名次、其余口径都用原值。本仓库只支持 `spearman` / `pearson`，
//!   **未知口径直接报错**，不复刻上游"除 `'spearman'` 外一律按 Pearson"的静默降级。
//! - 逐因子输出三组描述统计：`coverage`（非 NaN 占比）、`turnover`（横截面百分位名次的
//!   逐期变动幅度——先按标的取均值、再对标的取均值，×2）、`autocorr`（各标的滞后 1
//!   自相关，样本须大于 10 期）。
//! - 上游把每个因子的信号因子 `signal()` 透视成 `(timestamp, symbol)` 网格后，
//!   用 `pd.DataFrame(...).dropna()` 求全因子共同样本的**相关矩阵**；本仓库按
//!   (timestamp, symbol) 交集对齐后保留全部因子都非 NaN 的格子，口径一致。
//!
//! # 与 Python 版的口径对照
//!
//! - `ic_mean` / `ic_std` / `ir` / `t_stat` 逐格复刻上游 `np.nanmean` / `np.nanstd`，
//!   其中 **`ic_std` 是 `ddof = 0`**（上游用 numpy，不是 pandas 的 `ddof = 1`）。
//! - [`ic_series`] 如实复刻上游分母的错位：分子只累加两侧同时非 NaN 的格子，
//!   分母却各自累加单侧所有非 NaN 格子，两侧 NaN 分布不同时 `|IC|` 会被压低、取值
//!   不一定落在 `[-1, 1]`——这是上游 `core.py` 的实现细节，此处不做修正。
//! - `t_stat = ic_mean / (ic_std / √n)` 等于 `ir × √n`；`ic_std` 非正时上游给 `0.0`，
//!   本仓库同。
//! - 上游 `stats()` 的 `autocorr` 只在 `len(series) > 10` 时计入，pandas `.autocorr`
//!   剔除 NaN 后按 `(x[t], x[t+1])` 样本对计算，本仓库同。
//! - 上游 `stats()` 的 `turnover = rank_diff.mean().mean() * 2`——`DataFrame.mean()` 默认
//!   `axis = 0`，故这是"每个标的先取一个均值（跳 NaN），再对标的取均值"的**两级平均**，
//!   不是把所有格子池化求一个均值；各标的的有效差分个数不齐时两者并不相等。只有 0 期或
//!   0 标的（上游 `rank_diff.empty` 为真）时才给 `0.0`，其余算不出来的情况是 NaN
//!   （例如只有 1 期、或差分整块全 NaN），本仓库同。
//! - [`FactorAnalyzer::correlation`] 的对角线：pandas 的 `method='kendall'` 不走
//!   `nancorr` 而走 `nanops` 的逐对循环，把 `i == j` **硬编码成 `1.0`**；Pearson /
//!   Spearman 走 `nancorr`，常量列的对角线是 NaN。本仓库同——[`corr`] 本身仍按 scipy 给
//!   NaN，这个 `1.0` 只在 `correlation()` 这一层补。
//! - 上游三个输出区（IC / IR / 统计）排版为 18 字符因子名列 + 12 字符数值列，
//!   `summary()` 返回字符串而非打印，与 [`crate::factor::Factor::info`] /
//!   [`crate::backtest::Backtester::summary`] 风格一致。`print_summary` 与 `_*_cache`
//!   字段未移植——统计量开销很小，本仓库不缓存；前瞻收益只在单次 [`FactorAnalyzer::ic`]
//!   调用内按持有期算一次，跨因子共享（对应上游 `_compute_forward_returns`）。
//!
//! # 未复刻的上游细节
//!
//! - 上游 `correlation()` 用 `f.signal()` 作为比对对象，`signal()` 对含 NaN 的期输出整期
//!   NaN——本仓库保留这一语义，于是含 NaN 的整期会从相关样本里整行消失。
//! - 因子数不足 2 个或共同样本不足 2 格时，上游发 `warnings.warn` 并返回空 `DataFrame`；
//!   本仓库返回空 [`CorrMatrix`]，不警告。

pub mod corr;
pub mod ic;

use std::collections::BTreeSet;
use std::fmt;

use crate::backtest::stats::pearson_r;
use crate::factor::numeric::{count_valid, nanmean, rank_pct, RankMethod};
use crate::factor::Factor;

pub use self::corr::{corr, CorrMatrix, CorrMethod};
pub use self::ic::{ic_series, ic_stats, IcMethod, IcStats};

/// 默认持有期，对应上游 `_DEFAULT_HORIZONS = [1, 7, 30]`。
pub const DEFAULT_HORIZONS: [usize; 3] = [1, 7, 30];

/// 单个因子的 IC 结果，对应上游 `ic()` 字典里 `results[factor_name]` 那个内层字典。
#[derive(Debug, Clone)]
pub struct FactorIc {
    /// 因子在输入列表中的下标。
    pub factor_index: usize,
    /// 各持有期的 IC 统计，顺序与 [`FactorAnalyzer::horizons`] 一致。
    pub by_horizon: Vec<IcStats>,
}

impl FactorIc {
    /// 按持有期取 IC 统计。
    ///
    /// - 入参：`horizon` 持有期。
    /// - 加工：在 `by_horizon` 里线性查找持有期匹配的项。
    /// - 出参：`Some(&IcStats)`；未配置该持有期时返回 `None`。
    pub fn for_horizon(&self, horizon: usize) -> Option<&IcStats> {
        self.by_horizon.iter().find(|s| s.horizon == horizon)
    }
}

/// 单个因子的描述统计，对应上游 `stats()` 字典里一个因子的三个键。
#[derive(Debug, Clone, PartialEq)]
pub struct FactorStats {
    /// 因子在输入列表中的下标。
    pub factor_index: usize,
    /// 非 NaN 单元格占比。
    pub coverage: f64,
    /// 横截面百分位名次逐期变动幅度的两级均值 × 2（先按标的、再对标的）；0 期或 0 标的时
    /// 为 `0.0`，其余算不出来（如只有 1 期）时为 NaN。
    pub turnover: f64,
    /// 各标的滞后 1 自相关的均值；无任何标的满足样本条件时为 NaN。
    pub autocorr: f64,
}

/// 多因子分析与评价，对应上游 `analysis.FactorAnalyzer`。
///
/// 持引用到价格因子与候选因子，不拷贝数据。`ic` / `stats` / `correlation` 都是纯函数，
/// 每次调用按需重算。
pub struct FactorAnalyzer<'a> {
    /// 候选因子，顺序即输出顺序。
    factors: Vec<&'a Factor>,
    /// 价格因子，用于造前瞻收益。
    price: &'a Factor,
    /// 持有期序列。
    horizons: Vec<usize>,
}

impl<'a> FactorAnalyzer<'a> {
    /// 构造因子分析器。
    ///
    /// - 入参：`factors` 候选因子；`price` 价格因子；`horizons` 持有期，`None` **或空表**
    ///   时用 [`DEFAULT_HORIZONS`]（上游 `horizons or _DEFAULT_HORIZONS` 里空 list 是
    ///   falsy，同样回落到默认值）。
    /// - 加工：只做空表校验，不做计算。
    /// - 出参：`Ok(FactorAnalyzer)`；空因子表返回说明原因的 `Err`
    ///   （上游对 `analyze([], price)` 抛 `ValueError`）。
    pub fn new(
        factors: &[&'a Factor],
        price: &'a Factor,
        horizons: Option<&[usize]>,
    ) -> Result<FactorAnalyzer<'a>, String> {
        if factors.is_empty() {
            return Err("Must provide at least one factor".to_string());
        }
        let horizons = match horizons {
            Some(h) if !h.is_empty() => h.to_vec(),
            _ => DEFAULT_HORIZONS.to_vec(),
        };
        Ok(FactorAnalyzer {
            factors: factors.to_vec(),
            price,
            horizons,
        })
    }

    /// 候选因子。
    pub fn factors(&self) -> &[&'a Factor] {
        &self.factors
    }

    /// 价格因子。
    pub fn price(&self) -> &'a Factor {
        self.price
    }

    /// 持有期序列。
    pub fn horizons(&self) -> &[usize] {
        &self.horizons
    }

    /// 前瞻收益因子。持有期 `h` 的收益定义与上游一致：`price.shift(-h) / price - 1`，
    /// 末尾不足 `h` 期与价格缺失处都是 NaN。
    ///
    /// - 入参：`horizon` 持有期。
    /// - 加工：对每个 (timestamp, symbol) 求 `price[t+h] / price[t] - 1`；
    ///   越界或两侧任一缺失时输出 NaN。
    /// - 出参：与价格因子同索引同形状的收益因子，名为 `fwd_{horizon}d`。
    fn forward_returns(&self, horizon: usize) -> Factor {
        let name = format!("fwd_{horizon}d");
        let t = self.price.n_periods();
        let n = self.price.n_symbols();
        let values: Vec<f64> = (0..t)
            .flat_map(|ti| {
                (0..n).map(move |si| {
                    let p0 = self.price.at(ti, si);
                    let p1 = self.price.at(ti + horizon, si);
                    if p0.is_nan() || p1.is_nan() {
                        f64::NAN
                    } else {
                        p1 / p0 - 1.0
                    }
                })
            })
            .collect();
        // 形状必然与价格一致，unwrap 只作形状不变的断言
        Factor::new(
            self.price.timestamps().to_vec(),
            self.price.symbols().to_vec(),
            values,
            &name,
        )
        .expect("forward 因子形状与价格因子一致")
    }

    /// 逐期横截面 IC 统计，对应上游 `ic()`。
    ///
    /// - 入参：`method` IC 口径（`spearman` / `pearson`）。
    /// - 加工：先按持有期各造一份前瞻收益（只依赖价格，故跨因子共享，对应上游
    ///   `_compute_forward_returns`），再对每个因子 × 每个持有期算逐期 IC（见
    ///   [`ic_series`]），汇成均值 / 标准差 / IR / t 值。
    /// - 出参：按因子顺序排列的 [`FactorIc`] 列表，每个因子内按 [`FactorAnalyzer::horizons`]
    ///   顺序给出各持有期统计。
    pub fn ic(&self, method: IcMethod) -> Vec<FactorIc> {
        let fwd: Vec<Factor> = self
            .horizons
            .iter()
            .map(|&h| self.forward_returns(h))
            .collect();
        self.factors
            .iter()
            .enumerate()
            .map(|(i, f)| FactorIc {
                factor_index: i,
                by_horizon: self
                    .horizons
                    .iter()
                    .zip(fwd.iter())
                    .map(|(&h, r)| ic_stats(f, r, h, method))
                    .collect(),
            })
            .collect()
    }

    /// 相关矩阵，对应上游 `correlation()`。
    ///
    /// - 入参：`method` 相关系数口径（pearson / spearman / kendall）。
    /// - 加工：逐因子取 `signal()` → 在 (timestamp, symbol) 网格上取交集 → 只保留全部因子
    ///   都非 NaN 的格子作样本 → 两两算相关系数。相关系数对称，故只算上三角再镜像
    ///   （两侧入参互换在 IEEE 下逐位同值）；Kendall 口径的对角线按 pandas 直接置 `1.0`。
    ///   样本不足 2 格或因子不足 2 个时返回空矩阵。
    /// - 出参：[`CorrMatrix`]，`n_obs` 为共同有效样本格数。
    pub fn correlation(&self, method: CorrMethod) -> CorrMatrix {
        if self.factors.len() < 2 {
            return CorrMatrix::empty(method);
        }
        let signals: Vec<Factor> = self.factors.iter().map(|f| f.signal()).collect();
        let grids = match common_grid(&signals) {
            Some(g) => g,
            None => return CorrMatrix::empty(method),
        };
        let n_cells = grids[0].len();

        // 逐格判断：任一会 NaN 的格子从样本中剔除（对应上游 stack().dropna()）
        let keep: Vec<bool> = (0..n_cells)
            .map(|k| grids.iter().all(|g| !g[k].is_nan()))
            .collect();
        let n_obs = keep.iter().filter(|&&b| b).count();
        if n_obs < 2 {
            return CorrMatrix::empty(method);
        }

        // 每因子一个列向量（只含有效样本格）
        let cols: Vec<Vec<f64>> = grids
            .iter()
            .map(|g| (0..n_cells).filter(|&k| keep[k]).map(|k| g[k]).collect())
            .collect();
        let m = self.factors.len();
        let mut values = vec![f64::NAN; m * m];
        for i in 0..m {
            for j in i..m {
                // pandas 的 kendall 分支在方差检查之前把 i == j 写死为 1.0，
                // 故常量列在该口径下对角线是 1.0 而非 NaN
                let v = if i == j && method == CorrMethod::Kendall {
                    1.0
                } else {
                    corr(&cols[i], &cols[j], method)
                };
                values[i * m + j] = v;
                values[j * m + i] = v;
            }
        }
        CorrMatrix::new(
            self.factors.iter().map(|f| f.name.clone()).collect(),
            values,
            n_obs,
            method,
        )
    }

    /// 各因子的描述统计，对应上游 `stats()`。
    ///
    /// - 入参：无。
    /// - 加工：逐因子算非 NaN 占比、百分位名次换手率、滞后 1 自相关均值（见
    ///   [`factor_stats`]）。
    /// - 出参：按因子顺序排列的 [`FactorStats`]。
    pub fn stats(&self) -> Vec<FactorStats> {
        self.factors
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let (coverage, turnover, autocorr) = factor_stats(f);
                FactorStats {
                    factor_index: i,
                    coverage,
                    turnover,
                    autocorr,
                }
            })
            .collect()
    }

    /// 汇总报告，排版与上游 `summary()` 一致。
    ///
    /// - 入参：无。
    /// - 加工：跑一次 [`FactorAnalyzer::ic`]（Spearman）与 [`FactorAnalyzer::stats`]，
    ///   因子数大于 1 时再取一次 Pearson 相关矩阵；按固定列宽排版成四个区块。
    /// - 出参：多行字符串。与 [`crate::factor::Factor::info`] 一致，不直接打印。
    pub fn summary(&self) -> String {
        let ics = self.ic(IcMethod::Spearman);
        let statss = self.stats();
        let corr_matrix = if self.factors.len() > 1 {
            let cm = self.correlation(CorrMethod::Pearson);
            (!cm.is_empty()).then_some(cm)
        } else {
            None
        };

        let mut lines = Vec::new();
        lines.push(format!(
            "FactorAnalyzer(factors={}, horizons={:?})",
            self.factors.len(),
            self.horizons
        ));

        // 因子名列：名称截断/补齐到 18 字符，前导两个空格；数值列宽 12
        let underline = format!("  {}", "-".repeat(18 + 12 * self.horizons.len()));

        lines.push(String::new());
        lines.push("IC Analysis (Spearman):".to_string());
        // 表头因子名列与数据行同宽：整体 20 字符（“  ”前缀 + 18 字符名称列）
        let mut header = format!("{:<20}", "  Factor");
        for &h in &self.horizons {
            header.push_str(&format!("{:>12}", format!("{h}D")));
        }
        lines.push(header);
        lines.push(underline);
        for (fi, f) in self.factors.iter().enumerate() {
            let mut row = format!("  {:<18}", trunc_chars(&f.name, 18));
            for &h in &self.horizons {
                let v = ics[fi]
                    .for_horizon(h)
                    .map(|s| s.ic_mean)
                    .unwrap_or(f64::NAN);
                row.push_str(&cell_12(v, "N/A", |x| format!("{x:.4}")));
            }
            lines.push(row);
        }

        lines.push(String::new());
        lines.push("IR (IC Mean / IC Std):".to_string());
        // 与上游一致：IR 区不带表头与分隔线，紧跟各因子的 IR 行
        for (fi, f) in self.factors.iter().enumerate() {
            let mut row = format!("  {:<18}", trunc_chars(&f.name, 18));
            for &h in &self.horizons {
                let v = ics[fi].for_horizon(h).map(|s| s.ir).unwrap_or(f64::NAN);
                row.push_str(&cell_12(v, "N/A", |x| format!("{x:.3}")));
            }
            lines.push(row);
        }

        lines.push(String::new());
        lines.push("Factor Statistics:".to_string());
        lines.push(format!(
            "  {:<18}{:>12}{:>12}{:>12}",
            "Factor", "Coverage", "Turnover", "Autocorr"
        ));
        lines.push(format!("  {}", "-".repeat(54)));
        for (fi, f) in self.factors.iter().enumerate() {
            let s = &statss[fi];
            let mut row = format!("  {:<18}", trunc_chars(&f.name, 18));
            row.push_str(&format!("{:>12}", format!("{:.2}%", s.coverage * 100.0)));
            // NaN 走 Python 的 `f"{nan:.4f}"` 文本，即小写 nan，而非 Rust 的 NaN
            row.push_str(&cell_12(s.turnover, "nan", |x| format!("{x:.4}")));
            row.push_str(&cell_12(s.autocorr, "N/A", |x| format!("{x:.4}")));
            lines.push(row);
        }

        if let Some(cm) = &corr_matrix {
            lines.push(String::new());
            lines.push("Correlation Matrix:".to_string());
            for l in cm.to_string_table().lines() {
                lines.push(format!("  {l}"));
            }
        }

        lines.join("\n")
    }
}

impl fmt::Display for FactorAnalyzer<'_> {
    /// - 入参：`f` 格式化器。
    /// - 加工：直接输出 [`FactorAnalyzer`] 的单行描述。
    /// - 出参：形如 `FactorAnalyzer(factors=['alpha1'], horizons=[1, 7, 30])` 的文本。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self
            .factors
            .iter()
            .map(|f| format!("'{}'", f.name.replace('\'', "\\'")))
            .collect();
        write!(
            f,
            "FactorAnalyzer(factors=[{}], horizons={:?})",
            names.join(", "),
            self.horizons
        )
    }
}

/// 便捷构造，对应上游 `analyze(factors, price, horizons=None)`。
///
/// - 入参：`factors` 候选因子；`price` 价格因子；`horizons` 持有期，`None` 或空表用默认
///   `[1, 7, 30]`。
/// - 加工：转发给 [`FactorAnalyzer::new`]。
/// - 出参：`Ok(FactorAnalyzer)`；空因子表返回 `Err`。
pub fn analyze<'a>(
    factors: &[&'a Factor],
    price: &'a Factor,
    horizons: Option<&[usize]>,
) -> Result<FactorAnalyzer<'a>, String> {
    FactorAnalyzer::new(factors, price, horizons)
}

/// 按 Unicode 字符数截断。
fn trunc_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// 数值列单元格：NaN 打占位文本，否则按格式化函数输出；统一右对齐到 12 字符。
fn cell_12(v: f64, nan_txt: &str, fmt: impl Fn(f64) -> String) -> String {
    if v.is_nan() {
        format!("{nan_txt:>12}")
    } else {
        format!("{:>12}", fmt(v))
    }
}

/// 求多个信号因子在共享的 (timestamp, symbol) 网格上的逐因子取值。
///
/// - 入参：`signals` 信号因子（须非空）。
/// - 加工：时间戳与标的分别按自身升序求交集，逐因子重排到该交集；
///   交集为空时返回 `None`。
/// - 出参：各因子的行主序取值，取回后按格拼接即 (timestamp, symbol) 序列。
fn common_grid(signals: &[Factor]) -> Option<Vec<Vec<f64>>> {
    let mut ts: Vec<String> = signals[0].timestamps().to_vec();
    let mut sym: Vec<String> = signals[0].symbols().to_vec();
    for s in &signals[1..] {
        let ts_set: BTreeSet<&str> = s.timestamps().iter().map(String::as_str).collect();
        let sym_set: BTreeSet<&str> = s.symbols().iter().map(String::as_str).collect();
        ts.retain(|t| ts_set.contains(t.as_str()));
        sym.retain(|t| sym_set.contains(t.as_str()));
    }
    if ts.is_empty() || sym.is_empty() {
        return None;
    }
    Some(signals.iter().map(|s| s.reindex(&ts, &sym)).collect())
}

/// 逐因子计算 `coverage` / `turnover` / `autocorr` 三项描述统计。
///
/// - 入参：`f` 因子。
/// - 加工：coverage = 非 NaN 单元格 / 总单元格。turnover 先把每行换成横截面百分位名次
///   （pandas `rank(method='average', pct=True)` 语义），再按标的取逐期差分绝对值的
///   均值（跳 NaN），最后对这些标的均值再取一次均值（同样跳 NaN）并 ×2——上游
///   `rank_diff.mean().mean() * 2` 是两级平均，不是把全部格子池化；只有 0 期或 0 标的
///   时给 `0.0`（上游 `rank_diff.empty`），其余算不出来就是 NaN。autocorr 对各标的剔除
///   NaN 后取滞后 1 自相关（需样本数 > 10），再取均值。
/// - 出参：`(coverage, turnover, autocorr)`，`turnover` / `autocorr` 无有效样本时为 NaN。
#[allow(clippy::needless_range_loop)] // ranked 是"行主序"网格，按列取差分只能跨行索引
fn factor_stats(f: &Factor) -> (f64, f64, f64) {
    let t = f.n_periods();
    let n = f.n_symbols();

    let total = f.values().len();
    let coverage = if total == 0 {
        0.0
    } else {
        count_valid(f.values()) as f64 / total as f64
    };

    // 上游只在 rank_diff 为空（0 期或 0 标的）时给 0，其余走两级平均、算不出来就是 NaN
    let turnover = if t == 0 || n == 0 {
        0.0
    } else {
        let ranked: Vec<Vec<f64>> = (0..t)
            .map(|ti| rank_pct(f.row(ti), RankMethod::Average))
            .collect();
        let per_symbol: Vec<f64> = (0..n)
            .map(|si| {
                let diffs: Vec<f64> = (1..t)
                    .map(|ti| (ranked[ti][si] - ranked[ti - 1][si]).abs())
                    .collect();
                nanmean(&diffs)
            })
            .collect();
        nanmean(&per_symbol) * 2.0
    };

    let mut acs = Vec::new();
    for si in 0..n {
        let series: Vec<f64> = f.series(si).into_iter().filter(|v| !v.is_nan()).collect();
        if series.len() > 10 {
            let r = pearson_r(&series[..series.len() - 1], &series[1..]);
            if !r.is_nan() {
                acs.push(r);
            }
        }
    }
    let autocorr = if acs.is_empty() {
        f64::NAN
    } else {
        nanmean(&acs)
    };

    (coverage, turnover, autocorr)
}
