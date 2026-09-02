# phandas_rs

[![crates.io](https://img.shields.io/crates/v/phandas_rs.svg)](https://crates.io/crates/phandas_rs)
[![docs.rs](https://docs.rs/phandas_rs/badge.svg)](https://docs.rs/phandas_rs)
[![license](https://img.shields.io/crates/l/phandas_rs.svg)](./LICENSE)

Alpha factor construction on dense panel data — 70+ cross-sectional and time-series
operators, zero dependencies. A Rust port of the Python
[phandas](https://github.com/quantbai/phandas) factor engine.

因子构造库：把 Python 版 phandas 的 `Factor` / `Panel` 与全部因子运算子移植到 Rust，
**零外部依赖**（分位数反函数、最小二乘、条件数估计、CSV 解析全部自带实现）。

## 安装

```toml
[dependencies]
phandas_rs = "0.1"
```

## 快速开始

```rust
use phandas_rs::factor::{rank, vector_neut, Panel};

let csv = "\
timestamp,symbol,open,high,low,close,volume
2024-01-01,AAA,10,11,9,10,100
2024-01-01,BBB,20,21,19,20,200
2024-01-02,AAA,10,12,10,11,110
2024-01-02,BBB,20,22,18,19,190
";

let panel = Panel::from_csv_str(csv).unwrap();
let close = panel.factor("close").unwrap();
let volume = panel.factor("volume").unwrap();

// 1 期动量，再对成交量排名做向量投影中性化
let momentum = close.divide(&close.ts_delay(1)).subtract(1.0);
let alpha = vector_neut(&rank(&momentum), &rank(&volume.reverse()))
    .rename("momentum_1_neut");

println!("{}", alpha.show(10));
```

跑内置演示与性能冒烟测试：

```bash
cargo run --bin phandas-demo
cargo run --release --example smoke -- path/to/crypto_1d.csv
```

## 数据模型

行 = `timestamp`（升序去重），列 = `symbol`（升序去重），值为 `f64`，缺失位置为 `NaN`。

- 横截面运算逐行计算，对应 Python 侧 `groupby('timestamp')`
- 时序运算逐列计算，对应 Python 侧 `groupby('symbol').rolling(...)`
- 二元运算按 `(timestamp, symbol)` 取交集，等价于 `pd.merge(how='inner')`

## 运算子

| 类别 | 举例 |
| --- | --- |
| 横截面 | `rank` `zscore` `normalize` `quantile` `scale` `spread` `signal` |
| 中性化 | `vector_neut` `regression_neut` `regression_neut_multi` `group_neutralize` |
| 分组 | `group_rank` `group_mean` `group_zscore` `group_scale` `group_normalize` |
| 时序 | `ts_delay` `ts_delta` `ts_rank` `ts_zscore` `ts_corr` `ts_regression` `ts_decay_linear` 等 65 个 |

完整清单见 [docs.rs](https://docs.rs/phandas_rs)。

## 与 Python 版的偏差

移植刻意保留了 Python 侧的若干反直觉行为（例如 `ts_decay_linear` 的权重方向、
`ts_skewness` 的复合定义、`spread` 中 NaN 占用多头名额），以保证因子值可比对。
另有少量已知偏差（`norm.ppf` 用 Acklam 有理逼近，相对误差约 `1e-9`；
时间戳按字符串字典序排序，要求 ISO-8601 输入）。逐条说明见
[`factor` 模块文档](https://docs.rs/phandas_rs/latest/phandas_rs/factor/)。

## MSRV

Rust 1.82。

## License

MIT，见 [LICENSE](./LICENSE)。本 crate 是 [phandas](https://github.com/quantbai/phandas)
（MIT，Copyright (c) Phantom Management）的衍生作品，上游版权声明一并保留在 LICENSE 中。
