# Pi 账号切换一期设计

- 状态：已确认设计
- 日期：2026-09-05
- Cortana 基线：`cf6b191ea3216eeea3b4cae9244f4293482efcf6`（v0.3.9）
- Pi 参考基线：`da840b6216578c2a571d0374ac6a2091a83f9d91`（2026-09-05 `main`）

## 1. 目标

在 Cortana 中新增 Pi 产品支持，一期只实现 **Pi 的 OpenAI Codex（ChatGPT Plus/Pro OAuth）多账号保存、同步和切换**。

一期完成后，用户可以：

1. 在 Pi 中通过 `/login` 登录 OpenAI Codex。
2. 在 Cortana 中把 Pi 当前 `openai-codex` 凭据同步为一个本地账号档案。
3. 重复上述过程保存多个 ChatGPT 账号。
4. 在 Cortana 中点击账号即可切换 Pi 的 `openai-codex` credential。
5. 切换时不会覆盖 Pi `auth.json` 中 Anthropic、GitHub Copilot、Gemini 等其他 Provider 的 credential。
6. Pi 自己刷新当前 OAuth token 后，Cortana 能在读取状态或下一次切换前把最新 credential 同步回对应档案，避免保存过期 token 快照。

## 2. 设计依据

### 2.1 Cortana 当前架构

Cortana 当前已经以 `AccountProduct` 区分 Codex、Claude、Antigravity、Grok，并通过统一的 `get_app_status(product)`、`switch_profile(product, profile_id)`、`import_current_profile(product)` 等命令分发到各产品适配层。

后端产品代码位于：

```text
src-tauri/src/products/
  antigravity/
  claude/
  codex/
  grok/
```

账号存储统一落在 `accounts` SQLite 表，通过 `product` 区分产品。

Codex 现有切换逻辑已经具备本设计需要复用的关键语义：

- 真实认证文件是运行态事实来源。
- 能识别 Cortana 之外发生的登录状态变化。
- 正常切换发现未管理外部状态时先阻止覆盖。
- 写文件后再提交数据库状态。
- 数据库提交失败时恢复原文件。

Pi 应延续这套语义，而不是建立另一套独立账号系统。

### 2.2 Pi 当前认证存储

Pi 当前 `AuthStorage` 的认证文件是一个 Provider 到 Credential 的 JSON Map：

```ts
type AuthStorageData = Record<string, Credential>;
```

默认文件路径为：

```text
~/.pi/agent/auth.json
```

并支持通过：

```text
PI_CODING_AGENT_DIR
```

修改 agent 配置目录。

`openai-codex` 当前 OAuth credential 的稳定字段为：

```json
{
  "type": "oauth",
  "access": "...",
  "refresh": "...",
  "expires": 0,
  "accountId": "..."
}
```

Pi 的 OpenAI Codex OAuth 当前只保证以上 credential 数据；一期不得把 email、plan 等未保证存在的 JWT 信息作为账号主身份或必填展示字段。

Pi 当前认证存储还具有两个重要行为：

1. 使用 `proper-lockfile` 对 `auth.json` 加锁后再执行 read-modify-write。
2. 读取 credential 时会比较文件 revision，文件发生变化后重新加载最新数据。

因此 Cortana 不能直接裸写 `auth.json`，必须与 Pi 的锁语义兼容，并且每次切换只能更新目标 Provider 字段。

参考：

- Pi `AuthStorage`: https://github.com/earendil-works/pi/blob/da840b6216578c2a571d0374ac6a2091a83f9d91/packages/coding-agent/src/core/auth-storage.ts
- Pi OpenAI Codex OAuth: https://github.com/earendil-works/pi/blob/da840b6216578c2a571d0374ac6a2091a83f9d91/packages/ai/src/auth/oauth/openai-codex.ts
- Pi agent dir: https://github.com/earendil-works/pi/blob/da840b6216578c2a571d0374ac6a2091a83f9d91/packages/coding-agent/src/config.ts

## 3. 一期范围

### 3.1 包含

