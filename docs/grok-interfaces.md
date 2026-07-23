# Grok 官方接口清单

本文记录 Cortana 当前使用的 xAI OAuth、用户信息和 Grok Build 额度接口。内容以
[Grok CLI 官方说明](https://docs.x.ai/build/cli/reference)及
[grok-build 官方源码](https://github.com/xai-org/grok-build)为准，最后核对日期为 2026-07-22。

其中 OAuth discovery 遵循标准 OIDC；`cli-chat-proxy.grok.com` 下的用户和额度端点由
xAI 官方 CLI 使用，但不是公开稳定的开发者 API，升级前需要使用真实账号重新验证。

## 概览

| 接口 | 方法 | 用途 | 稳定性 |
| --- | --- | --- | --- |
| `https://auth.x.ai/.well-known/openid-configuration` | `GET` | 获取 Device Code、Token 和 UserInfo 端点 | 标准 OIDC discovery |
| discovery 返回的 `device_authorization_endpoint` | `POST` | 申请 Device Code | 标准 Device Authorization Grant |
| `https://auth.x.ai/oauth2/token` | `POST` | 轮询授权结果、刷新访问令牌 | 标准 OAuth Token 端点 |
| `https://cli-chat-proxy.grok.com/v1/user` | `GET` | 查询 Grok CLI 用户身份 | 官方 CLI 内部接口，可能变更 |
| discovery 返回的 `userinfo_endpoint` | `GET` | 用户身份查询兜底 | 标准 OIDC UserInfo |
| `https://cli-chat-proxy.grok.com/v1/billing?format=credits` | `GET` | 查询 Grok Build 额度和周期 | 官方 CLI 内部接口，可能变更 |

项目使用官方 Grok CLI 客户端 ID：

```text
b1a00492-073a-47ea-816f-4c329264a828
```

OAuth scope：

```text
openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write
```

## Device Code 授权

先读取 discovery，再向其 `device_authorization_endpoint` 提交：

```http
POST <device_authorization_endpoint>
Accept: application/json
Content-Type: application/x-www-form-urlencoded
x-grok-client-version: <cortana_version>
x-grok-client-surface: ui

client_id=b1a00492-073a-47ea-816f-4c329264a828
scope=<上述 OAuth scope>
referrer=grok-build
```

项目使用以下响应字段：

```json
{
  "device_code": "<device_code>",
  "user_code": "ABCD-EFGH",
  "verification_uri": "https://...",
  "verification_uri_complete": "https://...",
  "expires_in": 1800,
  "interval": 5
}
```

随后轮询 token 端点：

```http
POST https://auth.x.ai/oauth2/token
Accept: application/json
Content-Type: application/x-www-form-urlencoded

grant_type=urn:ietf:params:oauth:grant-type:device_code
device_code=<device_code>
client_id=b1a00492-073a-47ea-816f-4c329264a828
```

`authorization_pending` 继续等待，`slow_down` 增加轮询间隔，`access_denied` 和
`expired_token` 终止授权。成功响应使用 `access_token`、`refresh_token`、`id_token`、
`expires_in`。

刷新令牌时使用同一 token 端点：

```text
grant_type=refresh_token
refresh_token=<refresh_token>
client_id=b1a00492-073a-47ea-816f-4c329264a828
```

## 用户信息

优先请求 CLI proxy：

```http
GET https://cli-chat-proxy.grok.com/v1/user
Authorization: Bearer <access_token>
X-XAI-Token-Auth: xai-grok-cli
x-grok-client-version: <cortana_version>
Accept: application/json
```

失败时回退到 discovery 返回的 OIDC `userinfo_endpoint`。项目读取 `sub`、`email`、姓名、
团队和 principal 字段，用于生成账号 ID、邮箱和默认别名。

## Grok Build 额度

xAI 官方 `grok-build` 的 `x.ai/billing` 扩展会请求以下端点。当前实现依据官方仓库提交
[`3af4d5d`](https://github.com/xai-org/grok-build/blob/3af4d5d39897855bdcc74f23e690024a5dc05573/crates/codegen/xai-grok-shell/src/extensions/billing.rs)。

```http
GET https://cli-chat-proxy.grok.com/v1/billing?format=credits
Authorization: Bearer <access_token>
X-XAI-Token-Auth: xai-grok-cli
x-userid: <user_id>
x-grok-client-version: <cortana_version>
Accept: application/json
```

当前响应结构：

```json
{
  "config": {
    "creditUsagePercent": 25.5,
    "currentPeriod": {
      "type": "USAGE_PERIOD_TYPE_WEEKLY",
      "start": "2026-07-22T00:00:00Z",
      "end": "2026-07-29T00:00:00Z"
    },
    "onDemandCap": { "val": 0 },
    "onDemandUsed": { "val": 0 },
    "prepaidBalance": { "val": 0 },
    "isUnifiedBillingUser": true
  },
  "subscriptionTier": "SuperGrok"
}
```

Cortana 当前使用：

| 字段 | 用途 |
| --- | --- |
| `creditUsagePercent` | 已使用额度百分比，界面显示 `100 - creditUsagePercent` |
| `currentPeriod.start/end` | 计算额度周期并显示重置时间 |
| `subscriptionTier` | 套餐名称；缺失时保留本地已有值 |
| `monthlyLimit/used` | 兼容旧版响应，计算已用百分比 |
| `billingPeriodStart/End` | 兼容旧版周期字段 |

`creditUsagePercent` 可能缺失。免费额度没有公开的精确剩余量，Proto3 的零值字段也可能被
省略，因此项目不会把缺失值解释成 `0%` 或 `100%`，而是显示“官方未返回额度百分比”。
请求成功后仍会记录更新时间；达到限制时，模型请求可能返回 `402` 或 `429`。

## 本地凭据

Grok CLI 默认读取 `$GROK_HOME/auth.json`，未设置时使用 `~/.grok/auth.json`。Cortana 只管理
注册键：

```text
https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828
```

额度查询直接使用数据库中该账号的 OAuth 凭据，不切换账号、不启动或解析 Grok CLI
子进程。令牌刷新后，活动账号会同步回 `auth.json`；其他 API Key、企业 OIDC 等条目保持不变。

## 安全约束

- 文档、日志和错误信息不得输出完整 token、Device Code 或 refresh token。
- 授权地址必须使用 HTTPS；轮询必须支持取消、过期和拒绝。
- `auth.json` 与 SQLite 数据库包含可用凭据，不得提交到仓库。
- 私有接口失败时不得更新额度时间戳，响应字段缺失时不得伪造额度值。
