# 上下文管理评审与窗口设置

评审日期：2026-10-12。基线：`211d0413d3b227b9bb36971db9a97a5e8fde3a34`。

## 评审结论

现有实现能够按预算检索项目文件、保留用户指定的文件片段、标记修改后的过期内容、裁剪工具输出，并在有 token 使用量时尝试压缩当前运行历史。它适合有界的单轮任务；跨轮长会话记忆和调用前容量保护仍有缺口。

| 优先级 | 发现与触发条件 | 影响 | 本次处理 |
| --- | --- | --- | --- |
| P1 | 压缩依赖上一次成功调用的 input tokens；首轮为 0，服务商不报告 usage 时也为 0。`should_compact` 此时直接返回 false。 | 首次大提示词或无 usage 的服务商可能直接超窗；新增工具结果和输出预留也没有调用前检查。窗口设置不会自动限制 API 请求的实际 token 数。 | 保持原有策略，明确设置只影响预算显示和压缩阈值。建议后续增加调用前预算检查、输出预留和超窗恢复。 |
| P2 | 主模型失败并由备用模型响应时，预算窗口和模型标签仍取自原模型。 | 界面显示错误的使用比例，下一次压缩按错误分母触发。 | 已改为读取实际成功响应的 Provider，并增加离线假 Provider 回退测试。 |
| P2 | 窗口只能使用 Provider 类型的固定预设。`ModelSpec.context_window_override` 是未接入运行时的字段。 | 自定义接入或不同容量的模型无法校准上下文预算。 | 增加全局 `context-window-tokens` 设置，复用现有 capabilities override；空值恢复预设。没有扩展 Provider 配置结构。 |
| P2 | 跨轮只回放最近 8 个结束或中止的轮次，Agent 再限定为最近 16 条公开消息、最多约 32,000 字符。 | 更早的要求、决定和事实会被省略；提高窗口设置不会扩大这段历史回放。 | 保持现有范围。建议后续增加可持久化的会话摘要与稳定的约束记录。 |
| P2 | 轮内工具结果追加后先按 48 条消息裁剪，token 压缩在下一次调用前执行。 | 消息数量超过上限时，较旧内容可能先被直接丢弃，无法再参与摘要。 | 本次不调整历史裁剪策略。工具调用和结果的完整分组边界继续保留。 |

代码依据：`crates/agent/src/compaction.rs::should_compact`、`crates/agent/src/lib.rs::{initial_history,push_tool_results,trim_history,run}`、`crates/shell/src/turn.rs::conversation_history`。

已有的文件版本标记、原始轨迹保存和恢复检查点能够支持重新检查文件与恢复任务计划，但它们不等同于完整的持久会话摘要。压缩释放百分比目前按消息数量估算，也不等同于实际释放的 token 百分比。

## 窗口设置行为

- 入口：设置 → 模型 → 上下文窗口 tokens。
- 单位是 token，输入正整数；留空恢复各服务商预设。
- 编辑后离开输入框或按 Enter 保存，下一轮加载生效；当前正在运行的轮次不动态改预算。
- 自定义值是全局预算，适用于主模型、备用模型以及 CLI。使用多种模型时，应按实际支持的容量填写；设置不能扩大服务商的真实上下文容量。
- 前端拒绝 0、负数、小数、科学计数法、非数字和超过 u32 范围的数值；桌面保存边界再次校验。读取旧配置中的无效值时恢复预设。
- 保存失败时保留输入并显示错误，存储中的旧值不被前端替换。未编辑的输入框不会在失焦时写入配置。
- 继续使用既有 80% 压缩阈值；仍需要有效的 usage 和足够长的历史才能执行压缩。

## AGENTS.md 规则优化

原规则要求每次开发前 fetch/pull，却又要求任何联网先确认。这让本地评审和可逆修改依赖网络；同时 `origin/main` 本身只是缓存，相同哈希并不能证明远端没有变化。

本次将开发检查与远程交付检查分开：默认检查本地 `main`、记录基线哈希并建立任务分支；准备远程协作或交付时，在已有授权内同步远程引用。未经联网核验必须明确报告。更新历史、推送、PR、合并与清理仍遵守各自的授权要求。

同时修正了工作区判断：`git status --short --branch` 即使干净也输出分支标题；现在用 porcelain 的文件记录判断改动。用户已确认原样保留的现场不重复请求确认，已检查的旧基线提交也不再被误认作并发修改。

## 改动范围