- 新增 `pi` AccountProduct。
- Pi 产品入口和产品图标。
- Pi `auth.json` 路径解析。
- `openai-codex` OAuth credential 解析和校验。
- 同步 Pi 当前登录态。
- 保存多个 Pi OpenAI Codex 账号。
- 切换 Pi OpenAI Codex 账号。
- 账号别名编辑。
- 账号排序。
- 账号删除。
- 工具外登录变化检测。
- 当前 Pi token 被 Pi 自己刷新后的反向同步。
- 与 Pi `proper-lockfile` 兼容的并发保护。
- 保留 `auth.json` 中其他 Provider 的数据。
- 默认路径和继承到 Cortana 进程中的 `PI_CODING_AGENT_DIR`。

### 3.2 不包含

- Cortana 内直接发起 Pi OAuth 登录。
- Pi Anthropic 账号切换。
- Pi GitHub Copilot 账号切换。
- Pi Gemini / Antigravity / 其他 Provider 账号切换。
- Pi API Key Provider 管理。
- Pi 中转站管理。
- Pi 模型配置。
- Pi 会话管理。
- Pi 提示词管理。
- Pi 统计分析。
- Pi 使用量 / 额度查询。
- Cortana 主动刷新 Pi OAuth token。
- 从 Cortana 删除账号时自动让 Pi `/logout`。
- GUI 设置页中的 Pi agent dir 自定义输入框。

## 4. 核心架构决策

### 4.1 Pi 是一个 Product

一期继续沿用现有产品维度：

```text
AccountProduct
├── codex
├── claude
├── antigravity
├── grok
└── pi
```

Pi 不作为 Codex 的子模式处理。

原因：

- Codex CLI 与 Pi 是两个独立运行时。
- 两者的认证文件、credential 生命周期和当前激活状态彼此独立。
- 用户可能同时处于 `Codex=A`、`Pi=B`。
- 后续 Pi 还会拥有多个 Provider，不能把它设计为 Codex 的别名。

### 4.2 Pi profile 是 Provider credential，不是整份 auth.json 快照

Cortana 中一个 Pi 账号档案只保存某个 Provider 的 credential。

一期唯一 Provider：

```text
product      = pi
provider_key = openai-codex
```

Cortana 数据库中的 `auth_json` 对 Pi 保存的是：

```json
{
  "type": "oauth",
  "access": "...",
  "refresh": "...",
  "expires": 0,
  "accountId": "..."
}
```

而不是：

```json
{
  "openai-codex": { "...": "..." },
  "anthropic": { "...": "..." }
}
```

这条规则是一期最重要的数据边界。

### 4.3 Pi auth.json 始终是运行态事实来源

Cortana 数据库是账号档案库，不是 Pi 当前运行态的唯一真相。

每次 `get_app_status(pi)` 和 `switch_profile(pi, ...)` 都必须重新观察磁盘上的 `openai-codex` credential。

不能只依赖数据库中的 `active_profile_id`。

### 4.4 一期不复用 Codex credential

即使 Pi `openai-codex` 和 Codex 当前使用同一 OpenAI OAuth Client ID，也不把 Codex profile 的 credential 直接共享给 Pi。

一期中两种 profile 独立存储：

```text
Codex profile A
Pi/openai-codex profile A
```

它们可以代表同一个 ChatGPT accountId，但 credential 生命周期独立。

理由：

- 两个 CLI 都可能刷新 OAuth credential。
- 共享 refresh token 会把两个产品的更新时序耦合起来。
- 一期目标是稳定切换，不是 credential 去重。

## 5. 数据模型

### 5.1 accounts.provider_key

在 `accounts` 表增加：

```sql
provider_key TEXT NOT NULL DEFAULT ''
```

旧产品使用空字符串：

```text
codex       -> ''
claude      -> ''
antigravity -> ''
grok        -> ''
```

Pi 一期使用：

```text
pi -> 'openai-codex'
```

选择 `NOT NULL DEFAULT ''` 而不是 nullable，是为了：

- 现有记录迁移无需回填。
- SQLite 唯一索引不会受到 `NULL` 可重复语义影响。
- 后续可以直接把 Provider 纳入组合唯一键。

### 5.2 ProfileSummary / Profile

后端 `ProfileSummary` 增加：

```rust
provider_key: String
```

前端 `Profile` 增加：

```ts
providerKey: string
```

对非 Pi 产品该值为空字符串。

### 5.3 唯一索引

现有非 Codex OAuth 唯一索引只按 `(product, account_id)` 和 `(product, email)`，加入 Pi 后应把 Provider 纳入唯一性。

调整为：

