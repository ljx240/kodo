# Kodo Skills 深化与 MCP thin client 设计

## 目标

把 work agent 价值密度最高的两块能力落地到 Kodo：

1. **Skills 深化**：让技能成为可沉淀、可复用的流程知识 —— 技能带一行描述、匹配后把 guidance 注入 prompt、支持项目级技能目录（项目覆盖用户）。
2. **MCP thin client**：让 Kodo 能接入外部工具服务器（stdio 与 Streamable HTTP 两种传输），外部工具以最小侵入进入现有工具协议、权限体系和轨迹展示。

产品边界不变：不是通用管理面板，也不是完整 IDE；GUI 只服务于项目内对话、agent 执行、代码变更与验证。

## 现状与问题

- 技能体系已存在（`crates/agent/src/skill.rs`）：SKILL.md 纯 Markdown 分节格式（无 frontmatter、无 `## description`），8 个内置技能经 `include_str!` 内嵌，用户技能为 `state_dir()/skills/<name>.md` 平铺目录，chip `【技能：id】` 选取，选取完全确定性（classify + chip），模型从不选技能。
- prompt 从不注入技能全文，只有一行 "Active skill" 横幅 + plan 块；`guidance` 仅 UI 展示 —— 技能最有价值的"何时用、怎样算好"从未到达模型。
- 技能只有用户目录一个来源；团队无法在仓库里携带项目专属流程（如发布检查），也无法覆盖个人版本以保证可复现。
- 工具协议为闭合 `ToolName` 枚举 + typed `ToolArgs`（`crates/agent/src/protocol.rs`）；`SkillSpec::allows_label` 经 `ToolName::parse` 解析标签 —— 无法命名 MCP 工具，技能白名单会误杀外部工具，需要显式放行策略。
- `Permission::needs_approval` 对未知 `StepKind` 落 `_ => false` 兜底 —— 新步骤类型会旁路审批，必须补显式分支。
- 全同步栈：blocking `ureq` + `std::thread`/`mpsc`，无 tokio；`process.rs` 的进程组 kill、`provider.rs` 的 `Utf8LineDecoder`/`SseEventAssembler` 可复用；`crates/core` 保持零依赖。
- 设置先例：`providers` JSON 数组存 `settings.log`，密钥走 `credentials.log`（0600）+ `mask_secret` 掩码回写 —— MCP 配置沿用同一套。

## 设计决策

### Skills

1. **`## description` 字段**：`SkillSpec` 增加 `description: String`；解析向后兼容（缺失时回退首个 completion_criteria 截断 120 字符）；`save_user_skill` 要求显式非空（zh-CN 报错）；8 个内置 SKILL.md 补上该节。GUI 技能详情与列表随之展示一行描述。
2. **匹配后注入（渐进披露的"载入全文"半边）**：原横幅抽为 `fn skill_prompt_block(skill, task_type) -> String`，输出横幅 + `Skill description:` + `Skill guidance:` 全文；workflow 已在 plan 块中不重复。**不注入技能目录** —— 选取是确定性的，模型不选技能，目录只烧 token。
3. **多目录**：`run()` 从 `request.project` 派生 `<project>/.kodo/skills/`，与用户目录共用同一加载器（继续平铺 `<name>.md`，不支持嵌套布局）。不加 `RunRequest` 字段 —— 项目目录从既有 `project` 路径派生，密闭性由测试临时目录天然保证。
4. **同名优先级：项目技能覆盖用户技能**（git local 覆盖 global 惯例，团队仓库可复现优先）；内置名在所有目录保留不可占用。注册追加序 builtin → project → user（`select()` 首配仍 builtin 优先）。
5. **GUI**：SkillsPage 增第二分区「项目技能」（有项目时显示，`＋` 就地新建到项目存储）；`SkillView.source` 增 `"project"`；CRUD 命令加 `scope: "user"|"project"`（默认 user，旧调用不变）；内置只读。卡片头为 flex 行，动作右对齐。

### MCP

1. **配置**：`settings.log` 键 `mcp-servers`，JSON 数组，字段 `{id, name, enabled, transport:"stdio"|"http", command, args[], env{}, url, headers{}}`；env/headers 的值一律走 `credentials.log`（键 `mcp.{id}`），settings 副本置空字符串，掩码回写流程照抄 providers。`crates/core` 加 `write_secret`/`read_secret` 薄封装（0600）。
2. **客户端模块** `crates/agent/src/mcp/`（`mod.rs` 配置/McpClient/Drop 拆除、`jsonrpc.rs`、`stdio.rs`、`http.rs`）。全同步：
   - 生命周期：`run()` 内 registry 组装后 `McpClients::connect`，**单 server 失败不致命**（记 zh-CN note，继续）；`Drop` 保证拆除（stdio `kill_process_tree`，http 尽力 `DELETE`）。
   - 协议：`initialize`（`protocolVersion: "2026-07-28"`，接受服务端回显）→ `notifications/initialized` → `tools/list`（跟 `nextCursor` 分页）→ `tools/call`。`Mcp-Session-Id` 按可选处理（返回则回传，没有则省略），兼容 stateful/stateless 服务端。tools/list 结果按 run 缓存。
   - stdio：持久子进程（复用 `process.rs` 原语：`Command` + `setpgid` + `agent_env_policy` env 擦洗 + 读线程 + 请求 id 关联 + Drop 杀进程组）。
   - http：ureq POST，响应兼容 `application/json` 与 `text/event-stream`（复用 SSE 解析器）；短读超时 + `alive()` 抢占避免同步阻塞。
   - 错误映射：JSON-RPC error / `isError:true` → `ToolError::execution`；401/403 → `permission_denied`；超时/传输 → execution（带 server 标签）。
