# 实现方案与参考评估

## 本地参考

只读检查同级本地源码，无需上传 Kodo 文件：

- `../codex/codex-rs/core/src/tools/handlers/multi_agents_v2/spawn.rs`、`agent/control/spawn.rs`、`agent/child_config.rs`：独立 child、父子身份、执行边界继承、深度控制。
- `../deepseek-harness/packages/subagent/tool-subagent/src/index.ts`：模型可见的委派工具，前台收集结果，非 completed 终态转错误，释放执行资源。
- `../deepseek-harness/packages/subagent/subagent/src/control-types.ts`：持久 child 身份与运行状态区分，父子归属检查。

Kodo 的 `run()`、Provider、SkillSpec、Permission、SandboxContext 与 TurnLogger 已覆盖大部分执行能力。直接复用，比引入第二个框架或进程协议更适合当前同步回合模型。前台执行在不重写审批和共享文件并发控制的前提下提供真实委派。

## 模块

1. `crates/agent/src/subagent.rs`：斜杠解析、受限 child 请求、生命周期、公开 trace 收集、执行结果。`run()` 增加内部 scope，限制后代工具与预算，不改变公开 RunRequest 调用方式。
2. `protocol.rs`：typed `subagent` 参数、schema、native 与 fallback 协议一致；现有内置 skills 明确允许委派。
3. `kodo-core::session`：可向后兼容的 `subagent` item，公开步骤与状态用 serde 编码；运行信息不混入父回答或验证计数。
4. `kodo-shell::TurnLogger`：独立子任务 ID 映射，实时更新既有 item，不占用父步骤的 open_id。详细模型 I/O 默认关闭，子任务不启用额外私密归档。
5. Desktop DTO、Composer、AgentTrace、Trace timeline/map：复用已有样式和层次，子步骤展开查看，状态用文字表达。
6. CLI：显示委派开始及终态，保留本地会话索引。

## 约束与失败

- 子历史清空，父上下文仅由任务描述、显式附加上下文和原有安全指令传递；不自动复制父工具日志。
- scope 贯穿 registry 初次构建、动态刷新与执行检查；只读通过工具隐藏和实际执行拒绝双重约束。
- 子请求无独立 session_id，避免覆盖父恢复与 undo 文件；父执行器对委派前后进行文件快照，合并真实变更。
- 限制为单层、最多 4 个 child；限制子回合轮次、工具次数与墙钟时间。Trace 有界、脱敏，忽略 Thinking 和私有模型 I/O。
- child 成功以最终 delivery/verification 为准；错误包含有限的部分结果，供父任务处理。

## 验证

使用离线 Rust 单测与本地 mock 提供方，覆盖协议、执行、权限、取消、限制、变更集和日志重载。运行相关 crate tests/check、全仓 fmt check、前端 typecheck/build、针对性 Playwright 交互测试。无新依赖。远程 CI 与真实模型测试单独标记未运行。

## Analyze

FR-001/002 对应协议和入口；FR-003/004/005 对应运行 scope；FR-006/008 对应 session/TurnLogger/UI；FR-007 对应现有 opaque 文件变更合并。无未解决的兼容性决策；Git 提交和本地合并须待验证结果可复核后获得单独授权。

## 完成摘要与执行意图修复

复用既有状态机和验收证据：Finish 的空正文路径生成实际变更路径、真实校验命令摘要；不改变失败/截断/提供方错误优先级，不从拒绝的模型承诺推导成功。路由在共享 classifier 识别“运行已有命令”的动作，避免弱测试词和读取材料名压过动作；明确作者任务继续原路由。沿用现有 skill、沙箱与 Scope 过滤，不引入依赖或新任务类型。

先添加失败回归，再修改共享根因。验证 kodo-agent 全部单测及调用方 check/test；真实联网只在独立临时项目中通过现有 provider 加载接口进行，清空历史、记忆、用户技能、MCP 和 fallback。旧失败证据保留；新验收断言必须包含正文与结构化状态的一致性。

Analyze：FR-009 对应完成路径和真实委派回归，FR-010 对应 classifier 与实际工具路由回归，AC-011 对应合成项目执行/取消/Trace。没有新增协议、依赖或权限扩张。