```sql
CREATE UNIQUE INDEX accounts_oauth_account_identity_uq
ON accounts(product, provider_key, account_id)
WHERE product <> 'codex'
  AND account_type = 'oauth'
  AND account_id <> '';
```

email 索引同样加入 `provider_key`：

```sql
CREATE UNIQUE INDEX accounts_oauth_email_identity_uq
ON accounts(product, provider_key, email COLLATE NOCASE)
WHERE product <> 'codex'
  AND account_type = 'oauth'
  AND email <> '';
```

Codex 专用 `(account_id, chatgpt_user_id)` 唯一索引保持不变。

Relay 唯一索引也应加入 `provider_key`，旧产品默认空字符串，因此行为不变；这样后续 Pi API Key Provider 不需要再次修改数据库唯一键。

### 5.4 Pi OpenAI Codex identity

一期唯一稳定账号身份：

```text
provider_key + accountId
```

即：

```text
('openai-codex', credential.accountId)
```

不使用 access token 指纹作为账号身份，因为 access token 会刷新。

不使用 refresh token 指纹作为账号身份，因为 refresh token 也可能轮换。

### 5.5 Pi alias

新导入 Pi profile 时：

1. 如果同一 `(provider_key, accountId)` 已存在，保留原 alias。
2. 如果不存在，默认 alias 使用：

```text
OpenAI Codex · <accountId 尾部 6~8 位>
```

用户可随后编辑 alias。

一期不要求从 token 推导 email 或昵称。

## 6. Pi 产品适配层

新增：

```text
src-tauri/src/products/pi/
  mod.rs
  auth.rs
  accounts.rs
```

### 6.1 auth.rs

只负责 Pi 文件协议，不负责 Cortana UI 语义。

建议职责：

```text
pi_agent_dir(state)
pi_auth_path(state)
read_auth_file_locked(...)
with_pi_auth_lock(...)
parse_auth_storage(...)
parse_openai_codex_credential(...)
merge_openai_codex_credential(...)
write_auth_file_safely(...)
restore_auth_file(...)
```

定义常量：

```text
PI_PROVIDER_OPENAI_CODEX = "openai-codex"
PI_AGENT_DIR_ENV = "PI_CODING_AGENT_DIR"
```

### 6.2 accounts.rs

负责 Cortana 账号语义。

建议职责：

```text
app_status(...)
import_current_profile(...)
switch_profile(...)
update_profile_alias(...)
find_profile_for_current_credential(...)
sync_current_credential(...)
profile_summary(...)
```

### 6.3 mod.rs

只导出一期需要的公共入口，避免让通用 `features/accounts` 直接依赖 Pi 内部文件实现。

## 7. Pi auth.json 路径解析

一期路径规则与 Pi 当前实现保持一致：

1. 如果 Cortana 进程环境存在非空 `PI_CODING_AGENT_DIR`，使用该目录。
2. 否则使用当前用户 home 下：

```text
~/.pi/agent
```

3. auth 文件固定为：

```text
<agent-dir>/auth.json
```

需要在状态卡中显示最终解析后的完整 auth path，方便用户确认 Cortana 操作的是哪份 Pi 配置。

一期不增加额外 Cortana `pi_agent_dir` 设置项。对于 GUI 启动时没有继承 shell 环境变量的自定义 Pi 目录，一期明确不做额外 shell 环境探测；这不影响默认路径用户的核心账号切换。

## 8. Credential 校验

一期只接受：

```json
{
  "type": "oauth",
  "access": "non-empty string",
  "refresh": "non-empty string",
  "expires": "finite number",
  "accountId": "non-empty string"
}
```

规则：

- `type !== "oauth"`：视为当前 Provider 状态不受一期支持。
- 缺少 `access`、`refresh`、`expires`、`accountId`：视为无效 credential。
- `expires` 已过期不代表档案无效，因为 Pi 可以使用 refresh token 刷新。
- 不由 Cortana 主动调用 OpenAI refresh endpoint。
- 不依赖 credential 中不存在的 email/plan 字段。

## 9. 与 Pi 的文件锁兼容

### 9.1 为什么不能使用普通 flock

Pi 当前通过 `proper-lockfile` 对 `auth.json` 做进程间锁。

Cortana 如果仅使用平台 `flock` / Windows file lock，Pi 不会感知该锁，仍然可能并发执行 read-modify-write。

