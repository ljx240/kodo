# Kodo 介绍（中文概览）

> 本文件由「为我介绍一下 kodo」任务生成，内容均取自仓库事实：`README.md`、`CLAUDE.md`、`docs/design/`、`crates/`、`apps/desktop/`。

## 一句话定位

极简 Work / Coding Agent 桌面工作台，全局双模式（**工作 / 代码**），主线：**项目 → 会话 → Agent 执行轨迹 → 最终回答 → 变更文件**。
不是通用后台，也不是 IDE；GUI 只服务于项目级对话、执行过程与代码变更的理解和审查。

## 技术架构

| 层 | 路径 | 职责 |
|---|---|---|
| Core | `crates/core` | 项目/会话/设置的 append-only 日志 |
| Agent | `crates/agent` | 工具循环、可选 LLM HTTP、写文件与验证 |
| Shell | `apps/desktop/src-tauri` | Tauri 命令、run 线程、审批机制 |
| GUI | `apps/desktop/src` | React + Vite |

目标平台：macOS / Linux（Windows 非一等目标）。本地状态目录（macOS）：`~/Library/Application Support/Kodo`。

## 核心功能

- 全局 **Work / Code 双模式**（设置键 `agent-mode`，与 CLI 共享）：Work 承接写文档/总结/问答等通用工作（无项目=纯对话，有项目=文件工具、不做验证）；Code 为完整编程代理（必选项目，含符号工具与验证）
- 左侧**单一项目树**（项目下直接嵌套会话，严禁第二会话栏；无项目会话归入「未选择项目」伪分组）
- 对话内嵌 Agent 轨迹：思考 / 搜索 / 读文件 / 命令 / 模型 / 写文件
- 可折叠 Inspector：摘要、变更文件、工具、LLM
- Response Trace / Archive / Settings 页面
- 多 AI Provider：Anthropic / OpenAI / DeepSeek / 自定义兼容端点；Key 仅显示掩码，存放于 `credentials.log`（0600）
- 确定性演示路由 `/ui-demo/*` 供视觉回归，参考视口 1586×992

## Secure by Default

| 档位 | 行为 |
|---|---|
| `ask`（默认） | 每个 shell 命令与写文件都需审批 |
| `auto` | 普通命令自动执行；危险命令与写文件仍需审批 |
| `full` | 项目内全自动（不推荐） |

危险命令按风险分类（Catastrophic / DestructiveGit / SensitiveData / ProcessControl / PackageInstall / Network / FilesystemWrite），
通过审批票据（wait point）机制请求人工批准；运行已取消时绝不等待审批。

## 开发与验证

```bash
./start.sh          # Tauri 桌面应用
./start.sh --ui     # 仅浏览器 UI（跳过 Rust 编译）

cargo test -p kodo-core -p kodo-agent
cd apps/desktop && npm run typecheck && npx playwright test
```

UI 改动工作流：先读 `docs/design/`（DESIGN / LAYOUT / COMPONENTS / UI_ACCEPTANCE / DEMO_DATA）与参考截图（视觉事实来源），
实现最小正确改动，在参考视口截图比对，直至 `UI_ACCEPTANCE.md` 通过——编译通过不等于页面完成。
