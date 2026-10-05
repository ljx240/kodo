# Kodo Skills 深化与 MCP thin client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地技能描述/渐进披露/项目技能目录（Track A）与 MCP thin client（Track B），不改变 Kodo 的产品边界、同步栈和工具协议闭合性。

**Architecture:** Skills 侧在既有 `skill.rs` 解析器上加 `description` 与多目录装配，prompt 注入收敛为一个 `skill_prompt_block` helper；MCP 侧新增 `crates/agent/src/mcp/` 同步客户端模块，外部工具经 `ToolInvocation::External` 进入既有 registry/权限/轨迹管道，`ToolName` 枚举保持闭合。

**Tech Stack:** Rust（零新依赖，复用 ureq/std::thread/process.rs/provider.rs 解析器）、React 19 + TypeScript、Playwright（1586×992）。

**Spec:** `docs/superpowers/specs/2026-09-28-skills-and-mcp.md`

## Global Constraints

- `crates/core` 保持零依赖；不引入 tokio/reqwest；不加 npm 依赖。
- 不新增 session `ItemKind`/`Step` 变体、不新增 Settings 分类、不新增导航层级。
- 密钥不得明文经过 `settings.log` / webview（照抄 providers 掩码回写）。
- GUI 改动遵循 `docs/design/*`（source of truth），每屏过 `UI_ACCEPTANCE.md`；Playwright 截图 1586×992，不用 macOS 截屏。
- 不修改 Git 历史；提交由用户后续决定。

## Track A — Skills（已完成）

### Task A1: `## description` 字段 + 解析 + 内置补齐

**Files:** `crates/agent/src/skill.rs`、`skills/*/SKILL.md`（8 个）

- [x] Step 1: `SkillSpec` 增 `description`；`parse_skill` 读 `## description`，缺失回退首个 completion_criteria 截断 120 字符；`save_user_skill` 拒绝缺失（zh-CN 报错）。
- [x] Step 2: 8 个内置 SKILL.md 补 `## description` 一行描述。
- [x] Step 3: 单测覆盖解析形状、回退规则、保存拒绝、内置形状断言。

### Task A2: 匹配后注入

**Files:** `crates/agent/src/lib.rs`、`docs/design/DESIGN.md`

- [x] Step 1: 抽 `fn skill_prompt_block(skill, task_type) -> String`（横幅 + description + guidance 全文，空则省略对应段），替换原内联 format。
- [x] Step 2: helper 单测（含/不含 guidance）；DESIGN.md Guidance scope 段更新为匹配后注入、无目录。

### Task A3: 项目技能目录 + 优先级

**Files:** `crates/agent/src/lib.rs`、`docs/design/DESIGN.md`

- [x] Step 1: `fn build_skill_registry(user_dir, project)`：builtin → `<project>/.kodo/skills` → user 顺序装配，同名项目胜用户；内置名跳过。
- [x] Step 2: 单测覆盖项目目录可达、同名覆盖、内置名不可劫持、`select()` 仍落 builtin、裸 registry 密闭。

### Task A4: GUI 项目技能分区 + scope 参数

**Files:** `apps/desktop/src-tauri/src/{main,view}.rs`、`apps/desktop/src/{api.ts,i18n.ts}`、`apps/desktop/src/pages/{SkillsPage,SkillDetailModal}.tsx`、`apps/desktop/src/conversation/Composer.tsx`、`apps/desktop/src/App.tsx`、`apps/desktop/src/styles/pages.css`、`apps/desktop/tests/visual/{skills.spec.ts,shell.ts}`、`docs/design/{COMPONENTS,UI_ACCEPTANCE,DEMO_DATA}.md`

- [x] Step 1: Rust 侧 CRUD 加 `scope: "user"|"project"`（默认 user），`SkillView.source` 增 `"project"`，`list_skills` 返回 builtin+project+user。
- [x] Step 2: SkillsPage 第三卡片（有项目才显示，`skill-new-project` 就地新建），行 hook `data-project-skill-id`/`data-user-skill-id`，Modal scope 隔离读写，模板带 `## description`，Composer 选择器合并项目技能。
- [x] Step 3: Playwright：夹具补 `description`；新增项目卡用例（行归属、空态、scope 隔离的新建）；typecheck + 全量 test:visual 绿。
- [x] Step 4: `/ui-demo/skills` 截图 1586×992 过 UI_ACCEPTANCE §5g；修 `skills-card-head` flex 行使 `＋` 右对齐；更新 COMPONENTS §13b/§13c、UI_ACCEPTANCE §5g、DEMO_DATA 技能段。