3. **工具注册（不动 `ToolName` 闭合枚举）**：`protocol.rs` 增 `ExternalToolDef`/`ExternalCall`/`ToolInvocation::External`，registry 增 `with_externals`；`accept()` 在 `ToolName::parse` 失败后按 wire name 查 externals。wire 名 **`mcp__<server_id>__<tool>`**（双下划线；`mcp:a:b` 会被部分 provider 的 `^[A-Za-z0-9_-]+$` 校验拒绝）。`tool_schemas()` 映射 externals（parameters=inputSchema），原生 tool calling 免费获得。
4. **skill 门控策略**：skill 白名单只约束**内置**工具；`mcp__*` 标签过 skill gate 恒放行，但**始终过 Permission**（内置工具的 read-only 保证不受影响；`filter_registry` 处写注释，DESIGN.md 记录该决策）。
5. **权限补洞**：增 `StepKind::ExternalTool`；`needs_approval`：Ask→要审批、Auto→要审批（外部风险≥网络）、Full→放行；detail = `mcp:<server>:<tool> <args>` 经 `redact_secrets`。审批闭包签名不变。会话/轨迹最小暴露：以 `Step::Command` 形态发事件（label 带 wire name），不加 `ItemKind`/`Step` 变体，Inspector 零改动。
6. **GUI**（2026-09-29 修订，2026-09-30 二次修订，2026-09-30 三次修订）：MCP 管理为侧边栏「技能」下的独立目的地页 `McpPage`：顶部「连接器目录」卡片区（`MCP_TEMPLATES` 20 个预设，紧凑瓦片——经批准的唯一瓦片网格例外，DESIGN §9；整卡点击预填添加编辑器，密钥留空；每个连接器只添加一次——已配置模板（按名称或启动标识：stdio 包/http URL 匹配）的卡片呈禁用「已添加」态，自定义「添加 MCP 服务器」不限次、可建刻意的第二实例），下方服务器列表（启用开关 + 行内编辑 + 每服务器工具清单经 `mcp_list_tools` 一次性连接读取，密钥只在 shell 进程内合并；三个 MCP 命令均 `#[tauri::command(async)]`，stdio spawn 不阻塞主线程）。编辑器无预设下拉（目录取代）。Settings「工具与权限」只保留权限模式。不做 test-connection（工具清单展开即唯一在线接触）。i18n 全走 `i18n.ts`。
7. **密闭性**：`RunRequest.mcp_servers: Vec<McpServerConfig>`（空=无 MCP），全部构造点补 `Vec::new()`（desktop/CLI 传 `ShellConfig` 值）。测试用 `FakeTransport`（`McpTransport` trait 是可测性接缝）。

## 非目标

- MCP resources / prompts / sampling / OAuth / JSON-RPC 批量。
- 技能目录注入 prompt；skill 白名单引用 MCP 工具。
- 嵌套 `<name>/SKILL.md` 布局。
- 新 session `ItemKind`/`Step` 变体；新导航层级；新 npm 依赖。
- Windows 支持；test-connection UI。
- 插件框架与 subagent（按前期评估缓做，证据驱动后再启）。

## 验收标准

- 技能保存缺 `## description` 被拒绝（zh-CN 报错）；解析回退规则有单测；内置 8 技能均带描述。
- 命中技能后 prompt 含 `Skill description` 与 `Skill guidance` 全文（`skill_prompt_block` 单测）。
- `<project>/.kodo/skills/` 技能可被 chip 选取；同名项目技能覆盖用户技能；内置名无法被占用；`select()` 仍落 builtin。
- GUI：项目技能卡有/无项目的渲染、scope 隔离的 CRUD、空态文案、class 拆分（`.skill-row` 仅内置）全部有 Playwright 用例；`/ui-demo` 截图 1586×992 过 `UI_ACCEPTANCE.md` §5g。
- MCP：serde 往返、密钥 0600、JSON-RPC 编解码与乱序关联、stdio/HTTP 罐头服务器集成测（loopback，不触外网）、session-id 回放与 DELETE、单 server 失败隔离、FakeTransport 流式场景（模型发 `mcp__fake__echo` → Ask 审批 → 派发 → 回填）、空配置零连接。
- 权限矩阵：ExternalTool 在 Ask/Auto 需审批、Full 放行；审批拒绝路径不派发。
- `cargo test --workspace`、`cargo clippy --workspace`、`npm run typecheck`、`npm run test:visual` 全绿。
