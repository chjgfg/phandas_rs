//! OKX v5 REST 鉴权：签名、凭证与运行环境。
//!
//! 上游走 `python-okx` SDK，把签名藏在里面。这里自己实现——就四步，而且离线可验：
//!
//! ```text
//! prehash = timestamp + METHOD + requestPath + body
//! sign    = base64(HMAC_SHA256(prehash, secret_key))
//! ```
//!
//! 两处最容易签错，本模块用类型与文档把它们钉住：
//!
//! 1. **`requestPath` 必须含 query string**。签 `/api/v5/account/positions` 而发
//!    `/api/v5/account/positions?instType=SWAP` 就是 `Invalid Signature`。
//! 2. **`body` 必须是实际发出的那份字节**。批量下单的 body 是**顶层 JSON 数组**
//!    （`[{...},{...}]`），不是对象；重新序列化一遍再签很可能字节不同。
//!
//! 时间戳要求 UTC、带毫秒、以 `Z` 结尾，且与 OKX 服务器相差不超过 **30 秒**——签名被拒时
//! 先查本机时钟。生成见 [`crate::net::now_iso8601_millis`]。

use std::fmt;

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::net::{now_iso8601_millis, HttpRequest, Method};

/// OKX 的 API 根地址。实盘与模拟盘**同一个域名**，靠 `x-simulated-trading` 头区分。
pub const BASE_URL: &str = "https://www.okx.com";

/// 读凭证用的三个环境变量名。
pub const ENV_KEYS: [&str; 3] = ["OKX_API_KEY", "OKX_SECRET_KEY", "OKX_PASSPHRASE"];

/// 跑在模拟盘还是实盘。
///
/// **刻意不实现 `Default`**：调用方必须把 `Live` 显式写出来。这样「不小心打到实盘」在代码里
/// 就不是一个能默默发生的状态，review 时也一眼看得见。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// 模拟盘（demo trading）。请求带 `x-simulated-trading: 1`。
    Demo,
    /// 实盘。**会动真钱。**
    Live,
}

impl Environment {
    /// `x-simulated-trading` 头的取值。
    ///
    /// - 入参：无（取自身枚举值）。
    /// - 加工：模拟盘给 `"1"`、实盘给 `"0"`。
    /// - 出参：头值。上游 `python-okx` 的 `flag` 参数就是这个，且实盘也照样带上 `"0"`，
    ///   本仓库同。
    pub fn simulated_flag(self) -> &'static str {
        match self {
            Environment::Demo => "1",
            Environment::Live => "0",
        }
    }

    /// 是否实盘。
    ///
    /// - 入参：无。
    /// - 加工：与 [`Environment::Live`] 比较。
    /// - 出参：实盘为 `true`。调用方可据此在日志或提示里加重警示。
    pub fn is_live(self) -> bool {
        self == Environment::Live
    }
}

/// API 凭证。三项都不能为空。
///
/// `Debug` 是**打码**的：`secret_key` 与 `passphrase` 完全不输出，`api_key` 只留前 4 个字符。
/// 这样把整个客户端 `{:?}` 进日志也不会泄露凭证。
#[derive(Clone)]
pub struct Credentials {
    api_key: String,
    secret_key: String,
    passphrase: String,
}

impl Credentials {
    /// 由三段明文构造。
    ///
    /// - 入参：`api_key` / `secret_key` / `passphrase`。
    /// - 加工：逐项去空白后校验非空。
    /// - 出参：`Ok(Credentials)`；任一项为空（或只有空白）时返回指名是哪一项的 `Err`。
    pub fn new(
        api_key: impl Into<String>,
        secret_key: impl Into<String>,
        passphrase: impl Into<String>,
    ) -> Result<Credentials, String> {
        let (api_key, secret_key, passphrase) =
            (api_key.into(), secret_key.into(), passphrase.into());
        for (name, v) in [
            ("api_key", &api_key),
            ("secret_key", &secret_key),
            ("passphrase", &passphrase),
        ] {
            if v.trim().is_empty() {
                return Err(format!("{name} 不能为空"));
            }
        }
        Ok(Credentials {
            api_key: api_key.trim().to_string(),
            secret_key: secret_key.trim().to_string(),
            passphrase: passphrase.trim().to_string(),
        })
    }

    /// 从环境变量读取，变量名见 [`ENV_KEYS`]。
    ///
    /// - 入参：无。
    /// - 加工：依次读 `OKX_API_KEY` / `OKX_SECRET_KEY` / `OKX_PASSPHRASE`，再走
    ///   [`Credentials::new`] 校验。
    /// - 出参：`Ok(Credentials)`；缺哪个变量就在 `Err` 里指名。配合
    ///   [`crate::net::load_default_dotenv`] 可以把凭证放进 `.env`（**已在 `.gitignore` 里**）。
    ///
    /// 上游只能从构造参数传凭证，没有环境变量或配置文件入口——写脚本时容易把 key 硬编码
    /// 进源码，这里补上这条路。
    pub fn from_env() -> Result<Credentials, String> {
        let mut vals = Vec::with_capacity(3);
        for key in ENV_KEYS {
            vals.push(
                std::env::var(key)
                    .map_err(|_| format!("环境变量 {key} 未设置（或不是合法 UTF-8）"))?,
            );
        }
        Credentials::new(vals[0].clone(), vals[1].clone(), vals[2].clone())
    }