因此 Cortana 必须实现与 `proper-lockfile` 可互操作的锁协议，而不是单独选择另一种锁机制。

### 9.2 锁行为

实现一个 Pi 专用锁 helper，语义与 Pi 当前使用保持一致：

- 锁目标是 `auth.json`。
- 使用 `proper-lockfile` 对应的 sidecar lock path / directory 语义。
- 获取锁失败时重试。
- 采用约 30 秒 stale 边界，与 Pi 当前 async lock 配置一致。
- 等待必须运行在 `spawn_blocking` 或等价后台阻塞线程中，不能阻塞 Tauri 主线程。
- 所有读取最新 credential、同步当前 profile、切换写文件操作都在同一把锁内完成。
- 无论成功或失败都必须释放锁。

具体 lock directory 命名和 stale 判定应通过与真实 Pi 的互操作测试固定，而不是凭假设实现。

## 10. 文件写入规则

一次 Pi credential 写入必须是：

```text
lock
  -> read latest auth.json
  -> parse full JSON object
  -> replace only ["openai-codex"]
  -> write full merged object safely
  -> release lock
```

例如原文件：

```json
{
  "anthropic": { "type": "oauth", "...": "..." },
  "openai-codex": { "type": "oauth", "accountId": "A", "...": "..." },
  "github-copilot": { "type": "oauth", "...": "..." }
}
```

切换到 B 后只能变成：

```json
{
  "anthropic": { "type": "oauth", "...": "..." },
  "openai-codex": { "type": "oauth", "accountId": "B", "...": "..." },
  "github-copilot": { "type": "oauth", "...": "..." }
}
```

禁止：

- 用数据库中的单个 credential 直接覆盖整份 `auth.json`。
- 修改其他 Provider 字段。
- 在无法解析原 JSON 时强制生成新文件覆盖旧内容。

### 10.1 文件安全

- 新创建 auth 文件在 Unix 上使用 `0600`。
- 新创建 agent 目录在 Unix 上使用 `0700`。
- 原文件已存在时，不主动扩大权限。
- 写入前把原始文件内容保存在内存备份中。
- 优先使用同目录临时文件 + replace/rename 的安全写入方式；平台差异由 helper 封装。
- 写入后若数据库事务提交失败，在仍持有 Pi 文件锁时恢复原始文件。

## 11. 状态模型

沿用现有：

```text
managed
unmanaged
missing
```

### 11.1 managed

Pi `auth.json["openai-codex"]` 是受支持的 OAuth credential，并且 `(provider_key, accountId)` 能匹配一个 Cortana Pi profile。

返回：

- `isActive = true`
- `detectedProfile = null`
- `authState.kind = managed`

如果磁盘 credential 与数据库中该 profile 的 `auth_json` 不同，说明 Pi 可能已经刷新 token；在返回状态前把最新 credential 更新回数据库，但不修改 alias、排序等用户数据。

### 11.2 unmanaged

存在受支持的 `openai-codex` OAuth credential，但 Cortana 没有对应 `(provider_key, accountId)` profile。

返回：

- `profiles`：已保存 Pi profiles
- `detectedProfile`：由磁盘 credential 生成的临时 profile
- `authState.kind = unmanaged`

UI 提供：

```text
同步当前账号
```

### 11.3 missing

以下情况视为 missing：

- `auth.json` 不存在。
- `auth.json` 中没有 `openai-codex`。

如果 `openai-codex` 存在但 credential 类型或结构不受一期支持，则使用明确错误/unsupported 状态文案，不能伪装为普通 missing 后静默覆盖。

## 12. 同步当前 Pi 登录

调用现有统一命令：

```text
import_current_profile(product = pi)
```

流程：

```text
获取 Pi 文件锁
  -> 读取最新 auth.json
  -> 读取 openai-codex
  -> 校验 OAuth credential
  -> 获取 accountId
  -> 查找 (pi, openai-codex, accountId)
      -> 已存在：更新 auth_json，保留 alias/sort_order
      -> 不存在：创建 profile
  -> 返回 profile
  -> 释放锁
```

创建记录：

