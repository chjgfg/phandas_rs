//! 相关系数与相关矩阵，对应上游 `analysis.correlation()` 里 `DataFrame.corr(method=...)` 那一步。
//!
//! Pearson 直接复用 [`crate::backtest::stats::pearson_r`]——同一个统计量没有第二种算法，
//! 那个模块本身就是按通用工具公开的。Spearman 先取平均名次再走 Pearson，Kendall 取
//! `scipy.stats.kendalltau` 的 tau-b 口径（带并列修正），与 pandas 的三个选项一一对应。
//!
//! Kendall 照 scipy 的办法用树状数组数反序对，复杂度 `O(n log n)`；`correlation()` 的样本
//! 是整块堆叠面板（`T × N` 格），朴素的 `O(n²)` 枚举在几万格上就要十秒量级，不可用。

use std::fmt;

use crate::backtest::stats::pearson_r;
use crate::factor::numeric::{argsort_stable, has_nan, rank_pct, RankMethod};

/// 相关系数口径，对应 pandas `DataFrame.corr(method=...)` 的三个取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CorrMethod {
    /// 积矩相关系数，pandas 默认，也是上游 `correlation()` 的默认。
    #[default]
    Pearson,
    /// 秩相关：两侧各取平均名次后再算 Pearson。
    Spearman,
    /// Kendall tau-b，带并列修正。
    Kendall,
}

impl CorrMethod {
    /// 由字符串解析，对应上游把 `method` 原样转交 pandas 的三个合法值。
    ///
    /// - 入参：`s` 口径名称。
    /// - 加工：与 `"pearson"` / `"spearman"` / `"kendall"` 精确匹配。
    /// - 出参：匹配成功返回 [`CorrMethod`]；否则返回含可选值清单的错误信息。
    ///   上游把任意字符串直接喂给 pandas，非法值由 pandas 抛 `ValueError`。
    pub fn parse(s: &str) -> Result<CorrMethod, String> {
        match s {
            "pearson" => Ok(CorrMethod::Pearson),
            "spearman" => Ok(CorrMethod::Spearman),
            "kendall" => Ok(CorrMethod::Kendall),
            other => Err(format!(
                "Invalid method: {other}. Must be one of ['pearson', 'spearman', 'kendall']"
            )),
        }
    }

    /// 口径名称的字符串形式，用于报告排版。
    ///
    /// - 入参：无（取自身枚举值）。
    /// - 加工：枚举值到 pandas 同名字符串的映射。
    /// - 出参：`"pearson"` / `"spearman"` / `"kendall"` 之一。
    pub fn label(self) -> &'static str {
        match self {
            CorrMethod::Pearson => "pearson",
            CorrMethod::Spearman => "spearman",
            CorrMethod::Kendall => "kendall",
        }
    }
}

/// 成对相关系数，对应 `pandas.Series.corr(other, method)`。
///
/// - 入参：`x` / `y` 等长的数值切片（调用方通常已剔除 NaN）；`method` 相关系数口径。
/// - 加工：Pearson 走 [`pearson_r`]；Spearman 先把两侧各自换成平均名次百分位
///   （名次整体缩放不改变相关系数，故直接用 [`rank_pct`]）再走 Pearson；
///   Kendall 走 [`kendall_tau_b`]。
/// - 出参：相关系数，落在 `[-1, 1]`；长度不足 2、长度不等、任一侧无波动或**任一侧含 NaN**
///   时返回 NaN（pandas 在这些输入上同样给 NaN）。三个口径遇 NaN 的行为一致：
///   Pearson / Spearman 由 NaN 自然传播，Kendall 在 [`kendall_tau_b`] 里显式挡掉。
pub fn corr(x: &[f64], y: &[f64], method: CorrMethod) -> f64 {
    if x.len() != y.len() || x.len() < 2 {
        return f64::NAN;
    }
    match method {
        CorrMethod::Pearson => pearson_r(x, y),
        CorrMethod::Spearman => pearson_r(
            &rank_pct(x, RankMethod::Average),
            &rank_pct(y, RankMethod::Average),
        ),
        CorrMethod::Kendall => kendall_tau_b(x, y),
    }
}

/// 稠密名次（1 起算、并列同名次）与并列对数 `Σ t(t − 1) / 2`。
///
/// - 入参：`v` 数值切片（须已无 NaN）。
/// - 加工：稳定升序排序后扫描相等区间，逐段分配同一名次并累加该段的并列对数。
/// - 出参：`(逐元素名次, 并列对数)`；名次落在 `[1, 去重个数]`，供树状数组当下标用。
fn dense_ranks(v: &[f64]) -> (Vec<usize>, f64) {
    let order = argsort_stable(v);
    let mut rank = vec![0usize; v.len()];
    let mut ties = 0.0f64;
    let (mut k, mut r) = (0usize, 0usize);
    while k < order.len() {
        let mut j = k + 1;
        while j < order.len() && v[order[j]] == v[order[k]] {
            j += 1;
        }
        r += 1;
        for &i in &order[k..j] {
            rank[i] = r;
        }
        let t = (j - k) as f64;
        ties += t * (t - 1.0) / 2.0;
        k = j;
    }
    (rank, ties)
}

