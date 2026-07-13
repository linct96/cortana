# Codex auth.json 结构

Codex OAuth 登录使用以下标准结构：

```json
{
  "OPENAI_API_KEY": null,
  "last_refresh": "2026-07-01T10:01:36.062573Z",
  "tokens": {
    "access_token": "<access_token>",
    "account_id": "<chatgpt_account_id>",
    "id_token": "<id_token>",
    "refresh_token": "<refresh_token>"
  }
}
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `OPENAI_API_KEY` | `string \| null` | OAuth 登录时为 `null`；API Key 登录时保存对应密钥。 |
| `last_refresh` | `string` | 最近刷新认证信息的 RFC 3339 UTC 时间。 |
| `tokens.access_token` | `string` | 调用 Codex 与账户信息接口的访问令牌。 |
| `tokens.account_id` | `string` | ChatGPT 账户或工作区 ID。 |
| `tokens.id_token` | `string` | 包含邮箱、账户 ID、套餐等身份声明的 JWT。 |
| `tokens.refresh_token` | `string` | 用于刷新访问令牌。 |

Codex API Key 登录可以使用更精简的结构：

```json
{
  "OPENAI_API_KEY": "<api-key>"
}
```

本应用的中转站账户仅保存 `OPENAI_API_KEY`，不写入 `auth_mode`，并在 `config.toml` 中设置：

```toml
model_provider = "relay"

[model_providers.relay]
name = "Relay"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
```

中转站地址必须兼容 Responses API。切换回 OAuth 账户时，应用会删除顶层
`model_provider`、`openai_base_url` 以及整个 `model_providers` 配置，其他设置和注释会保留。

默认文件路径为 `~/.codex/auth.json`。该文件包含完整认证凭据，不得提交到代码仓库或公开分享。
使用 HTTP 中转地址时，API Key 可能通过明文网络传输，应优先使用 HTTPS。
