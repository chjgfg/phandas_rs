//! 数值工具：语义对齐 pandas / numpy 的 NaN 处理。
//!
//! 含跳过 NaN 的统计量、稳定排序与名次百分位、标准正态分位数反函数与 [`Driver`]，
//! 以及因子名数值渲染、条件真值判定、样本协方差这几个小工具。

/// 判断切片中是否存在 NaN。
///
/// - 入参：`v` 待检查的数值切片。
/// - 加工：顺序扫描，遇到首个 NaN 即短路返回。
/// - 出参：存在 NaN 返回 `true`，否则 `false`（空切片返回 `false`）。
pub fn has_nan(v: &[f64]) -> bool {
    v.iter().any(|x| x.is_nan())
}

/// 统计切片中的有效（非 NaN）元素个数。
///
/// - 入参：`v` 待统计的数值切片。
/// - 加工：过滤掉 NaN 后计数。
/// - 出参：非 NaN 元素个数，用于判定 pandas `min_periods` 是否满足。
pub fn count_valid(v: &[f64]) -> usize {
    v.iter().filter(|x| !x.is_nan()).count()
}

/// 求和并跳过 NaN，对应 `Series.sum(skipna=True)`。
///
/// - 入参：`v` 待求和的数值切片。
/// - 加工：跳过 NaN 后累加。
/// - 出参：有效值之和；全为 NaN 或切片为空时返回 `0.0`（与 pandas 一致）。
fn nansum(v: &[f64]) -> f64 {
    v.iter().filter(|x| !x.is_nan()).sum()
}

/// 均值并跳过 NaN，对应 `Series.mean(skipna=True)`。
///
/// - 入参：`v` 待求均值的数值切片。
/// - 加工：先数有效值个数，再用有效值之和除以该个数。
/// - 出参：有效值的算术平均；无有效值时返回 NaN。
pub fn nanmean(v: &[f64]) -> f64 {
    let n = count_valid(v);
    if n == 0 {
        return f64::NAN;
    }
    nansum(v) / n as f64
}

/// 标准差并跳过 NaN。`ddof = 1` 对应 pandas 默认，`ddof = 0` 对应 numpy 默认。
///
/// - 入参：`v` 数值切片；`ddof` 自由度修正量（分母为 `有效个数 - ddof`）。
/// - 加工：求有效值均值 → 累加平方偏差 → 除以 `n - ddof` → 开平方。
/// - 出参：标准差；有效个数不足 `ddof + 1` 时返回 NaN。
pub fn nanstd(v: &[f64], ddof: usize) -> f64 {
    let n = count_valid(v);
    if n <= ddof {
        return f64::NAN;
    }
    let m = nanmean(v);
    let ss: f64 = v
        .iter()
        .filter(|x| !x.is_nan())
        .map(|x| (x - m) * (x - m))
        .sum();
    (ss / (n - ddof) as f64).sqrt()
}

/// 中位数并跳过 NaN，对应 `Series.median(skipna=True)`。
///
/// - 入参：`v` 数值切片。
/// - 加工：滤掉 NaN → 升序排序 → 奇数个取正中、偶数个取中间两数均值。
/// - 出参：中位数；无有效值时返回 NaN。
pub fn nanmedian(v: &[f64]) -> f64 {
    let mut xs: Vec<f64> = v.iter().copied().filter(|x| !x.is_nan()).collect();
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).expect("已过滤 NaN"));
    let n = xs.len();
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    }
}

/// 最小值并跳过 NaN，对应 `Series.min(skipna=True)`。
///
/// - 入参：`v` 数值切片。
/// - 加工：以 NaN 为初值折叠，遇到更小的有效值就替换。
/// - 出参：有效值中的最小者；无有效值时返回 NaN。
pub fn nanmin(v: &[f64]) -> f64 {
    v.iter()
        .copied()
        .filter(|x| !x.is_nan())
        .fold(
            f64::NAN,
            |acc, x| if acc.is_nan() || x < acc { x } else { acc },
        )
}

/// 最大值并跳过 NaN，对应 `Series.max(skipna=True)`。
///
/// - 入参：`v` 数值切片。
/// - 加工：以 NaN 为初值折叠，遇到更大的有效值就替换。
/// - 出参：有效值中的最大者；无有效值时返回 NaN。
pub fn nanmax(v: &[f64]) -> f64 {
    v.iter()
        .copied()
        .filter(|x| !x.is_nan())
        .fold(
            f64::NAN,
            |acc, x| if acc.is_nan() || x > acc { x } else { acc },
        )
}

/// `±inf` 归一为 NaN，对应 Python 侧 `Factor._replace_inf`。
///
/// - 入参：`x` 单个数值。
/// - 加工：判断是否为无穷。
/// - 出参：无穷则返回 NaN，其余原样返回（NaN 仍是 NaN）。
pub(crate) fn clean_inf(x: f64) -> f64 {
    if x.is_infinite() {
        f64::NAN
    } else {
        x
    }
}