/// 树状数组：在名次 `i` 处 +1。`sup` 为最大名次（数组下标 1 起算）。
fn fenwick_add(tree: &mut [i64], mut i: usize, sup: usize) {
    while i <= sup {
        tree[i] += 1;
        i += i & i.wrapping_neg();
    }
}

/// 树状数组：名次不超过 `i` 的已插入个数。
fn fenwick_sum(tree: &[i64], mut i: usize) -> i64 {
    let mut s = 0i64;
    while i > 0 {
        s += tree[i];
        i &= i - 1;
    }
    s
}

/// Kendall tau-b，对应 `scipy.stats.kendalltau`（pandas 的 `method='kendall'` 就是它）。
///
/// - 入参：`x` / `y` 等长的数值切片。
/// - 加工：照 scipy 的公式走——先把 `y` 换成稠密名次，再按 `(x, y)` 升序扫描；`x` 相等的
///   一段内部不构成同序/反序，故整段先查询、再整段写入树状数组，查询得到的
///   "已插入里名次更大的个数"即反序对数 `dis`。同时扫出 `x` 侧并列对数 `xtie` 与
///   `(x, y)` 同时并列的对数 `ntie`，最后按
///   `con − dis = n₀ − xtie − ytie + ntie − 2·dis`、
///   `tau = (con − dis) / √(n₀ − xtie) / √(n₀ − ytie)` 求值并夹到 `[-1, 1]`
///   （与 scipy 同样的两次开方与夹取，避免浮点误差溢出值域）。复杂度 `O(n log n)`。
/// - 出参：tau-b，落在 `[-1, 1]`；任一侧取值全同（分母为 0）或含 NaN 时返回 NaN，同 scipy。
fn kendall_tau_b(x: &[f64], y: &[f64]) -> f64 {
    // scipy 会先 dropna 再算；本函数的调用方已保证无 NaN，这里兜一道，返回 NaN 而非 panic
    if has_nan(x) || has_nan(y) {
        return f64::NAN;
    }
    let n = x.len();
    let n0 = (n * (n - 1) / 2) as f64;

    let (yr, ytie) = dense_ranks(y);
    let sup = yr.iter().copied().max().unwrap_or(0);

    // 已排除 NaN，partial_cmp 必有值；unwrap_or 只为不留 panic 路径
    let cmp = |a: f64, b: f64| a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| cmp(x[a], x[b]).then_with(|| cmp(y[a], y[b])));

    let mut tree = vec![0i64; sup + 1];
    let mut inserted = 0i64;
    let mut dis = 0i64;
    let mut xtie = 0.0f64;
    let mut ntie = 0.0f64;

    let mut i = 0usize;
    while i < n {
        let mut j = i + 1;
        while j < n && x[order[j]] == x[order[i]] {
            j += 1;
        }
        // 只与"x 严格更小"的元素比：已插入里 y 名次更大的个数即反序对
        for &o in &order[i..j] {
            dis += inserted - fenwick_sum(&tree, yr[o]);
        }
        for &o in &order[i..j] {
            fenwick_add(&mut tree, yr[o], sup);
            inserted += 1;
        }
        let t = (j - i) as f64;
        xtie += t * (t - 1.0) / 2.0;
        // 段内再按 y 分小段，数 (x, y) 同时并列的对
        let mut a = i;
        while a < j {
            let mut b = a + 1;
            while b < j && y[order[b]] == y[order[a]] {
                b += 1;
            }
            let c = (b - a) as f64;
            ntie += c * (c - 1.0) / 2.0;
            a = b;
        }
        i = j;
    }

    let (dx, dy) = (n0 - xtie, n0 - ytie);
    if dx <= 0.0 || dy <= 0.0 {
        return f64::NAN;
    }
    let con_minus_dis = n0 - xtie - ytie + ntie - 2.0 * dis as f64;
    (con_minus_dis / dx.sqrt() / dy.sqrt()).clamp(-1.0, 1.0)
}

/// 因子两两相关矩阵，对应上游 `correlation()` 返回的那个方阵 `DataFrame`。
///
/// 上游用 dict 收集各因子的堆叠序列，因子同名时会互相覆盖；此处按传入顺序存名字，
/// 重名不会合并（见 [`crate::analysis`] 的偏差说明）。
#[derive(Debug, Clone, PartialEq)]
pub struct CorrMatrix {
    /// 行列共用的因子名，顺序与 [`crate::analysis::FactorAnalyzer::factors`] 一致。
    names: Vec<String>,
    /// 行主序的 `n × n` 相关系数。
    values: Vec<f64>,
    /// 参与计算的 `(timestamp, symbol)` 单元格数，即上游 `dropna()` 之后的行数。
    n_obs: usize,
    /// 计算所用口径，供报告排版标注。
    method: CorrMethod,
}