```text
product              = pi
provider_key          = openai-codex
account_type          = oauth
account_id            = credential.accountId
chatgpt_user_id       = ''
email                 = ''
plan_type             = ''
auth_json             = credential JSON
api_base_url          = NULL
usage_*               = NULL
model_profile_id      = NULL
default_model_id      = NULL
upstream_protocol     = default value
upstream_auth_mode    = default value
anthropic_max_tokens  = default value
```

不需要调用额度接口，也不需要主动 refresh OAuth。

## 13. Pi 账号切换事务

调用现有统一命令：

```text
switch_profile(product = pi, profile_id, force)
```

必须按照以下事务顺序：

```text
1. 获取 Pi auth 文件锁
2. 读取并保留整份原 auth.json
3. 解析当前 openai-codex credential
4. 如果当前 credential 属于已管理 profile：
     把磁盘最新 credential 同步回该 profile
5. 如果当前 credential 是未管理账号并且 force = false：
     返回“检测到工具外 Pi 登录变化”，不写文件
6. 在同一个数据库事务中读取目标 profile
7. 校验目标：
     product = pi
     provider_key = openai-codex
     account_type = oauth
     credential 合法
8. 生成 merged auth.json：
     原 JSON 所有字段保持不变
     仅替换 openai-codex
9. 安全写入 Pi auth.json
10. 更新目标 profile last_used_at / updated_at
11. 提交数据库事务
12. 如果数据库提交失败：
      恢复第 2 步原始文件
13. 返回最新 ProfileSummary
14. 释放 Pi 文件锁
```

### 13.1 force 的语义

如果用户在 Cortana 外通过 Pi `/login` 登录了一个未保存账号：

普通切换必须阻止覆盖，并提示：

```text
检测到工具外的 Pi OpenAI Codex 登录变化。
请先同步当前账号，或确认后强制切换。
```

用户明确确认“强制切换”后才允许替换当前 `openai-codex` credential。

这与现有 Codex 的外部变更保护保持一致。

### 13.2 openai-codex 缺失时

如果 auth 文件存在，但没有 `openai-codex`：

- 不视为外部冲突。
- 允许直接把目标 profile 作为新的 `openai-codex` 字段写入。
- 其他 Provider 保持不变。

## 14. Pi token 反向同步

这是一期必须实现的可靠性功能。

场景：

```text
Cortana 保存账号 A
  -> Pi 使用 A
  -> access token 过期
  -> Pi 用 refresh token 刷新
  -> Pi 把新的 access/refresh/expires 写回 auth.json
```

如果 Cortana 仍保存旧 credential，之后切换到 B，再切回 A，可能把旧 token 写回 Pi。

因此必须在两个位置同步当前 credential：

1. `get_app_status(pi)`：当前磁盘 accountId 匹配已保存 profile 时，把磁盘 credential 更新回数据库。
2. `switch_profile(pi, ...)`：在替换当前 credential 之前，再在锁内做一次同步。

第 2 条是最终防线，不能只依赖 UI 定时刷新。

Cortana 一期只“吸收 Pi 已刷新后的 credential”，不自己执行 refresh。

## 15. 删除账号语义

删除 Pi profile 只删除 Cortana 中的保存档案，不主动修改 Pi `auth.json`。

如果删除的是当前 Pi 账号：

```text
删除前：managed
删除后：unmanaged
```

理由：

- “从 Cortana 删除档案”不等于“让 Pi 登出”。
- 自动删除 `openai-codex` 会产生超出用户预期的运行时副作用。
- 用户仍可重新点击“同步当前账号”恢复管理。

## 16. 前端产品能力模型

Pi 接入后不应继续在页面中大量追加：

```ts
if (product === 'pi')
```

新增一个轻量产品 capability 定义，至少覆盖账号页面和导航所需能力。

概念结构：

```ts
type ProductCapabilities = {
  accountAdd: boolean;
  importCurrent: boolean;
  accountRefresh: boolean;
  usage: boolean;
  relay: boolean;
  models: boolean;
  sessions: boolean;
  analytics: boolean;
  prompts: boolean;
  config: boolean;
  gateway: boolean;
  openCliFromAccount: boolean;
  showAuthPath: boolean;
};
```

Pi 一期：

```text
accountAdd          false
importCurrent       true
accountRefresh      false
usage               false
relay               false
models              false
sessions            false
analytics           false
prompts             false
config              false
gateway             false
openCliFromAccount  false
showAuthPath        true
```

现有产品能力按当前行为填写，不改变功能。

