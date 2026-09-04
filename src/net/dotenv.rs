//! `.env` 读取：让「本地要走代理、服务器不要」这类差异落在文件里而不是代码里。
//!
//! 为什么自己写而不引 `dotenvy`：解析规则本来就只有几行，而这个仓库连 CSV 解析都是自己写的
//! （见 `factor::Panel::from_csv_str`），为二十行逻辑多挂一个依赖不合算。
//!
//! # 用法
//!
//! 在 `main` 的最前面调一次，之后 `reqwest` 建客户端时就能读到 `HTTPS_PROXY`：
//!
//! ```no_run
//! # #[cfg(feature = "_net")] {
//! phandas_rs::net::load_default_dotenv();
//! # }
//! ```
//!
//! 仓库根放一个 `.env`（**已在 `.gitignore` 里**，不会被提交）：
//!
//! ```text
//! # 本地走代理；服务器上不放这个文件，于是直连
//! HTTPS_PROXY=http://127.0.0.1:7897
//! HTTP_PROXY=http://127.0.0.1:7897
//! ```
//!
//! # 两条要留意的规则
//!
//! - **已存在的环境变量不会被覆盖**：真实环境变量优先于 `.env`，所以临时
//!   `HTTPS_PROXY= cargo test` 能压过文件里的设置。
//! - **文件不存在不算错**：[`load_default_dotenv`] 直接返回 `0`。服务器上就是靠这一条什么都不用配。
//!
//! 只做最朴素的解析：跳过空行与 `#` 注释、去掉可选的 `export ` 前缀、按**第一个** `=` 切分、
//! 去掉值两端配对的单双引号。不支持变量插值、多行值与转义。

use std::path::Path;

/// 从指定文件读环境变量。
///
/// - 入参：`path` `.env` 文件路径。
/// - 加工：逐行解析 `KEY=VALUE`；**跳过**已经存在于环境里的键。
/// - 出参：`Ok(实际写入的条数)`；文件读不出来时返回带路径的 `Err`。
///
/// 内部调 `std::env::set_var`：它改的是进程全局状态，请在程序**最前面**、还没起线程也没建
/// HTTP 客户端时调用。
pub fn load_dotenv(path: impl AsRef<Path>) -> Result<usize, String> {
    let text = std::fs::read_to_string(path.as_ref())
        .map_err(|e| format!("读取 {} 失败：{e}", path.as_ref().display()))?;
    let mut n = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || std::env::var_os(key).is_some() {
            continue;
        }
        let value = value.trim();
        // 去掉配对的引号；不配对的原样保留
        let value = match (value.chars().next(), value.chars().last(), value.len()) {
            (Some('"'), Some('"'), n) if n >= 2 => &value[1..value.len() - 1],
            (Some('\''), Some('\''), n) if n >= 2 => &value[1..value.len() - 1],
            _ => value,
        };
        std::env::set_var(key, value);
        n += 1;
    }
    Ok(n)
}

/// 从当前工作目录的 `.env` 读环境变量；文件不存在就什么也不做。
///
/// - 入参：无。
/// - 加工：转发给 [`load_dotenv`]，把「文件不存在」这类错误吞掉。
/// - 出参：实际写入的条数；没有 `.env` 时为 `0`。
///
/// 这正是「本地放 `.env` 走代理、服务器不放就直连」想要的行为，两边同一份代码。
pub fn load_default_dotenv() -> usize {
    load_dotenv(".env").unwrap_or(0)
}