impl CorrMatrix {
    /// 由完整方阵构建；`values` 为行主序，长度须为 `names.len()²`。
    ///
    /// - 入参：`names` 因子名；`values` 行主序方阵；`n_obs` 参与计算的样本数；
    ///   `method` 计算口径。
    /// - 加工：仅记录，不校验对称性。
    /// - 出参：[`CorrMatrix`]。
    pub(crate) fn new(
        names: Vec<String>,
        values: Vec<f64>,
        n_obs: usize,
        method: CorrMethod,
    ) -> CorrMatrix {
        debug_assert_eq!(values.len(), names.len() * names.len());
        CorrMatrix {
            names,
            values,
            n_obs,
            method,
        }
    }

    /// 空矩阵，对应上游因子不足 2 个或重叠样本不足时返回的空 `DataFrame`。
    ///
    /// - 入参：`method` 计算口径。
    /// - 加工：名字与取值都置空。
    /// - 出参：`is_empty()` 为真的 [`CorrMatrix`]。
    pub(crate) fn empty(method: CorrMethod) -> CorrMatrix {
        CorrMatrix {
            names: Vec::new(),
            values: Vec::new(),
            n_obs: 0,
            method,
        }
    }

    /// 行列共用的因子名。
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// 阶数，即因子个数。
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// 是否为空矩阵，对应上游 `DataFrame.empty`。
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// 参与计算的 `(timestamp, symbol)` 单元格数。
    pub fn n_obs(&self) -> usize {
        self.n_obs
    }

    /// 计算所用口径。
    pub fn method(&self) -> CorrMethod {
        self.method
    }

    /// 按下标取相关系数；越界返回 NaN。
    ///
    /// - 入参：`i` 行下标；`j` 列下标。
    /// - 加工：换算成行主序偏移 `i * n + j` 后读取。
    /// - 出参：该格相关系数；任一下标越界时返回 NaN 而非 panic。
    pub fn at(&self, i: usize, j: usize) -> f64 {
        let n = self.names.len();
        if i >= n || j >= n {
            return f64::NAN;
        }
        self.values[i * n + j]
    }

    /// 按因子名取相关系数。
    ///
    /// - 入参：`a` / `b` 两个因子名。
    /// - 加工：在名字表中线性查找下标后读取。
    /// - 出参：`Some(相关系数)`（值本身仍可能是 NaN）；任一名字不存在时返回 `None`。
    ///   有重名因子时取首个匹配。
    pub fn get(&self, a: &str, b: &str) -> Option<f64> {
        let i = self.names.iter().position(|n| n == a)?;
        let j = self.names.iter().position(|n| n == b)?;
        Some(self.values[i * self.names.len() + j])
    }

    /// 渲染成表格文本，复刻上游 `corr_matrix.to_string(float_format=lambda x: f'{x:.4f}')`
    /// 的排版。
    ///
    /// - 入参：无。
    /// - 加工：按 pandas 的三条规则排版——索引列左对齐到最长因子名；数值列的表头先补一个
    ///   前导空格（pandas 对数值列表头的固定处理），列宽取"表头宽"与"最宽单元格"的较大者，
    ///   单元格右对齐；列与列之间一个空格。数值按四位小数、NaN 写 `NaN`（pandas 的 `na_rep`）。
    /// - 出参：不含行尾空格、不含末尾换行的多行字符串；空矩阵返回空串。
    ///   宽度按字符数而非字节数计，与 Python 的 `ljust` / `rjust` 一致。
    pub fn to_string_table(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let width = |s: &str| s.chars().count();
        let cell = |v: f64| {
            if v.is_nan() {
                "NaN".to_string()
            } else {
                format!("{v:.4}")
            }
        };

        let n = self.names.len();
        let index_width = self.names.iter().map(|s| width(s)).max().unwrap_or(0);
        let headers: Vec<String> = self.names.iter().map(|s| format!(" {s}")).collect();
        let cells: Vec<Vec<String>> = (0..n)
            .map(|i| (0..n).map(|j| cell(self.at(i, j))).collect())
            .collect();
        let widths: Vec<usize> = (0..n)
            .map(|j| {
                let w = cells.iter().map(|row| width(&row[j])).max().unwrap_or(0);
                w.max(width(&headers[j]))
            })
            .collect();

        let mut out = " ".repeat(index_width);
        for (h, w) in headers.iter().zip(widths.iter()) {
            out.push_str(&format!(" {h:>w$}", w = w));
        }
        for (name, row) in self.names.iter().zip(cells.iter()) {
            out.push_str(&format!("\n{name:<index_width$}"));
            for (c, w) in row.iter().zip(widths.iter()) {
                out.push_str(&format!(" {c:>w$}", w = w));
            }
        }
        out
    }
}

impl fmt::Display for CorrMatrix {
    /// - 入参：`f` 格式化器。
    /// - 加工：直接输出 [`CorrMatrix::to_string_table`] 的表格文本。
    /// - 出参：多行表格；空矩阵输出一行提示。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            write!(f, "CorrMatrix(empty, method={})", self.method.label())
        } else {
            write!(f, "{}", self.to_string_table())
        }
    }
}
