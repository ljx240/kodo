# Kodo 会话过程展示评审与完善

日期：2026-10-12。范围：用户提供的 Kodo、Codex 执行过程截图，以及本地 Agent、会话存储、桌面事件和展示代码。

## 对比结论

截图中的 Codex 用公开进展说明解释“已确认什么、接下来做什么”，再穿插简短工具活动；Kodo 把调度阶段、工具调用和模型计数作为主内容。用户因此能看到执行活动，却难以判断任务进展。需要补齐的是公开进展摘要的生成、保存和展示。

公开摘要与模型私有推理分开处理：只展示面向用户的说明，不把 `reasoning_content`、`<think>`、调度诊断或工具协议转成会话正文。截图内的历史确认消息只用于分析展示效果。

## 发现与处理

| 问题 | 影响 | 本次处理 |
| --- | --- | --- |
| 系统提示只规定工具协议和最终交付，未要求公开进展 | 调用工具时往往只有工具参数，缺少阶段性发现和下一步 | 要求简短、基于已读取结果的公开摘要；原生工具使用 assistant text，JSON 回退增加可选 `commentary` |
| `TextDelta` 仅进入前端引用缓存 | 刷新会话丢失中途文字，历史记录无法重放 | 用现有 `Step::Reasoning` 的 `commentary` phase 保存公开说明，不改变日志字段或事件协议 |
| 任意非模型步骤都会刷新文字缓存 | 预算记录、恢复提示可能把草稿误当成摘要，随后又与正式摘要重复 | 旧流仅在真实工具边界刷新；持久化摘要清空对应草稿；下一次模型调用清除旧草稿 |
| 全部阶段默认展开，主状态直接拼接内部 detail | 英文占位和技术计数压过对用户有用的信息 | 主状态显示中文阶段与耗时；连续执行记录折叠，完整详情按需展开 |
| 最终回答到达后删除整个过程区 | 已公开的判断与阶段说明一起消失 | 进展说明保留原顺序；工具记录在最终回答前仍可展开 |
| 长回答默认只展示约 480 字 | 评审发现、结尾约束和风险容易隐藏 | 完整展示最终答复；继续复用 Markdown 的代码块折叠与目录 |
| 多个执行分组共用展开面板 ID | 键盘及辅助技术可能定位错误面板 | 每个阶段通过 React `useId` 分配独立 ID |
| 完成后未知 token 用量仍是“统计中”；零模型调用显示同样提示 | 状态暗示仍有统计工作，实际已结束 | 活跃轮次才标记统计中；缺失用量使用 `—`；空调用显示“尚未调用模型” |
| 进展文字被计入 Inspector 执行步骤 | 内容段落和执行活动混计 | 执行步骤统计排除公开说明；未使用工具明确显示空状态 |

## 最终展示与数据链路

会话顺序为：用户任务 → 开始说明 → 折叠执行记录 → 阶段性发现与下一步 → 折叠执行记录 → 完整最终答复、实际验证结果与文件变化。

- `commentary` 通过现有 Started / Finished、TurnLogger、ItemDto 链路进入会话日志；刷新后与其他已保存步骤一起恢复。
- 仅显式 `commentary` phase 渲染为公开段落，使用现有 Markdown 和文件路径打开能力。其他 reasoning 仍属于执行详情，诊断不会因此进入正文。
- 模型工具响应通过现有截断、预算和状态检查后才提交摘要；截断的工具响应不会生成该摘要或派发其工具。
- 工具记录默认关闭；失败或拒绝自动展开，原因与命令输出仍可检查。原生 `details` 支持键盘展开，子面板 ID 唯一。
- 审批、停止、中断、错误、模型切换、继续任务、重新生成、验证和安全撤销保持现有入口与判定逻辑。
- 最终回答的复制仍复制最终正文。进展说明和执行记录不混入复制结果，也不重复加入下一轮公开问答历史。
- 复用现有组件、字段和依赖；没有新增模型请求、配置项、遥测、外部服务或依赖。

## 变更文件