能力模型只解决本次 Pi 会直接遇到的条件分支，不做无关大规模前端重构。

## 17. 产品导航

产品菜单增加：

```text
Pi
```

Pi 一期只开放：

```text
账号
设置（全局）
```

隐藏：

```text
统计分析
会话管理
提示词管理
自定义模型
配置
```

原因：这些页面当前有明确的产品后端实现，Pi 一期没有对应数据源。不能只把 Pi 加入 `AccountProduct` 后继续展示会报错或产生空状态的页面。

切换到 Pi 时统一导航到：

```text
/accounts
```

如果未来通过旧 URL / 深链接进入 Pi 不支持的产品页面，应回到 `/accounts`，而不是调用不存在的 Pi backend。

## 18. Pi 账号页 UX

### 18.1 页面头部

Pi 页面不显示：

- 网关模式。
- “刷新全部”。
- 通用“添加账号” OAuth / Relay 弹窗。

主操作使用：

```text
同步当前账号
```

如果磁盘没有 `openai-codex` credential，该按钮可以保留但 disabled，并提示：

```text
请先在 Pi 中运行 /login 并登录 OpenAI (ChatGPT Plus/Pro)。
```

### 18.2 当前状态卡

显示：

```text
当前 Pi 账号
<alias 或 未纳入管理>
OpenAI Codex
<短 accountId>
~/.pi/agent/auth.json
```

状态：

```text
已由 Cortana 管理
未纳入 Cortana 管理
尚未检测到 OpenAI Codex 登录
```

Pi 不显示额度进度组件。

### 18.3 账号列表

每行稳定展示：

```text
alias
OpenAI Codex · <短 accountId>
使用中（如果 active）
```

操作：

```text
切换
编辑别名
删除
```

不显示：

```text
刷新额度
更新登录令牌
重置卡
额度详情
打开 Codex 终端
模型方案
中转站信息
```

### 18.4 Empty State

Pi 没有保存账号时，不打开现有 `AddAccountDialog`。

Empty State 文案：

```text
尚未保存 Pi 账号
先在 Pi 中使用 /login 登录 OpenAI (ChatGPT Plus/Pro)，
然后回到 Cortana 同步当前账号。
```

操作按钮：

```text
同步当前账号
```

## 19. useAccountManager 调整原则

`useAccountManager` 当前已经包含 Codex Gateway、模型方案、额度、Grok Relay、OAuth 等大量产品条件。

Pi 一期不应该把这些流程复制一份。

保留可直接复用的通用状态：

- `status`
- `loading`
- `busy`
- `refresh()`
- `activeProfile`
- `switchTo()`
- `importCurrent()`
- `reorderProfiles()`
- `openEditor()` 的 alias-only 路径
- `deleteProfile()`

通过 capability 禁止 Pi 触发：

- `refreshGateway`
- `refreshModels`
- `refreshAllAccounts`
- `refreshAccount`
- `generateOAuthLink`
- `submitAdd`
- relay flows
- quota / reset credit flows

后端即使收到一期不支持的 Pi 命令，也必须返回明确的“不支持”错误，不能依赖前端完全阻止。

## 20. 后端统一命令分发

`AccountProduct` 增加 `Pi` 后，所有 exhaustive match 必须显式处理。

核心分发：

```text
get_app_status(Pi)        -> pi::app_status
switch_profile(Pi)        -> pi::switch_profile
import_current_profile(Pi)-> pi::import_current_profile
```

其他命令：

```text
add_relay_profile(Pi)     -> 明确拒绝
refresh_profile_usage(Pi) -> 明确拒绝
OAuth add(Pi)             -> 明确拒绝
get_profile_auth(Pi)      -> 一期不暴露完整 credential 编辑，明确拒绝
```

Pi alias 编辑走产品专用 alias update，不能把现有 Codex `auth.json` 编辑表单误用于 Pi credential。

`active_product()` 需要识别：

```text
"pi" -> AccountProduct::Pi
```

## 21. CLI 检测

Pi 一期账号切换只需要文件，不要求 Cortana 能直接启动 Pi CLI。

因此产品页面不能因为 `pi` executable 未被 Cortana GUI PATH 检测到就禁止账号切换。

推荐一期策略：

- Pi 产品关闭通用 `CliAlert`。
- 账号页依据 Pi auth path / credential 状态判断功能是否可用。
- 不新增“使用该账号打开 Pi 终端”。