    /// API key。要放进 `OK-ACCESS-KEY` 头，故必须能取出。
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}

impl fmt::Debug for Credentials {
    /// - 入参：`f` 格式化器。
    /// - 加工：`api_key` 只留前 4 个字符并标注长度，另两项完全不输出。
    /// - 出参：形如 `Credentials { api_key: "1a2b…(36)", secret_key: <redacted>,
    ///   passphrase: <redacted> }`，可以放心打进日志。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let head: String = self.api_key.chars().take(4).collect();
        write!(
            f,
            "Credentials {{ api_key: \"{head}…({})\", secret_key: <redacted>, passphrase: <redacted> }}",
            self.api_key.chars().count()
        )
    }
}

/// 生成 OKX v5 的 `OK-ACCESS-SIGN`。
///
/// - 入参：`secret_key` 私钥；`timestamp` ISO-8601 带毫秒的 UTC 时刻（`2026-09-04T12:34:56.789Z`）；
///   `method` HTTP 方法；`request_path` **含 query string** 的路径（如
///   `/api/v5/account/positions?instType=SWAP`）；`body` 实际发出的请求体，`GET` 传空串。
/// - 加工：四段按顺序直接拼成待签串 → HMAC-SHA256 → base64。
/// - 出参：可直接放进 `OK-ACCESS-SIGN` 头的字符串。
///
/// 纯函数，不读时钟也不碰网络，故可以用已知向量离线钉死（见 `tests/test_trader.rs`）。
pub fn sign(
    secret_key: &str,
    timestamp: &str,
    method: Method,
    request_path: &str,
    body: &str,
) -> String {
    let prehash = format!("{timestamp}{}{request_path}{body}", method.as_str());
    // HMAC 接受任意长度密钥，new_from_slice 对 Hmac 而言不会失败
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret_key.as_bytes()).expect("HMAC 接受任意长度密钥");
    mac.update(prehash.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// 组装一个带全部鉴权头的请求。
///
/// - 入参：`creds` 凭证；`env` 运行环境；`base_url` API 根地址（不含尾斜杠）；`method` 方法；
///   `request_path` 含 query string 的路径；`body` 请求体（`GET` 传空串）；
///   `timestamp` 待签时刻——**显式传入**而不是内部读时钟，这样整条组装逻辑可复现、可测。
/// - 加工：算签名 → 拼 URL → 依次加五个鉴权头与 `Content-Type`。
/// - 出参：可直接交给 [`crate::net::HttpTransport`] 的请求。
///
/// 头的顺序固定（OKX 不在意顺序，但固定下来测试才好逐字符比对）：
/// `OK-ACCESS-KEY` / `OK-ACCESS-SIGN` / `OK-ACCESS-TIMESTAMP` / `OK-ACCESS-PASSPHRASE` /
/// `Content-Type` / `x-simulated-trading`。
pub fn signed_request_at(
    creds: &Credentials,
    env: Environment,
    base_url: &str,
    method: Method,
    request_path: &str,
    body: &str,
    timestamp: &str,
) -> HttpRequest {
    let signature = sign(&creds.secret_key, timestamp, method, request_path, body);
    let url = format!("{base_url}{request_path}");
    let req = match method {
        Method::Get => HttpRequest::get(url),
        Method::Post => HttpRequest::post(url, body.to_string()),
    };
    req.header("OK-ACCESS-KEY", creds.api_key.clone())
        .header("OK-ACCESS-SIGN", signature)
        .header("OK-ACCESS-TIMESTAMP", timestamp.to_string())
        .header("OK-ACCESS-PASSPHRASE", creds.passphrase.clone())
        .header("Content-Type", "application/json")
        .header("x-simulated-trading", env.simulated_flag())
}

/// [`signed_request_at`] 的便捷版：时间戳取当前时刻。
///
/// - 入参：同 [`signed_request_at`]，少了 `timestamp`。
/// - 加工：用 [`crate::net::now_iso8601_millis`] 取当前 UTC 时刻后转发。
/// - 出参：带鉴权头的请求。本机时钟与 OKX 相差超过 30 秒时会被服务端拒签。
pub fn signed_request(
    creds: &Credentials,
    env: Environment,
    base_url: &str,
    method: Method,
    request_path: &str,
    body: &str,
) -> HttpRequest {
    signed_request_at(
        creds,
        env,
        base_url,
        method,
        request_path,
        body,
        &now_iso8601_millis(),
    )
}