### Task A5: 交付文档

**Files:** `docs/superpowers/{specs,plans}/2026-09-28-skills-and-mcp.md`

- [x] Step 1: 中文 spec（目标/现状与问题/设计决策/非目标/验收标准）。
- [x] Step 2: 本 checkbox plan（Track A 勾选、Track B 待办）。

## Track B — MCP（待办）

### Task B1: 配置 + 持久化 + 密钥

**Files:** `crates/agent/src/mcp/mod.rs`（新建）、`crates/core/src/settings.rs`、`crates/shell/src/config.rs`、`apps/desktop/src-tauri/src/main.rs`

- [x] Step 1: `McpServerConfig`/`McpTransportKind` serde（往返单测）；`split_secrets`/`merge_secrets`（env/headers 值拆到 credentials）。
- [x] Step 2: core `write_secret`/`read_secret`（0600 单测）；`ShellConfig.mcp_servers` 解析。
- [x] Step 3: Tauri `load_mcp_servers`/`save_mcp_servers`，掩码回写照抄 providers 分支。

### Task B2: JSON-RPC 层 + `McpTransport` trait + FakeTransport

**Files:** `crates/agent/src/mcp/jsonrpc.rs`、`crates/agent/src/mcp/mod.rs`

- [x] Step 1: 信封编解码、请求 id 关联、`McpError` 映射；乱序响应关联单测。
- [x] Step 2: `McpTransport` trait（`request`/`notify`/`alive`）+ `FakeTransport` 测试替身。

### Task B3: stdio 传输

**Files:** `crates/agent/src/mcp/stdio.rs`

- [x] Step 1: 持久子进程（setpgid + `agent_env_policy` + 读线程 + id 关联 + Drop `kill_process_tree`）。
- [x] Step 2: `#[cfg(unix)]` 临时 `sh` 罐头应答器集成测：initialize → tools/list → tools/call。

### Task B4: Streamable HTTP 传输

**Files:** `crates/agent/src/mcp/http.rs`

- [x] Step 1: ureq POST；JSON 与 SSE 双响应（复用 `Utf8LineDecoder`/`SseEventAssembler`）；`Mcp-Session-Id` 可选捕获/回放；Drop 尽力 DELETE。
- [x] Step 2: 本机 `TcpListener` 罐头服务端（loopback）：session 头回放断言 + DELETE 断言。

### Task B5: McpClient 生命周期 + tools 缓存

**Files:** `crates/agent/src/mcp/mod.rs`

- [x] Step 1: 握手顺序（initialize → initialized → tools/list 分页）；tools 缓存按 run。
- [x] Step 2: `call_tool` 拍平 `content[]` 文本块；单 server 失败隔离返回 note 不致命。

### Task B6: registry externals

**Files:** `crates/agent/src/protocol.rs`、`crates/agent/src/lib.rs`、`crates/agent/src/skill.rs`

- [x] Step 1: `ExternalToolDef`/`ExternalCall`/`ToolInvocation::External`/`accept()` 外部分支/`protocol_instructions`；`tool_schemas` 映射（parameters=inputSchema）。
- [x] Step 2: skill gate 对 `mcp__*` 恒放行（写注释 + DESIGN.md 记录）；`ToolInvocation` 穷举匹配连锁点由编译器兜底修正（~6 处）。

### Task B7: 权限 + 轨迹形态

**Files:** `crates/agent/src/lib.rs`、`apps/desktop/src-tauri/src/view.rs`（如需）

- [x] Step 1: `StepKind::ExternalTool`；`needs_approval` 显式分支（Ask/Auto 审批、Full 放行）；detail 带 `redact_secrets`。
- [x] Step 2: 权限矩阵单测 + 审批拒绝路径单测；外部调用以 `Step::Command` 发事件（label 带 wire name）。

### Task B8: `run()` 集成 + 密闭性

**Files:** `crates/agent/src/lib.rs` 及全部 `RunRequest` 构造点（desktop/CLI/evals/benchmark）