这样可以避免 npm/fnm/pnpm 等全局安装路径在桌面 GUI 环境下造成误报，也保持一期范围聚焦。

## 22. 错误处理

需要可区分以下错误：

### 22.1 auth.json JSON 损坏

```text
Pi auth.json 不是有效的 JSON，已停止操作以避免覆盖其他 Provider 凭据。
```

绝不自动重置为 `{}`。

### 22.2 openai-codex credential 不支持

```text
当前 Pi openai-codex 凭据格式不受此版本 Cortana 支持。
```

### 22.3 文件锁超时

```text
Pi 正在更新认证信息，暂时无法切换账号，请稍后重试。
```

不绕过锁强写。

### 22.4 工具外账号变化

```text
检测到工具外的 Pi OpenAI Codex 登录变化。
请先同步当前账号，或确认后强制切换。
```

沿用现有 force-switch 确认模式。

### 22.5 数据库失败

如果 auth 文件已经写入但数据库提交失败：

- 在锁内恢复原始 auth 文件。
- 返回数据库错误。
- 不留下“文件已切换、数据库未切换”的半状态。

### 22.6 文件恢复失败

这是高优先级错误，应直接报告：

```text
Pi 账号切换失败，且认证文件自动恢复失败。请立即检查 <authPath>。
```

不能用普通 toast success/info 掩盖。

## 23. 安全要求

- Pi credential 与现有 Cortana auth token 一样，只保存在本机 SQLite。
- 不向日志输出 `access`、`refresh`。
- 不向前端返回完整 Pi credential，除非未来明确实现凭据编辑功能。
- 一期 alias 编辑不需要把 `auth_json` 发到前端。
- 错误消息不得包含完整 credential JSON。
- `accountId` 可以用于 UI 识别，但默认只展示短格式。
- SQLite 继续保持现有 Unix `0600`。
- Pi auth 新文件使用 Unix `0600`。

## 24. 并发与一致性不变量

实现必须始终满足：

1. Cortana 不持有 Pi 文件锁时，不做 Pi auth read-modify-write。
2. 写 Pi credential 时只改变目标 Provider。
3. 当前 managed credential 在被替换前必须先同步回数据库。
4. 数据库不能把 `isActive` 当成 Pi 当前状态的唯一来源。
5. `accountId` 不随 token refresh 改变，token 内容可以改变。
6. 未管理外部状态默认不能被覆盖。
7. 删除 Cortana profile 不等于删除 Pi credential。
8. JSON 损坏时宁可拒绝操作，也不覆盖文件。
9. 一期不主动刷新 Pi OAuth token。
10. Pi 的其他 Provider 对本功能应完全透明。

## 25. 测试设计

### 25.1 Rust 单元测试

至少覆盖：

- 解析有效 `openai-codex` credential。
- 缺字段 credential 被拒绝。
- API key 类型被一期拒绝。
- full auth JSON merge 只修改 `openai-codex`。
- 其他 Provider 深度内容在 merge 后保持不变。
- 不存在 `openai-codex` 时可插入。
- 无效 JSON 不写文件。
- 默认路径解析。
- `PI_CODING_AGENT_DIR` 路径解析。
- Pi profile upsert 按 `(provider_key, accountId)` 去重。
- token 内容变化但 accountId 相同时更新原 profile，而不是新增 profile。
- alias 在重新同步时保持不变。
- active profile 从磁盘状态解析。
- 删除 active profile 后磁盘不变、状态变 unmanaged。

### 25.2 切换事务测试

至少覆盖：

- managed A -> B。
- missing provider -> B。
- unmanaged D -> B 普通切换被阻止。
- unmanaged D -> B force 成功。
- 切换前把 A 在磁盘上的新 token 同步回数据库。
- 写文件失败时数据库不提交。
- 数据库提交失败时 auth 文件恢复。
- 切换 B 后其他 Provider 完全不变。

### 25.3 锁互操作测试

需要真实验证 Cortana 锁与 Pi 当前 `proper-lockfile` 的互操作，而不是只测 Rust helper 自己。

至少两组：

1. Node/Pi 持有 `auth.json` 锁时，Rust/Cortana 不能进入临界区。
2. Rust/Cortana 持有锁时，Node `proper-lockfile` 不能进入临界区。