/// 稳定升序排序后的下标，NaN 排在末尾（对齐 `np.argsort` 的 NaN 语义）。
///
/// - 入参：`v` 数值切片。
/// - 加工：对下标序列排序——有效值按数值升序、相等时按原下标升序（保证稳定），NaN 一律靠后。
/// - 出参：长度与 `v` 相同的下标向量，`result[0]` 指向最小值所在位置。
pub(crate) fn argsort_stable(v: &[f64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| match (v[a].is_nan(), v[b].is_nan()) {
        (true, true) => a.cmp(&b),
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => v[a].partial_cmp(&v[b]).expect("已排除 NaN").then(a.cmp(&b)),
    });
    idx
}

/// 并列名次的处理方式，对应 pandas `Series.rank(method=...)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RankMethod {
    /// 并列取最小名次，对应 `method='min'`（横截面 `rank()` 使用）。
    Min,
    /// 并列取平均名次，对应 `method='average'`（`group_rank()` 使用）。
    Average,
}

/// pandas `Series.rank(method)`：1 起算的名次，NaN 保持 NaN。
///
/// - 入参：`v` 数值切片；`method` 并列名次的处理方式。
/// - 加工：稳定排序取出有效值顺序 → 扫描相等区间 → 按 `method` 取该区间的最小或平均名次。
/// - 出参：长度与 `v` 相同的名次向量，取值落在 `[1, 有效个数]`；原 NaN 位置仍为 NaN。
pub(crate) fn ranks(v: &[f64], method: RankMethod) -> Vec<f64> {
    let mut out = vec![f64::NAN; v.len()];
    let valid: Vec<usize> = argsort_stable(v)
        .into_iter()
        .filter(|&i| !v[i].is_nan())
        .collect();
    let mut k = 0usize;
    while k < valid.len() {
        let mut j = k + 1;
        while j < valid.len() && v[valid[j]] == v[valid[k]] {
            j += 1;
        }
        // 1-based 名次区间为 [k + 1, j]
        let r = match method {
            RankMethod::Min => (k + 1) as f64,
            RankMethod::Average => ((k + 1 + j) as f64) / 2.0,
        };
        for &i in &valid[k..j] {
            out[i] = r;
        }
        k = j;
    }
    out
}

/// pandas `Series.rank(method, pct=True)`：NaN 保持 NaN，分母为非 NaN 元素个数。
///
/// - 入参：`v` 数值切片；`method` 并列名次的处理方式。
/// - 加工：先取 [`ranks`] 的名次，再逐个除以有效个数得到百分位。
/// - 出参：长度与 `v` 相同的百分位向量，取值落在 `(0, 1]`；原 NaN 位置仍为 NaN。
pub(crate) fn rank_pct(v: &[f64], method: RankMethod) -> Vec<f64> {
    let n_valid = count_valid(v);
    if n_valid == 0 {
        return vec![f64::NAN; v.len()];
    }
    ranks(v, method)
        .into_iter()
        .map(|r| r / n_valid as f64)
        .collect()
}

/// 标准正态分布分位数反函数（Acklam 有理逼近），对应 `scipy.stats.norm.ppf`。
///
/// - 入参：`p` 累积概率。
/// - 加工：按 `p` 落在低尾 / 中段 / 高尾分三段，各用一组有理多项式逼近。
/// - 出参：满足 `Φ(x) = p` 的分位点 `x`；`p = 0` 返回 `-inf`、`p = 1` 返回 `+inf`、
///   越界或 NaN 返回 NaN。相对误差约 `1e-9`。
// 系数照抄 Acklam 原文，保留其位数以便与参考实现逐位比对
#[allow(clippy::excessive_precision)]
pub fn norm_ppf(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383577518672690e+02,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];

    const P_LOW: f64 = 0.02425;

    if p.is_nan() || p <= 0.0 || p >= 1.0 {
        return if p == 0.0 {
            f64::NEG_INFINITY
        } else if p == 1.0 {
            f64::INFINITY
        } else {
            f64::NAN
        };
    }

    if p < P_LOW {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - P_LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

/// 标准正态分布累积分布函数（Hart 1968 有理逼近，West 的双精度实现），
/// 对应 `scipy.stats.norm.cdf`。回测的 PSR 需要它。
///
/// - 入参：`x` 分位点。
/// - 加工：先算 `|x|` 的上尾概率——`|x| < 7.07` 走两组 7 阶有理多项式之比，
///   更远的尾部走连分式；`|x| > 37` 时上尾在双精度下已是 0。最后按 `x` 的符号翻转。
/// - 出参：`Φ(x)`，取值落在 `[0, 1]`；绝对误差约 `1e-15`。`x` 为 NaN 时返回 NaN。
///   系数照抄 Hart / West 原文，保留其位数以便与参考实现逐位比对
#[allow(clippy::excessive_precision)]
pub fn norm_cdf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    let xabs = x.abs();
    let upper = if xabs > 37.0 {
        0.0
    } else {
        let e = (-xabs * xabs / 2.0).exp();
        if xabs < 7.071_067_811_865_47 {
            let mut num = 3.526_249_659_989_11e-02 * xabs + 0.700_383_064_443_688;
            num = num * xabs + 6.373_962_203_531_65;
            num = num * xabs + 33.912_866_078_383;
            num = num * xabs + 112.079_291_497_871;
            num = num * xabs + 221.213_596_169_931;
            num = num * xabs + 220.206_867_912_376;
            let mut den = 8.838_834_764_831_84e-02 * xabs + 1.755_667_163_182_64;
            den = den * xabs + 16.064_177_579_207;
            den = den * xabs + 86.780_732_202_946_1;
            den = den * xabs + 296.564_248_779_674;
            den = den * xabs + 637.333_633_378_831;
            den = den * xabs + 793.826_512_519_948;
            den = den * xabs + 440.413_735_824_752;
            e * num / den
        } else {
            // 远尾用连分式：xabs + 1/(xabs + 2/(xabs + 3/(xabs + 4/(xabs + 0.65))))
            let mut cf = xabs + 0.65;
            for k in (1..=4).rev() {
                cf = xabs + k as f64 / cf;
            }
            e / cf / 2.506_628_274_631
        }
    };
    if x > 0.0 {
        1.0 - upper
    } else {
        upper
    }
}

