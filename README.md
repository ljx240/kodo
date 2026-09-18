# Kodo

极简 Coding Agent 桌面工作台：**项目 → 会话 → Agent 执行轨迹 → 最终回答 → 变更文件**。

Rust Core + Agent + Tauri/React GUI。GUI 服务项目级对话、执行过程与代码变更，而不是通用后台。

## 功能

- 左侧单一项目树（项目下直接展开会话）
- 对话内嵌 Agent 轨迹（思考 / 搜索 / 读文件 / 命令 / 模型 / 写文件）
- 可折叠 Inspector（摘要、变更文件、工具、LLM）
- Response Trace / Archive / Settings
- 多 AI Provider（Anthropic / OpenAI / DeepSeek / 自定义兼容端点）
- **Secure by Default**：默认 `ask` 权限，危险命令与写文件需审批

## 快速开始

### 依赖

- Node.js 20+、npm
- Rust（[rustup](https://rustup.rs)）
- macOS / Linux（Windows 未作为一等目标）

### 启动

```bash
./start.sh          # Tauri 桌面应用（会编译 Rust）
./start.sh --ui     # 仅浏览器界面（跳过 Rust，适合调 UI）
```

演示路由（确定性 fixture，供视觉回归）：

| 路径 | 页面 |
|---|---|
| `/ui-demo/conversation` | 对话（Inspector 展开） |
| `/ui-demo/conversation?inspector=closed` | 对话（折叠） |
| `/ui-demo/trace` | Response Trace |
| `/ui-demo/archive` | Archive |
| `/ui-demo/settings` | Settings |

参考视口：`1586×992`。

## 架构

| 层 | 路径 | 职责 |
|---|---|---|
| Core | `crates/core` | 项目/会话/设置的 append-only 日志 |
| Agent | `crates/agent` | 工具循环、可选 LLM HTTP、写文件与验证 |
| Shell | `apps/desktop/src-tauri` | Tauri 命令、run 线程、审批 |
| GUI | `apps/desktop/src` | React + Vite |

本地状态目录（macOS）：`~/Library/Application Support/Kodo`

- `settings.log` — 普通设置（无密钥）
- `credentials.log` — API Key（权限 `0600`）
- `projects.log` / `sessions/` — 项目列表与会话日志

## AI Provider

**Settings → AI Provider** 添加服务商并保存 Key。

- 界面只显示掩码（`••••abcd`），不回传明文
- 未配置 Key 时仍会做本地扫描，并诚实说明未调用模型

## 权限

| 档位 | 行为 |
|---|---|
| `ask`（默认） | 每个 shell 命令与写文件都需要审批 |
| `auto` | 普通命令自动执行；危险命令与写文件需审批 |
| `full` | 项目内工作自动执行（不推荐） |

危险命令示例：`rm`、`sudo`、`git push`、管道下载执行等。

## 开发

```bash
# Rust
cargo test -p kodo-core -p kodo-agent

# 前端
cd apps/desktop
npm install
npm run typecheck
npx playwright test

# 有意改 UI 后更新视觉基线
npx playwright test --update-snapshots
```

设计规范见 `docs/design/`（`CLAUDE.md`、`LAYOUT.md`、`UI_ACCEPTANCE.md` 等）。

## License

MIT — 见 [LICENSE](./LICENSE)。
