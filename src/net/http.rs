//! HTTP 传输层：把「真正发出网络请求」这件事抽成一个 trait，让上层逻辑离线可测。
//!
//! trait 用 RPITIT（trait 里的 `async fn`，Rust 1.75 稳定）做静态派发：不引 `async_trait`、
//! 不给每次调用装箱，返回的 future 显式带 `Send`，调用方照样能 `tokio::spawn`。
//!
//! 设计取舍见 [`crate::net`] 的模块文档。

use std::fmt;
use std::future::Future;
use std::time::Duration;

/// 默认单次请求超时。交易所偶发慢响应，比 reqwest 的「不超时」保险。
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP 方法。只列本仓库用到的两个。
///
/// 用枚举而不是 `&str`：OKX 的签名要把大写方法名原样拼进待签串，拼错一个字母就是
/// `Invalid Signature`，让类型系统把这条路堵死。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`，本仓库全部只读端点。
    Get,
    /// `POST`，下单与账户设置。
    Post,
}

impl Method {
    /// 大写方法名。
    ///
    /// - 入参：无（取自身枚举值）。
    /// - 加工：枚举值到字面量的映射。
    /// - 出参：`"GET"` / `"POST"`，可直接拼进 OKX 的待签串。
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
        }
    }
}

impl fmt::Display for Method {
    /// - 入参：`f` 格式化器。
    /// - 加工：输出 [`Method::as_str`]。
    /// - 出参：`GET` / `POST`。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一次 HTTP 请求的全部输入。
///
/// `url` 里**已经含好 query string**：OKX 的签名要对「路径 + 查询串」整体签，若让传输层再拼一次
/// 参数，签的和发的就可能不是同一份。故拼参数是调用方的事，传输层只管原样发出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// HTTP 方法。
    pub method: Method,
    /// 完整 URL，含 query string。
    pub url: String,
    /// 头部键值对，按插入顺序发出。
    pub headers: Vec<(String, String)>,
    /// 请求体；`GET` 传空串。
    pub body: String,
}

impl HttpRequest {
    /// 构造一个 `GET` 请求。
    ///
    /// - 入参：`url` 完整 URL（含 query string）。
    /// - 加工：方法置 `GET`，头部与请求体置空。
    /// - 出参：[`HttpRequest`]，可继续用 [`HttpRequest::header`] 链式加头。
    pub fn get(url: impl Into<String>) -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: String::new(),
        }
    }

    /// 构造一个 `POST` 请求。
    ///
    /// - 入参：`url` 完整 URL；`body` 请求体（本仓库一律是 JSON 文本）。
    /// - 加工：方法置 `POST`，头部置空——`Content-Type` 由调用方按需加，
    ///   因为它要参与 OKX 的签名流程。
    /// - 出参：[`HttpRequest`]。
    pub fn post(url: impl Into<String>, body: impl Into<String>) -> HttpRequest {
        HttpRequest {
            method: Method::Post,
            url: url.into(),
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// 追加一个头部并返回自身，便于链式书写。
    ///
    /// - 入参：`name` 头名；`value` 头值。
    /// - 加工：追加到 `headers` 末尾，不去重、不覆盖同名项。
    /// - 出参：改动后的自身（消耗所有权，避免多余克隆）。
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> HttpRequest {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// 一次 HTTP 响应。
///
/// 只保留状态码与响应体文本：两个交易所的错误信息都在 JSON 体里（Binance 的 `code` / `msg`、
/// OKX 的 `code` / `msg` / `sCode`），头部对本仓库没有用处。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// HTTP 状态码。
    pub status: u16,
    /// 响应体文本。
    pub body: String,
}

impl HttpResponse {
    /// 是否 2xx。
    ///
    /// - 入参：无。
    /// - 加工：判断状态码落在 `[200, 300)`。
    /// - 出参：成功为 `true`。注意两个交易所都可能在 200 里带业务错误码，
    ///   故这只是第一道闸门，业务码要各自再判。
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// HTTP 传输层抽象。实现它即可替换掉本仓库全部网络出口。
///
/// 返回的 future 带 `Send`，可直接丢进 `tokio::spawn`。
pub trait HttpTransport {
    /// 发出一次请求。
    ///
    /// - 入参：`req` 完整请求（URL 已含 query string，头部与请求体已备好）。
    /// - 加工：由实现决定——真实实现打网络，测试实现按 URL 匹配返回预置响应。
    /// - 出参：`Ok(HttpResponse)` 只表示**传输成功**（拿到了状态码与响应体），
    ///   4xx / 5xx 也走这一支；只有连接失败、超时、响应体读不出来才返回 `Err`。
    fn send(&self, req: HttpRequest) -> impl Future<Output = Result<HttpResponse, String>> + Send;
}

/// 基于 `reqwest` 的真实传输层。
///
/// 内部持一个 `reqwest::Client`：它自带连接池与 keep-alive，跨请求复用比上游每个 symbol
/// 新建连接省一大截握手开销。`Client` 内部是 `Arc`，克隆很便宜。
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// 用默认配置构造：30 秒超时，其余走 reqwest 默认（含读取 `HTTPS_PROXY` 等环境变量）。
    ///
    /// - 入参：无。
    /// - 加工：`ClientBuilder` 设超时后 build。
    /// - 出参：`Ok(ReqwestTransport)`；TLS 后端初始化失败时返回 `Err`。
    pub fn new() -> Result<ReqwestTransport, String> {
        ReqwestTransport::with_timeout(DEFAULT_TIMEOUT)
    }

    /// 指定超时构造。
    ///
    /// - 入参：`timeout` 单次请求的总超时。
    /// - 加工：同 [`ReqwestTransport::new`]，只换超时值。
    /// - 出参：`Ok(ReqwestTransport)`；构造失败时返回带原因的 `Err`。
    pub fn with_timeout(timeout: Duration) -> Result<ReqwestTransport, String> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("构造 HTTP 客户端失败：{e}"))?;
        Ok(ReqwestTransport { client })
    }

    /// 用调用方自己配好的 `reqwest::Client` 构造。
    ///
    /// - 入参：`client` 已配置的客户端（代理、超时、根证书、UA 都由调用方定）。
    /// - 加工：直接持有。
    /// - 出参：[`ReqwestTransport`]。
    pub fn with_client(client: reqwest::Client) -> ReqwestTransport {
        ReqwestTransport { client }
    }
}

impl HttpTransport for ReqwestTransport {
    /// - 入参：`req` 完整请求。
    /// - 加工：按方法建 `RequestBuilder` → 逐条加头 → `POST` 时挂上请求体 → 发出 →
    ///   读状态码与响应体文本。
    /// - 出参：`Ok(HttpResponse)`（含 4xx / 5xx）；连接失败、超时或响应体读取失败返回 `Err`。
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let mut builder = match req.method {
            Method::Get => self.client.get(&req.url),
            Method::Post => self.client.post(&req.url).body(req.body),
        };
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| format!("请求 {} 失败：{e}", req.url))?;
        let status = resp.status().as_u16();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("读取 {} 的响应体失败：{e}", req.url))?;
        Ok(HttpResponse { status, body })
    }
}