- [x] Step 1: `RunRequest.mcp_servers: Vec<McpServerConfig>`，全部构造点补齐（desktop/CLI 传 `ShellConfig` 值，其余 `Vec::new()`）。
- [x] Step 2: registry 组装后 connect、外部调用派发走 `McpClients`、失败 notes；`agent_benchmark`/`evals` 保持编译。
- [x] Step 3: FakeTransport 流式场景测试（`mcp__fake__echo` → Ask 审批 → 派发 → 回填）+ 空配置零连接测试。

### Task B9: Settings MCP UI

**Files:** `apps/desktop/src/pages/SettingsPage.tsx`（ToolsBody）、`apps/desktop/src/api.ts`、`apps/desktop/src/i18n.ts`、`apps/desktop/tests/visual/*`

- [x] Step 1: `McpServerListSection`/`McpServerEditor`（克隆 Provider 形态；transport 条件字段 + env/headers 键值行 + 掩码回写）。
- [x] Step 2: typecheck + Playwright settings 用例 + `/ui-demo` 截图 1586×992 过 LAYOUT §8「每个分类非空」、COMPONENTS §14「一行一设置/无死控件」。

### Task B10: 文档收尾

**Files:** `docs/design/{LAYOUT,COMPONENTS,DESIGN}.md`、`docs/CODE_TOUR.md`、本 plan/spec

- [x] Step 1: LAYOUT §8、COMPONENTS MCP 小节、DESIGN MCP 小节 + skill 门控策略、CODE_TOUR 加 `mcp/`（如枚举模块）。
- [x] Step 2: 本 plan 全部勾选；最终验证 `cargo test --workspace`、`cargo clippy --workspace`、desktop typecheck + test:visual。

## Track C — MCP 独立页 + 预设目录（2026-09-29 追加，已完成）

用户反馈 Settings 内找不到 MCP 入口，确认两项后追加本轨道：MCP 挪到侧边栏「技能」下的独立目的地页；内置常用 MCP 预设目录（一键添加，预填 command/args/URL，密钥留空）。

### Task C1: `McpPage` 目的地页

**Files:** `apps/desktop/src/{routes.ts,App.tsx}`、`shell/{Sidebar,TopBar,controls}.tsx`、`inspector/Inspector.tsx`、`pages/{McpPage,SettingsPage}.tsx`、`styles/app.css`、`i18n.ts`

- [x] Step 1: `RouteName` 增 `"mcp"`；侧边栏 `技能` 下新增 `MCP` 行（Plug 图标）；TopBar 收起态目的地条同步；Inspector `PANEL_TABS.mcp`。
- [x] Step 2: `McpPage`（page-head + 服务器行：chevron 展开工具清单 / transport 徽标 / 启用开关 / 行内编辑）；`McpServerListSection`/`McpServerEditor`/`KeyValueRows` 迁出自 Settings；`Field`/`Switch` 抽到 `shell/controls.tsx` 共用。
- [x] Step 3: Settings → 工具与权限回退为仅权限模式（MCP 块删除）。

### Task C2: 工具清单 + 预设目录

**Files:** `apps/desktop/src-tauri/src/main.rs`、`crates/agent`（无改动，复用 `McpClient`）、`apps/desktop/src/{api.ts,data/{mcp,demo}.ts}`、`pages/McpPage.tsx`

- [x] Step 1: Tauri `mcp_list_tools(id)`：shell 进程内合并密钥 → `McpClient::connect` → 返回 name/description（密钥与 schema 不进 webview）；前端 `listMcpTools` 区分空/错。
- [x] Step 2: `MCP_TEMPLATES`（filesystem/github/gitlab/postgres/sqlite/memory/puppeteer/brave-search/slack/notion）；新建编辑器首个字段为预设选择器，预填字段 + 已知密钥空行；编辑已有服务器不显示预设。
- [x] Step 3: `DEMO_MCP_TOOLS` 夹具；Playwright：7 个既有 MCP 用例改指 `/mcp`，新增侧边栏/预设/工具清单/失败态/设置回归 5 用例；`mcp`/`mcp-tools`/`mcp-editor` 截图，退役 `settings-tools*` 基线。
- [x] Step 4: 文档同步（LAYOUT §2/§8、COMPONENTS §14、DESIGN §12、UI_ACCEPTANCE §5a/§5h、DEMO_DATA、CODE_TOUR、spec 决策 6 修订）；最终验证 cargo test 513 绿、clippy 0、typecheck 绿、test:visual 除他人在写的 `tmp-acceptance.spec.ts` 外全绿。

