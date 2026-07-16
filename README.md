# Cortana

本地 Tauri 桌面工具，用于保存多个 Codex CLI `auth.json` 档案，并将选中的档案写回 Codex 使用的认证目录。账户既可以使用 ChatGPT OAuth，也可以使用提供 API Key 和 Responses API 地址的中转站。

## 开发环境

通用要求：

- Node.js 22
- pnpm 11.8.0
- Rust 1.93.0
- 支持 `app-server` 的 Codex CLI（会话管理功能需要）

系统依赖：

- Windows：Visual Studio Build Tools 2022，安装“使用 C++ 的桌面开发”和 Windows SDK；Rust 使用 `x86_64-pc-windows-msvc`；安装 WebView2 Runtime。
- macOS：运行 `xcode-select --install` 安装 Xcode Command Line Tools。
- Debian/Ubuntu：

```sh
sudo apt update
sudo apt install build-essential curl wget file libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

安装依赖并启动：

```sh
corepack enable
pnpm install --frozen-lockfile
pnpm dev
```

仅启动 Web 前端：

```sh
pnpm --filter @cortana/desktop dev:web
```

Web 访问默认关闭，可在桌面应用的“设置 → 常规 → Web 访问”中启用，默认地址为 `http://127.0.0.1:11456`。开发时需运行 `pnpm dev` 以同时启动桌面后端，再访问 Vite 提供的 Web 链接。生产环境可从托盘菜单“在浏览器中打开”进入；首次进入会自动完成本机访问授权。

应用默认使用 `~/.codex/auth.json`。可在设置中改为其他 Codex 主目录。所有档案仅保存在本机用户级 SQLite 数据库，包含可用认证令牌或 API Key；请勿把该数据库上传到同步盘或代码仓库。中转站没有统一额度接口，因此应用不会查询或展示其额度。

相关文档：

- [Codex auth.json 结构](docs/auth-json.md)
- [OpenAI 接口清单](docs/openai-interfaces.md)
- [常见开发问题](docs/common-development-issues.md)

## 构建

构建必须在目标操作系统上执行。产物位于 `src-tauri/target/release/bundle/`。

当前系统的安装包：

```sh
pnpm build
```

Windows x64 MSI：

```sh
pnpm tauri build --target x86_64-pc-windows-msvc --bundles msi
```

macOS universal `.pkg`：

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
sh scripts/package-macos.sh
```

正式发布前还需配置对应平台的代码签名；当前 CI 发布目标为 Windows x64 和 macOS arm64。
