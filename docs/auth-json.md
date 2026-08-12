# Codex auth.json 结构

Codex OAuth 登录使用以下标准结构：

```json
{
  "access_token": "<access_token>",
  "account_id": "<chatgpt_account_id>",
  "id_token": "<id_token>",
  "last_refresh": "2026-07-01T10:01:36.062573Z",
  "refresh_token": "<refresh_token>",
  "type": "codex"
}
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `access_token` | `string` | 调用 Codex 与账户信息接口的访问令牌。 |
| `account_id` | `string` | ChatGPT 账户或工作区 ID。 |
| `id_token` | `string` | 包含邮箱、账户 ID、套餐等身份声明的 JWT。 |
| `last_refresh` | `string` | 最近刷新认证信息的 RFC 3339 UTC 时间。 |
| `refresh_token` | `string` | 用于刷新访问令牌。 |
| `type` | `string` | 认证类型，Codex 账户为 `codex`。 |

Codex API Key 登录可以使用更精简的结构：

```json
{
  "OPENAI_API_KEY": "<api-key>"
}
```

本应用的中转站账户不写入 `auth.json`，而是在 `config.toml` 的自定义 Provider 中保存
Bearer Token：

```toml
forced_login_method = "api"
model_provider = "cortana"

[model_providers.cortana]
name = "Cortana"
base_url = "https://relay.example.com/v1"
experimental_bearer_token = "<api-key>"
```

中转站地址必须兼容 Responses API。切换到中转站账户时，应用会删除旧 `auth.json`；
切换回 OAuth 账户时会重新写入 OAuth `auth.json`，并删除顶层 `model_provider`
以及整个 `model_providers` 配置。其他设置和注释会保留。

默认配置路径为 `~/.codex/config.toml`。该文件包含中转站 API Key，不得提交到代码仓库或公开分享。
使用 HTTP 中转地址时，API Key 可能通过明文网络传输，应优先使用 HTTPS。