| 文件 | 作用 |
| --- | --- |
| `crates/agent/src/lib.rs` | 开始与验证说明、模型公开摘要提交、清理及端到端测试 |
| `crates/agent/src/protocol.rs` | 区分原生工具与 JSON 回退，读取可选公开摘要 |
| `crates/shell/src/turn.rs` | 验证摘要持久化和公开问答历史的边界 |
| `apps/desktop/src/conversation/ConversationProcess.tsx` | 按顺序显示摘要和折叠工具记录 |
| `apps/desktop/src/conversation/AgentTrace.tsx` | 展开面板的独立 ID |
| `apps/desktop/src/conversation/AssistantReply.tsx` | 保留过程、简化状态、完整最终答复 |
| `apps/desktop/src/conversation/trace.ts` | 摘要映射、私有标签清理、正确统计状态 |
| `apps/desktop/src/pages/ConversationPage.tsx` | 流缓冲与已提交摘要去重、重试草稿清除 |
| `apps/desktop/src/styles/conversation.css` | 复用既有字体、颜色与间距，设置安静的过程布局 |
| `apps/desktop/src/i18n.ts` | 过程、空调用和工具空状态文案；删除已停用的回答展开文案 |
| `apps/desktop/src/inspector/ResponseInspector.tsx` | 步骤与空状态统计 |
| `apps/desktop/src/inspector/TraceInspector.tsx` | 保持元数据计数一致，复用空调用文案 |
| `apps/desktop/tests/visual/qa-status.spec.ts` | 新过程契约、刷新、去重、恢复草稿、完整答复、状态及键盘验证 |
| `apps/desktop/tests/visual/behaviour.spec.ts` | 更新折叠交互与事件顺序回归 |
| `apps/desktop/tests/visual/file-panel.spec.ts` | 验证展开后的文件入口 |
| `apps/desktop/tests/visual/pages.spec.ts-snapshots/conversation-main-visual-darwin.png` | 更新会话截图基线 |
| `apps/desktop/tests/visual/pages.spec.ts-snapshots/conversation-inspector-closed-visual-darwin.png` | 更新关闭 Inspector 的会话基线 |
| `docs/reviews/conversation-progress-2026-10-12.md` | 评审、设计、验证及边界记录 |

## 本地验证

前端使用本机已安装的 Node 24 运行环境；未安装依赖。Rust 使用 `--offline`。测试数据为本地临时项目、fake provider、模拟桌面事件或测试用本机服务。

```text
cargo fmt --all -- --check
exit 0

cargo check --offline -p kodo-agent -p kodo-shell
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.40s

cargo check --offline -p kodo-desktop
Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.36s

cargo test --offline -p kodo-agent --lib
test result: ok. 498 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 11.46s

cargo test --offline -p kodo-shell --lib turn::tests
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.02s

cargo test --offline -p kodo-desktop --bin kodo-desktop
test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

npm run typecheck
> node ./node_modules/typescript/lib/tsc.js --noEmit
exit 0

npm run build
✓ 4096 modules transformed.
✓ built in 3.18s

./node_modules/.bin/playwright test tests/visual/qa-status.spec.ts tests/visual/behaviour.spec.ts tests/visual/file-panel.spec.ts tests/visual/answer-styling.spec.ts --project=behaviour --workers=4
121 passed (51.4s)

./node_modules/.bin/playwright test tests/visual/pages.spec.ts --project=visual --grep 'conversation-' --workers=2
2 passed (2.0s)

git diff --check
exit 0
```

截图基线更新后重新运行比对；已检查公开摘要示例在 1440px、900px 下的渲染。回归覆盖最终答复、刷新重放、预算事件去重、恢复草稿、事件乱序、旧会话、停止、中断、错误、审批、模型切换、文件查看、继续任务和权限同步。

前端构建成功，仍提示部分 chunk 超过 500 kB。本次未调整已有依赖或拆包策略。

## 兼容与验证边界

- 旧会话可继续打开，但未保存的旧中途文字无法补回。不会根据历史工具输出伪造旧进展。
- 公开说明最长保留 1200 个字符；模型应生成一至两句摘要。模型省略摘要时，已有开始／验证说明和真实运行状态仍可展示。
- 摘要在有效模型响应完成后提交。模型长时间等待期间显示阶段与耗时，不凭计时器制造发现或成功结果。
- 原生 assistant text 已通过清理单元测试；JSON 回退通过 fake provider 运行测试；存储与桌面展示通过各自回归。没有调用真实 MiniMax 或其他远程模型，未进行已安装应用的发布验证，也未核验远程 CI。
- 现有私有推理和模型 IO 的调试入口未扩展为主会话正文。审批与执行授权保持原判定。
- 此次为同一代理进行的本地源码评审；没有发送项目代码到外部审查服务，也没有独立代理审查。

## 合入前 Git 现场

- 工作分支：`feat/conversation-progress`；唯一 worktree：`/Users/lijianxue/work/WorkSpace/kodo`。
- 开发基线：本地 `main` / `HEAD` 为 `ae6f870f511e0fc148b1fddc44f048b5a10e38b9`。
- `origin/main` 缓存为 `211d0413d3b227b9bb36971db9a97a5e8fde3a34`；本次未 fetch，远端新鲜度未核验。
- 新任务创建时没有其他未合入 `main` 的任务分支。当前改动尚未提交；本地提交、`--no-ff` 合并和删除任务分支按用户授权执行。
- 原有未跟踪文件 `apps/desktop/scripts/capture-welcome.mjs` 和 `docs/总体架构设计.md` 保留，SHA-256 与任务开始时一致。
- `stash@{0}` 为 `9abd8060247ecfb8e2e0155fa8d7f06e6177fa15`；`stash@{1}` 为 `81465bee9001d4b6a119622be9060ab7d788ffce`。均保留，未新增、pop 或 drop。
- 未推送、创建 PR、修改远程、访问真实凭据或读取用户会话运行数据。构建与测试产物不进入提交。
