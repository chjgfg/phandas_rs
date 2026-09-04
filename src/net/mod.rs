//! 联网底座：`data` 与 `trader` 两个模块共用的 HTTP 传输层与 UTC 时间换算。
//!
//! 本模块只在开启 `data` 或 `trader` feature 时编译；默认构建不含它，仓库的零依赖承诺不变。
//!
//! # 文件划分
//!
//! | 文件 | 内容 |
//! |---|---|
//! | `http.rs` | [`HttpTransport`] trait、[`HttpRequest`] / [`HttpResponse`] 与 [`ReqwestTransport`] |
//! | `time.rs` | epoch 毫秒 ↔ 时间戳字符串、OKX 签名要的 ISO-8601 |
//! | `dotenv.rs` | 极简 `.env` 读取，用来分开「本地走代理」与「服务器直连」 |
//!
//! # 为什么把网络挖成一个 trait
//!
//! 交易所端点在很多环境里连不上（防火墙、代理白名单、CI 无外网），若把 `reqwest` 直接写死在
//! 抓数逻辑里，「构造请求 → 解析响应」这段就完全无法离线验证。把它收进 [`HttpTransport`] 之后，
//! 测试可以塞一个返回固定 JSON 的假实现，把分页游标推进、多源合并、错误分支全部覆盖到，
//! 真正联网的测试则单独标 `#[ignore]` 交给能出网的环境跑。
//!
//! 附带的好处是这层对使用者也是开放的：要换 TLS 栈、加重试与退避、走自定义代理、改限频策略，
//! 实现一个 [`HttpTransport`] 传进去即可，不必等本仓库支持。
//!
//! # 时间口径
//!
//! 全程 UTC，不做时区转换——上游 `data.py` 同样没有任何时区参数，`start_date` / `end_date`
//! 都按当日 00:00:00 UTC 解释。日期运算复用 [`crate::backtest::date`] 里的 Hinnant 算法，
//! 不引 `chrono` / `time`。

pub mod dotenv;
pub mod http;
pub mod time;

pub use self::dotenv::{load_default_dotenv, load_dotenv};
pub use self::http::{HttpRequest, HttpResponse, HttpTransport, Method, ReqwestTransport};
pub use self::time::{date_to_millis, millis_to_date, millis_to_datetime, now_iso8601_millis};
