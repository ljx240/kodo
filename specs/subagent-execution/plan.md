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

## 自动识别与委派

复用已有模型工具选择，不增加第二次路由模型调用、关键词调度器或新配置。`protocol.rs` 的 `subagent` 工具描述是自动策略的唯一文本来源，明确多模块实质性任务每次只拆分一个模块或一个聚焦问题，避免将整个请求重派给 child；产物应简短并适配既有 120 秒预算。保留简单任务直接完成、用户否决优先、最少上下文、顺序执行及父级验收责任。`lib.rs::system_prompt` 在工具选择协议之前注入同一策略，native schema 和 JSON fallback 也继续使用这一工具定义。`run_registry` 与 `Scope::registry` 现有过滤继续决定是否可见；子任务不可见该工具，也不会收到自动委派提示。

测试覆盖共享工具描述的传播/过滤，以及普通输入的实际主子执行、父级继续、简单回答不产生子 Trace。离线 mock 验证运行协议，不声称证明模型的语义判断；在单独获得联网授权后，以不含委派术语的合成任务运行真实模型，记录正例、简单反例、取消及持久化证据。

Analyze：FR-011 对应共用工具策略和普通任务委派；FR-012 对应简单任务/用户限制提示和父级验收；FR-013 对应 registry/Scope 的既有权限过滤，AC-012/013/014 分别覆盖提示、运行和真实模型判断。保留手动入口，不改权限、执行器或 UI 协议。不委派本任务的实现工作。

委派写入衔接：`plan.rs` 将运行时核实的 child `changed_files` 记录为父级编辑证据；`state.rs` 在委派返回后清除旧完成声明，父级验证完成后回到执行循环，等待剩余工作和新的汇总正文。在现有验收证据中持久化一个带默认值的委派标记，兼容旧检查点；公开 Trace 协议不变。只读子任务返回后也需要父级新的汇总，不能因父级此前验证通过而跳过。即使工具最终失败，已核实的文件变化仍使旧验证失效。FR-014 / AC-015 由状态机与真实文件工具的离线主子回归覆盖。


真实合成子任务暴露了已有问答判定把“不要修改”“禁止执行”当成正向任务的问题，继而因 README.md 等材料名选中 docs 编写门。最小修复复用同一个动作判断：逐个出现位置检查紧邻的中英文否定前缀与英文单词/方法/路径边界，供 `is_conversational_question`、项目概览意图、文档输出意图与 `parse_output_contract` 使用；同一消息仍有正向动作时继续任务流程，避免 Work 模式的文件契约再次误把否定要求当成写入。没有改变显式技能、文件输出优先级或既有审阅技能，不按只读 Scope 一概撤销技能。FR-015 / AC-016 以路由反例和实际只读 child 回归取证。


`attempt-11` 显示“不要分析其他模块内部实现”中的名词仍可命中编写技能，单靠措辞规则不足。复用已知 child Scope：仅对未显式选技能、未明确保存文档、自动技能确含 Edit 步骤的只读 child，使用只读分析计划并解除误推的文件产物门；只读审阅工作流和显式约束保持原路径，最终 registry 仍被父白名单和 Scope 过滤。FR-016 / AC-017 使用实际 child 正反例回归，不增加依赖或第二次模型路由。


Scope 的明确文件交付判定复用输出契约的目标与动作位置：文件目标需与实际创建/更新动作对应，避免 catch 写入 failed 记录等数据流描述命中保存流程；显式匿名文件交付仍保留。用 Code/Work 实际只读 child 与保存目标在动作前后的反例验证，不放宽工具边界。