## Track D — 连接器目录卡片（2026-09-30 追加，已完成）

用户要求「优化 MCP 页面的 UI 并增加内置的 MCP 连接器，以卡片的形式展示」：MCP 页新增「连接器目录」卡片区（20 个预设，紧凑瓦片为经批准的 DESIGN §9 例外），整卡点击预填添加编辑器（密钥留空；当时口径为「永远新增实例」，2026-09-30 由 Track E 改为每个连接器只添加一次）；编辑器预设下拉删除；模板扩到 20（+sequential-thinking/context7/playwright/google-drive/redis/everything/figma/linear/fetch/time）。纯 GUI + 文档 + Playwright。

- [x] Step 1: `data/mcp.ts`（`McpTemplate.icon` + 20 模板）；`i18n.ts`（`catalogTitle/catalogHint/catalogAdd/catalogApplied/serversTitle/catalogDesc`，删 template* 三键）。
- [x] Step 2: `McpPage.tsx`（`McpCatalog` + `MCP_ICONS` + `templateFormSeed`，`edit` 提升到页级，节头承载添加按钮）；`app.css`（`.mcp-catalog/.mcp-card*/.mcp-section-head`，6px 圆角 1px 描边无阴影）。
- [x] Step 3: Playwright（改写预设用例为卡片点击，新增目录 20 卡/空白自定义/重复实例 3 用例）；`mcp`/`mcp-tools`/`mcp-editor` 三基线再生成并目检；全量 test:visual 184 绿。
- [x] Step 4: 文档同步（COMPONENTS §14、DESIGN §9/§12、UI_ACCEPTANCE §5h/§6、DEMO_DATA 补 `/ui-demo/mcp`、CODE_TOUR、spec 决策 6 二次修订）。

## Track E — 目录去重 + 添加流程卡顿（2026-09-30 追加，已完成）

用户反馈三点：(1) 同一 MCP 可重复添加不合理；(2) 内置 MCP 是否真实生效；(3) 添加过程卡顿严重。(2) 经链路核实确认生效（`send_message` 每次消息重载 `ShellConfig` → `McpClients::connect` 只连 enabled → `mcp__<server>__<tool>` wire 名注册 → Permission 把关；预设仅预填，生效需密钥与可 spawn 命令；保存后下一条消息即生效），无代码改动。

- [x] Step 1（去重）：`templateConfigured`（名称或启动标识：stdio 包/http URL）→ 已配置模板卡片禁用 + `已添加` 态（Check + `.mcp-card--added`）；`servers`/`edit` 提升到 `McpPage` 作单一数据源；自定义「添加 MCP 服务器」不限次（刻意第二实例入口）。i18n 增 `catalogAdded/catalogAddedLabel`，`catalogHint` 改「每个连接器只添加一次」；`app.css` 增 `.mcp-card--added/.mcp-card-state`。
- [x] Step 2（卡顿根因）：三个 MCP Tauri 命令（`load_mcp_servers`/`save_mcp_servers`/`mcp_list_tools`）原为同步 `#[tauri::command]`，跑在主线程；`mcp_list_tools` 的 stdio spawn（`npx -y` 首跑下载数秒）直接冻结 UI。改为 `#[tauri::command(async)]`（同 `list_provider_models` 先例）；`McpTools` 加 in-flight Promise 去重，杜绝 StrictMode 双挂载双 spawn。
- [x] Step 3: Playwright（重复实例用例反转为「已添加态拒二次添加、自定义不限、删除释放卡片」）；mcp 三基线再生成（filesystem 夹具命中模板 → 卡片呈 已添加）。
- [x] Step 4: 文档同步（COMPONENTS §14、DESIGN §12、UI_ACCEPTANCE §5h、spec 决策 6 三次修订、DEMO_DATA、CODE_TOUR）。剩余固有因素：`tauri dev` debug 构建本身偏慢，发布构建无此问题。