| 文件 | 变化 |
| --- | --- |
| `AGENTS.md` | 本地基线、远程同步时机及现场确认规则 |
| `apps/desktop/src/pages/SettingsPage.tsx` | 设置输入、校验、保存及错误反馈 |
| `apps/desktop/src/i18n.ts` | 中文标签、提示和设置搜索关键词 |
| `apps/desktop/src-tauri/src/main.rs` | 保存边界校验 |
| `crates/shell/src/config.rs` | 桌面/CLI 共享配置加载、主模型和备用模型覆盖、配置测试 |
| `crates/agent/src/lib.rs` | 实际响应模型的窗口和标签、回退测试 |
| `apps/desktop/tests/visual/behaviour.spec.ts` | 设置输入、恢复自动值和失败保存测试 |
| 本文档 | 评审结果、行为范围及验证记录 |

没有新增依赖，没有读取真实凭据或运行数据，没有调用真实模型服务。

## 本地验证

前端使用已安装的 Codex Node.js 24.19.0；系统 Node.js 18.14.2 不满足当前 Playwright 要求。未安装或升级依赖。

```text
npm run typecheck
> node ./node_modules/typescript/lib/tsc.js --noEmit
exit code: 0

npm run build
✓ 4095 modules transformed.
✓ built in 955ms
exit code: 0
```

构建仍提示部分已有 bundle 超过 500 kB；本次没有做打包优化。

```text
npm run test:visual -- --project=behaviour behaviour.spec.ts \
  --grep 'context window|every visible functional setting|settings toggles|Models category'
5 passed (2.8s)

cargo check --offline -p kodo-shell
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.73s

cargo check --offline -p kodo-desktop
Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.52s

cargo check --offline -p kodo-agent
Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.96s

cargo fmt --all -- --check
exit code: 0

cargo test --offline -p kodo-shell config::tests --lib
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out

cargo test --offline -p kodo-agent context_budget_tracks_the_provider --lib
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 492 filtered out

cargo test --offline -p kodo-agent compact --lib
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 482 filtered out

cargo test --offline -p kodo-desktop --bin kodo-desktop
test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

cargo test --offline -p kodo-agent context::tests --lib -- --test-threads=1
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 473 filtered out

git diff --check
exit code: 0
```

上下文检索测试的默认并行运行出现一次 `pinned_span_survives_collect` 失败：`invalid project root: No such file or directory`；串行 20 项通过。测试夹具用 PID 和时间的纳秒部分命名，并在创建前删除同名目录；疑似临时目录竞态，本次未修改该夹具，未把串行通过表述为并行问题已经解决。

新回退测试的 `SinkEvent::Failover` 更正已由用户确认属于本任务，格式检查已通过。审查为本地代码复核；未进行独立外部审查。完整视觉快照、远程 CI 和真实服务商的上下文极限未验证。

## 本地集成与现场保留

用户已明确授权：保留本任务 8 个文件的差异，整理格式并验证，分别提交规则和功能，以 `--no-ff` 合入本地 `main`，再删除 `feat/context-window-settings`。远程交付没有得到授权，不属于本次操作范围。

规则已提交为 `89215156a88bd7adf7110c8aab8d2091b9e8688a`（`docs(agents): allow checked local development baselines`）。功能提交与合并后的哈希、现场及验证结果以最终交付报告为准。

以下为创建任务分支时的快照，并非合并后的状态：

任务分支：`feat/context-window-settings`。
worktree：`/Users/lijianxue/work/WorkSpace/kodo`。

```text
git rev-parse HEAD
211d0413d3b227b9bb36971db9a97a5e8fde3a34

git rev-parse origin/main
211d0413d3b227b9bb36971db9a97a5e8fde3a34

git branch --no-merged main
（无输出：任务尚无独有提交，功能仍在工作区）

git worktree list --porcelain
worktree /Users/lijianxue/work/WorkSpace/kodo
HEAD 211d0413d3b227b9bb36971db9a97a5e8fde3a34
branch refs/heads/feat/context-window-settings
```

本任务只使用此分支和唯一已有 worktree，没有新建或清理 worktree。没有推送、PR、远程修改或 fetch；远程新鲜度未核验。

用户已确认以下两个文件和两条 stash 与本任务无关、原样保留：

```text
?? apps/desktop/scripts/capture-welcome.mjs
?? docs/总体架构设计.md

stash@{0}: 9abd8060247ecfb8e2e0155fa8d7f06e6177fa15
stash@{1}: 81465bee9001d4b6a119622be9060ab7d788ffce
```

stash 未新增、pop 或 drop。没有未解决冲突。两个原有文件不暂存、不提交、不覆盖；合并后仍原样保留。

本地集成完成的条件：所有任务改动存在于本地 `main`，合并后的模块检查通过，任务分支安全删除，保留现场再次复核。本报告保留提交前的基线和验证记录，不把缓存 `origin/main` 表述为已核验的远程状态。

恢复：继续使用当前目录，先执行 `git status --short --branch` 与 `git log --oneline -n 5 main` 检查最终状态；任务分支删除后可通过功能提交或合并提交定位本次工作，不使用 reset、stash pop 或 clean。
