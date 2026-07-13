# OpenAI 接口清单

本文记录本项目当前直接调用的 OpenAI/ChatGPT 接口。清单基于
实现位于 `src-tauri/src/accounts.rs`、`codex.rs` 和 `oauth.rs`，最后核对日期为 2026-07-10。

## 概览

| 接口 | 方法 | 用途 | 稳定性 |
| --- | --- | --- | --- |
| `https://auth.openai.com/oauth/authorize` | `GET` | 在浏览器中完成 Codex OAuth 授权 | OpenAI 登录端点；具体 Codex 参数未形成公开 API 合约 |
| `https://auth.openai.com/oauth/token` | `POST` | 使用授权码和 PKCE verifier 换取令牌 | OpenAI 登录端点；具体 Codex 参数未形成公开 API 合约 |
| `https://chatgpt.com/backend-api/wham/usage` | `GET` | 查询套餐、额度窗口和积分余额 | ChatGPT 内部接口，未见公开 API 文档，可能变更 |

本项目目前不调用 `https://api.openai.com/v1/*` 下的公开模型 API。OpenAI 官方资料确认
Codex 支持使用 ChatGPT 账户登录，并会把认证信息保存在本地；但没有公开承诺本文所列端点
的完整请求和响应结构。参考：[Codex CLI and Sign in with ChatGPT](https://help.openai.com/en/articles/11381614-api-codex-cli-and-sign-in-with-chatgpt)、
[Using Codex with your ChatGPT plan](https://help.openai.com/en/articles/11369540-using-codex-with-chatgpt)。

中转站账户同样不会由本应用直接调用模型接口。应用只把 API Key 写入 `auth.json`，并把
Responses API 自定义 Provider 写入 `config.toml`；后续请求由 Codex 发起。由于中转站没有统一的
额度接口，这类账户不参与套餐与额度刷新。

## OAuth 授权

### 发起授权

```http
GET https://auth.openai.com/oauth/authorize
```

当前查询参数：

| 参数 | 当前值或来源 |
| --- | --- |
| `response_type` | `code` |
| `client_id` | `app_EMoamEEZ73f0CkXaXp7hrann` |
| `redirect_uri` | `http://localhost:1455/auth/callback` |
| `scope` | `openid profile email offline_access api.connectors.read api.connectors.invoke` |
| `code_challenge` | 随机 PKCE verifier 的 SHA-256 Base64URL 值 |
| `code_challenge_method` | `S256` |
| `id_token_add_organizations` | `true` |
| `codex_cli_simplified_flow` | `true` |
| `state` | 每次授权随机生成，用于校验回调 |
| `originator` | `codex_cli_rs` |

授权完成后，浏览器回调本机 `localhost:1455`。应用只接受固定 host、端口和
`/auth/callback` 路径，并校验 `state`。本地回调服务不是 OpenAI 接口。

### 交换令牌

```http
POST https://auth.openai.com/oauth/token
Accept: application/json
Content-Type: application/x-www-form-urlencoded
```

请求体：

```text
grant_type=authorization_code
code=<authorization_code>
redirect_uri=http://localhost:1455/auth/callback
client_id=app_EMoamEEZ73f0CkXaXp7hrann
code_verifier=<pkce_verifier>
```

项目当前读取以下响应字段：

```json
{
  "access_token": "<access_token>",
  "refresh_token": "<refresh_token>",
  "id_token": "<id_token>"
}
```

令牌会转换为项目约定的 `auth.json` 结构，详见 [Codex auth.json 结构](auth-json.md)。
当前项目尚未调用 `refresh_token` grant；访问令牌失效后，账户信息查询会返回错误，需要重新授权。

## 套餐与额度

```http
GET https://chatgpt.com/backend-api/wham/usage
Authorization: Bearer <access_token>
ChatGPT-Account-ID: <account_id>
Accept: application/json
User-Agent: codex_cli_rs
```

`ChatGPT-Account-ID` 在账户 ID 可用时发送。接口响应中项目当前使用的字段如下：

```json
{
  "plan_type": "plus",
  "rate_limit": {
    "allowed": true,
    "limit_reached": false,
    "primary_window": {
      "used_percent": 42,
      "limit_window_seconds": 18000,
      "reset_at": 1777000000
    },
    "secondary_window": {
      "used_percent": 5,
      "limit_window_seconds": 604800,
      "reset_at": 1777600000
    }
  },
  "credits": {
    "has_credits": true,
    "unlimited": false,
    "balance": "9.99"
  }
}
```

字段用途：

| 字段 | 用途 |
| --- | --- |
| `plan_type` | 显示 Free、Plus、Pro、Team 等套餐 |
| `used_percent` | 用 `100 - used_percent` 计算剩余额度百分比 |
| `limit_window_seconds` | 识别 5 小时、7 天等额度窗口 |
| `reset_at` | Unix 秒级时间戳，表示额度窗口重置时间 |
| `credits.unlimited` | 标识积分是否不限量 |
| `credits.balance` | 显示积分余额 |

`primary_window`、`secondary_window` 和 `credits` 都可能缺失。项目仅在响应包含可识别的
额度窗口、积分或 `rate_limit.allowed` 时更新本地缓存。

> 注意：`/backend-api/wham/usage` 是 ChatGPT Web 后端接口，不是 OpenAI 公开开发者 API。
> 调用方式和字段均应视为可变实现细节；升级前应使用真实账户重新验证。

## 身份声明

项目还会解码 `id_token` 或 `access_token` 的 JWT payload，但不会把 JWT claim URL 当作
HTTP 接口调用。当前读取：

| Claim | 字段 |
| --- | --- |
| `https://api.openai.com/auth` | `chatgpt_account_id`、`chatgpt_plan_type` |
| `https://api.openai.com/profile` | `email` |
| JWT 根对象 | `email` 兜底值 |

## 安全约束

- 文档、日志和错误信息不得输出完整令牌。
- OAuth 必须保留 PKCE `S256` 和随机 `state` 校验。
- `auth.json` 与 SQLite 数据库包含可用凭据，不得提交到仓库。
- 私有接口返回 `401` 或响应结构变化时，不应继续使用旧额度数据冒充最新结果。