此测试决定锁实现是否真正兼容 Pi。

### 25.4 前端测试

至少覆盖：

- Pi 产品出现在 ProductMenu。
- Pi 页面只显示支持的导航。
- Pi 不显示 Gateway、Models、Usage、Relay 操作。
- unmanaged 状态显示“同步当前账号”。
- missing 状态显示 `/login` 指引。
- Pi profile 行显示 `OpenAI Codex + 短 accountId`。
- Pi 不显示“额度未查询”。
- force-switch confirmation 可复用现有确认流。

## 26. 预计受影响文件边界

设计允许实施阶段按实际编译依赖微调，但主要改动应限制在以下区域：

```text
apps/desktop/src/components/app-shell-context.ts
apps/desktop/src/components/app-shell.tsx
apps/desktop/src/features/accounts/types.ts
apps/desktop/src/features/accounts/use-account-manager.ts
apps/desktop/src/features/accounts/accounts-page.tsx
apps/desktop/src/features/accounts/account-list.tsx
apps/desktop/src/features/accounts/account-dialog.tsx   # 仅避免 Pi 进入现有 Add/Edit credential flow
apps/desktop/src/assets/pi.svg

src-tauri/src/platform/state.rs
src-tauri/src/platform/db.rs
src-tauri/src/features/accounts/commands.rs
src-tauri/src/features/accounts/store.rs               # 通用 provider_key 查询/summary
src-tauri/src/products/mod.rs
src-tauri/src/products/pi/mod.rs
src-tauri/src/products/pi/auth.rs
src-tauri/src/products/pi/accounts.rs
```

如果 capability 独立成文件，可新增：

```text
apps/desktop/src/products.ts
```

或等价的产品 metadata 文件。

不应为了 Pi 一期修改 Gateway、Codex OAuth、Models、Sessions 等内部实现。

## 27. 验收标准

一期只有在以下条件全部满足时才算完成：

1. Cortana 可选择 Pi 产品。
2. 默认 `~/.pi/agent/auth.json` 中已有 `openai-codex` 时可同步。
3. 能连续保存至少两个不同 `accountId` 的 Pi profile。
4. A -> B -> A 切换成功。
5. 切换后 Pi 后续读取 credential 时看到目标账号。
6. Pi auth 中其他 Provider 在任意切换后字节语义等价，不丢字段。
7. Pi 自己刷新 A token 后，Cortana 不会在之后切回 A 时恢复旧 credential。
8. 外部 `/login` 到未保存账号时，Cortana 能识别 unmanaged。
9. unmanaged 状态下普通切换不会静默覆盖当前登录。
10. force 切换经过明确确认后可执行。
11. Pi 正持有 auth lock 时 Cortana 不并发写。
12. Cortana 正持有 auth lock 时 Pi 不并发写。
13. 无效 auth JSON 时 Cortana 不修改文件。
14. 删除当前 Cortana Pi profile 不会让 Pi 登出。
15. Pi 页面不出现无意义的额度、Relay、Gateway、Models、Sessions 等 UI。
16. 现有 Codex、Claude、Antigravity、Grok 行为不回归。

## 28. 后续扩展方向

一期完成后，Pi 产品模型自然扩展为：

```text
Pi
├── openai-codex
│   ├── Account A
│   └── Account B
├── anthropic
├── github-copilot
├── google-gemini-cli
└── ...
```

后续新增 Provider 时应新增 provider adapter，而不是把整份 Pi `auth.json` 重新变成 profile 快照。

未来可进一步抽象为：

```text
Product
  -> Provider
      -> Credential Profile
```

但一期只引入 `provider_key` 和必要 capability，不提前重构整个 Cortana 账号体系。

## 29. 最终设计结论

一期采用以下基线：

> **Pi 作为独立 AccountProduct；Pi profile 以 Provider credential 为最小管理单位；一期只支持 `openai-codex`；磁盘 auth.json 是运行态真相；Cortana 与 Pi 使用兼容文件锁，在锁内先吸收当前 token 更新，再仅替换目标 Provider，并用数据库事务 + 文件恢复保证一致性。**

该方案最大程度复用 Cortana 当前账号架构，同时不会把 Pi 的多 Provider 能力设计死，也避免因整文件快照、OAuth token 刷新和并发写入导致 credential 丢失或回退。