/// 分位数映射驱动，对应 Python 侧 `quantile(driver=...)` 参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Driver {
    /// `scipy.stats.norm.ppf`
    #[default]
    Gaussian,
    /// `scipy.stats.uniform.ppf`
    Uniform,
    /// `scipy.stats.cauchy.ppf`
    Cauchy,
}

impl Driver {
    /// 由字符串解析，对应 Python 侧 `"gaussian"` / `"uniform"` / `"cauchy"`。
    ///
    /// - 入参：`s` 驱动名称。
    /// - 加工：与三个合法名称精确匹配。
    /// - 出参：匹配成功返回 [`Driver`]；否则返回含可选值清单的错误信息。
    pub fn parse(s: &str) -> Result<Driver, String> {
        match s {
            "gaussian" => Ok(Driver::Gaussian),
            "uniform" => Ok(Driver::Uniform),
            "cauchy" => Ok(Driver::Cauchy),
            other => Err(format!(
                "Invalid driver: {other}. Must be one of ['gaussian', 'uniform', 'cauchy']"
            )),
        }
    }

    /// 按当前驱动做分位数反变换。
    ///
    /// - 入参：`p` 落在 `(0, 1)` 的累积概率。
    /// - 加工：正态走 [`norm_ppf`]；均匀分布恒等映射；柯西走 `tan(π · (p - 0.5))`。
    /// - 出参：对应分布下的分位点。
    pub(crate) fn ppf(self, p: f64) -> f64 {
        match self {
            Driver::Gaussian => norm_ppf(p),
            Driver::Uniform => p,
            Driver::Cauchy => (std::f64::consts::PI * (p - 0.5)).tan(),
        }
    }

    /// 驱动名称的字符串形式，用于拼接因子名。
    ///
    /// - 入参：无（取自身枚举值）。
    /// - 加工：枚举值到 Python 侧同名字符串的映射。
    /// - 出参：`"gaussian"` / `"uniform"` / `"cauchy"` 之一。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Driver::Gaussian => "gaussian",
            Driver::Uniform => "uniform",
            Driver::Cauchy => "cauchy",
        }
    }
}

/// 因子名中的数值渲染：整数值省略小数部分，贴近 Python 的 `str(int)` / `str(float)`。
///
/// - 入参：`v` 待渲染的数值。
/// - 加工：有限且小数部分为 0 且量级安全时按整数输出，否则用默认浮点格式。
/// - 出参：用于拼接因子名的字符串，例如 `2` 而非 `2.0`。
pub(crate) fn fmt_num(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// `cond` 的真值判定：NaN 视为假，非零视为真。
///
/// - 入参：`v` 条件因子中的单个取值。
/// - 加工：先排除 NaN，再判断是否非零。
/// - 出参：真值布尔。
pub(crate) fn truthy(v: f64) -> bool {
    !v.is_nan() && v != 0.0
}

/// 样本协方差（`ddof = 1`），要求两序列等长且无 NaN。
///
/// - 入参：`x` / `y` 等长的窗口切片。
/// - 加工：各自去均值后累加乘积，除以 `n - 1`。
/// - 出参：样本协方差；长度小于 2 时返回 NaN。
pub(crate) fn covariance(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len();
    if n < 2 {
        return f64::NAN;
    }
    let mx = nanmean(x);
    let my = nanmean(y);
    let s: f64 = x
        .iter()
        .zip(y.iter())
        .map(|(a, b)| (a - mx) * (b - my))
        .sum();
    s / (n - 1) as f64
}
