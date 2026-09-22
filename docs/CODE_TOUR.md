# Kodo 代码讲解（Code Tour）

> 本文档由代码走读生成，讲解项目各层的结构与协作方式。
> 快速开始与功能说明见根目录 `README.md`。

## 1. 仓库结构（Cargo workspace）

根 `Cargo.toml` 定义 workspace 成员：

| 成员 | 路径 | 职责 |
|---|---|---|
| `kodo-core` | `crates/core` | 项目/会话/设置的 append-only 日志 |
| `kodo-agent` | `crates/agent` | Agent 工具循环、LLM 调用、写文件与验证 |
| Tauri shell | `apps/desktop/src-tauri` | Tauri 命令、run 线程、审批（approval） |
| `evals` | `evals` | 评测 |
| GUI | `apps/desktop/src` | React + Vite 前端（不参与 Rust 编译） |

## 2. Core 层：`crates/core`

职责：把项目、会话、设置都建模为 **append-only 日志**（只追加、不修改）。

- `session.rs` — 会话日志。核心抽象：`Item` / `ItemKind` / `Phase` / `Status`，
  会话中的每一步都是一个日志条目。
- `settings.rs` — 设置日志；普通设置与密钥分离存储。
- `workspace.rs` / `line.rs` — 工作区与日志行解析。
- `lib.rs` — 对外导出。

本地状态目录（macOS）：`~/Library/Application Support/Kodo`

- `settings.log` — 普通设置（无密钥）
- `credentials.log` — API Key（权限 `0600`）
- `projects.log` / `sessions/` — 项目列表与会话日志

## 3. Agent 层：`crates/agent`

工具循环与策略实现：

- `tools.rs` — 工具定义（搜索、读文件、写文件、跑命令等）。
- `process.rs` / `protocol.rs` — 进程执行与事件协议
  （`SinkEvent`、`Step`、`StepKind`，推送给 GUI 渲染轨迹）。
- `provider.rs` — 多 AI Provider（Anthropic / OpenAI / DeepSeek / 自定义兼容端点）。
  未配置 Key 时仍做本地扫描，并诚实说明未调用模型。
- `classify.rs` — 危险命令分类（`rm`、`sudo`、`git push`、管道下载执行等）。
- `checkpoint.rs` / `verify.rs` / `patch.rs` — 检查点、验证、补丁处理。
- `skill.rs` / `plan.rs` / `state.rs` — 技能流程与计划状态
  （acceptance-first 工作流，验收标准驱动）。
- `repomap.rs` / `evidence.rs` / `context.rs` — 仓库地图、证据收集、上下文构建。
- `lib.rs` — 导出 `FileDelta`、`SinkEvent`、`Step`、`StepKind`、`Permission` 等。

## 4. Shell 层：`apps/desktop/src-tauri`

核心是 `src/run.rs`（约 574 行）的**运行驱动器**：线程、日志写入、审批会合点。

- **`Runs`** — 用 `HashSet<String>` 跟踪活跃 run，防止同一会话重复启动。
- **`Approvals`** — 审批机制核心：
  - 工作线程调用 `wait_point(session, step)` 创建 mpsc 通道并阻塞；
  - GUI 用户决定后调用 `resolve(session, step, choice)` 唤醒线程。
- **`ApprovalChoice`** — 三档：`Deny` / `AllowOnce` / `AllowSession`
  （会话级记住命令指纹，codex 风格渐进式允许）。
- **关键设计**：步骤**先写 started 再执行、完成时写 completed**，
  所以被杀掉的 run 会在日志中留下真实的 `running` 信封，状态不丢失。
- 权限三档：`ask`（默认，全部审批）/ `auto`（普通命令自动，危险命令与写文件审批）/
  `full`（项目内自动，不推荐）。

`view.rs` 提供序列化给前端的视图结构：`ProjectView`、`SessionRefView`、`Workspace`、
`ItemView`、`RunEvent`。

## 5. GUI 层：`apps/desktop/src`

React + Vite：

- `App.tsx` / `routes.ts` / `main.tsx` — 入口与路由，含 `/ui-demo/*` 演示路由
  （确定性 fixture，供 Playwright 视觉回归；参考视口 `1586×992`）。
- `conversation/` — 对话视图，内嵌 Agent 轨迹
  （思考 / 搜索 / 读文件 / 命令 / 模型 / 写文件）。
- `inspector/` — 可折叠 Inspector（摘要、变更文件、工具、LLM）。
- `pages/` — Response Trace / Archive / Settings 页面。
- `api.ts` — 与 Tauri 后端桥接。
- `shell/` / `data/` — 布局外壳与 fixture 数据。
- `styles/` — 样式（含 `conversation.css`）。

## 6. 端到端数据流

```
GUI 发起 run
  → Tauri run.rs 起线程，写入 "started" 信封
  → kodo-agent 执行工具循环，每步通过 SinkEvent 推给前端渲染轨迹
  → 遇到危险操作：在 Approvals 会合点阻塞，等用户审批
  → 完成后写 "completed" 信封到 append-only 会话日志
```

## 7. 安全与验证

- **Secure by Default**：默认 `ask` 档位，危险命令与写文件必须审批；
  UI 只显示掩码 Key（`••••abcd`），不回传明文。
- 验证命令：
  - `cargo test -p kodo-core -p kodo-agent`
  - `cd apps/desktop && npm run typecheck`
  - `npx playwright test`（有意改 UI 后 `npx playwright test --update-snapshots`）
- 设计规范见 `docs/design/`（`CLAUDE.md`、`LAYOUT.md`、`UI_ACCEPTANCE.md` 等）。
