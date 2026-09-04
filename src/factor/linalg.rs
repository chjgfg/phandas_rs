//! 线性代数：最小二乘、条件数（供中性化与滚动回归使用）。

/// 对称矩阵特征值（循环 Jacobi 法），用于条件数估计。
///
/// - 入参：`a` 方阵（按行存放，须对称）。
/// - 加工：反复对非对角元素做 Givens 旋转消元，直到非对角平方和收敛或达到 100 轮。
/// - 出参：对角线上的特征值向量（顺序不保证）。
// 矩阵消元按行列下标读写，下标循环比迭代器更贴近算法本身
#[allow(clippy::needless_range_loop)]
fn sym_eigenvalues(a: &[Vec<f64>]) -> Vec<f64> {
    let n = a.len();
    let mut m: Vec<Vec<f64>> = a.to_vec();
    for _ in 0..100 {
        let mut off = 0.0;
        for i in 0..n {
            for j in (i + 1)..n {
                off += m[i][j] * m[i][j];
            }
        }
        if off <= 1e-30 {
            break;
        }
        for p in 0..n {
            for q in (p + 1)..n {
                if m[p][q].abs() < 1e-300 {
                    continue;
                }
                let theta = (m[q][q] - m[p][p]) / (2.0 * m[p][q]);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());

                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let mkp = m[k][p];
                    let mkq = m[k][q];
                    m[k][p] = c * mkp - s * mkq;
                    m[k][q] = s * mkp + c * mkq;
                }
                for k in 0..n {
                    let mpk = m[p][k];
                    let mqk = m[q][k];
                    m[p][k] = c * mpk - s * mqk;
                    m[q][k] = s * mpk + c * mqk;
                }
            }
        }
    }
    (0..n).map(|i| m[i][i]).collect()
}

/// 对称正定矩阵的 2-范数条件数 `λmax / λmin`；退化时返回 `f64::INFINITY`。
///
/// - 入参：`a` 对称方阵（通常是设计矩阵的 Gram 矩阵 `XᵗX`）。
/// - 加工：算出全部特征值 → 取最大与最小 → 相除。
/// - 出参：条件数；矩阵为空、最小特征值非正（数值上已奇异）时返回 `+inf`，
///   供调用方与 [`MATRIX_COND_THRESHOLD`] 比较后决定是否放弃该次回归。
pub(crate) fn cond_sym(a: &[Vec<f64>]) -> f64 {
    if a.is_empty() {
        return f64::INFINITY;
    }
    let ev = sym_eigenvalues(a);
    let lmax = ev.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let lmin = ev.iter().copied().fold(f64::INFINITY, f64::min);
    if !lmax.is_finite() || lmin <= 0.0 {
        f64::INFINITY
    } else {
        lmax / lmin
    }
}

/// 部分选主元高斯消元求解 `a · x = b`；奇异时返回 `None`。
///
/// - 入参：`a` 系数方阵；`b` 右端向量（长度等于 `a` 的阶数）。
/// - 加工：逐列挑绝对值最大的行做主元并交换 → 前向消元成上三角 → 回代求解。
/// - 出参：解向量 `x`；主元过小（矩阵奇异）或解中出现非有限值时返回 `None`。
// 同上：消元过程需要同时按下标读写不同行
#[allow(clippy::needless_range_loop)]
fn solve_linear(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = a.len();
    let mut m: Vec<Vec<f64>> = a.to_vec();
    let mut rhs = b.to_vec();
    for col in 0..n {
        let (pivot, _) = (col..n).fold((col, 0.0), |(bi, bv), r| {
            let v = m[r][col].abs();
            if v > bv {
                (r, v)
            } else {
                (bi, bv)
            }
        });
        if m[pivot][col].abs() < 1e-300 {
            return None;
        }
        m.swap(col, pivot);
        rhs.swap(col, pivot);

        for r in (col + 1)..n {
            let f = m[r][col] / m[col][col];
            if f == 0.0 {
                continue;
            }
            for c in col..n {
                m[r][c] -= f * m[col][c];
            }
            rhs[r] -= f * rhs[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let mut acc = rhs[r];
        for c in (r + 1)..n {
            acc -= m[r][c] * x[c];
        }
        x[r] = acc / m[r][r];
    }
    if x.iter().any(|v| !v.is_finite()) {
        None
    } else {
        Some(x)
    }
}

/// Gauss-Jordan 求逆；奇异时返回 `None`。对应 `np.linalg.inv`。
///
/// - 入参：`a` 待求逆的方阵。
/// - 加工：右侧拼接单位矩阵 → 逐列选主元、归一化主元行 → 消去该列其余行 → 右半即为逆。
/// - 出参：逆矩阵；主元过小（奇异）时返回 `None`。
pub(crate) fn invert(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    let mut m: Vec<Vec<f64>> = a.to_vec();
    let mut inv: Vec<Vec<f64>> = (0..n)
        .map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect();
    for col in 0..n {
        let (pivot, _) = (col..n).fold((col, 0.0), |(bi, bv), r| {
            let v = m[r][col].abs();
            if v > bv {
                (r, v)
            } else {
                (bi, bv)
            }
        });
        if m[pivot][col].abs() < 1e-300 {
            return None;
        }
        m.swap(col, pivot);
        inv.swap(col, pivot);
        let d = m[col][col];
        for c in 0..n {
            m[col][c] /= d;
            inv[col][c] /= d;
        }

        for r in 0..n {
            if r == col {
                continue;
            }
            let f = m[r][col];
            if f == 0.0 {
                continue;
            }
            for c in 0..n {
                m[r][c] -= f * m[col][c];
                inv[r][c] -= f * inv[col][c];
            }
        }
    }
    Some(inv)
}

/// `design` 的 Gram 矩阵 `XᵗX`。
///
/// - 入参：`design` 设计矩阵（每行一个样本，列数即参数个数）。
/// - 加工：逐样本做外积累加。
/// - 出参：`k × k` 的对称矩阵（`k` 为列数），既用于正规方程也用于条件数估计。
pub(crate) fn gram(design: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let k = design.first().map_or(0, |r| r.len());
    let mut g = vec![vec![0.0; k]; k];
    for row in design {
        for i in 0..k {
            for j in 0..k {
                g[i][j] += row[i] * row[j];
            }
        }
    }
    g
}

/// 正规方程最小二乘：`design` 各行须已含前置常数列。对应 `np.linalg.lstsq`。
///
/// - 入参：`design` 设计矩阵（首列通常为全 1 的截距项）；`y` 因变量向量。
/// - 加工：算 `XᵗX` 与 `Xᵗy` → 交给 [`solve_linear`] 解正规方程。
/// - 出参：参数向量 `[截距, 斜率1, 斜率2, ...]`；列数为 0 或方程奇异时返回 `None`。
pub(crate) fn ols(design: &[Vec<f64>], y: &[f64]) -> Option<Vec<f64>> {
    let k = design.first().map_or(0, |r| r.len());
    if k == 0 {
        return None;
    }
    let g = gram(design);
    let mut rhs = vec![0.0; k];
    for (row, &yi) in design.iter().zip(y.iter()) {
        for i in 0..k {
            rhs[i] += row[i] * yi;
        }
    }
    solve_linear(&g, &rhs)
}
