# 验收记录

状态：实现验收通过；用户已明确授权本地提交、`--no-ff` 合并及合并后清理本任务分支。实际集成哈希与 main 交付门结果记入最终交付报告。

基线：`91fd94ecc5013e826d433bb9f8365251843bf791`。
任务分支：`feat/subagent-execution`。
远程新鲜度：未核验。

## 实现与证据

| 验收项 | 实际验证 |
| --- | --- |
| AC-001/002 | `parent_continues_after_child_result_and_slash_runs_the_same_child`：真实 run 执行模型委派、子结果回填后父回合继续、斜杠调用相同执行器 |
| AC-003 | `read_only_child_cannot_write_even_with_allowing_approver`、`forged_child_calls_still_fail_at_execution_boundary`：允许审批也不能绕过只读，伪造调用在执行边界拒绝 |
| AC-004 | `scope_can_only_narrow_tools_and_cannot_expose_nested_delegation`、`limits_and_provider_failures_are_explicit_and_preserve_partial_status`：嵌套、第五次委派、工具扩权被拒绝 |
| AC-005 | 取消和 sink 失败阻止 provider 分派；partial/blocked/failed/timeout 的状态映射单测；提供方错误不算成功 |
| AC-006 | `delegated_writes_enter_parent_diff_and_safe_undo`：子任务真实创建文件，父变更集含路径，undo 只恢复子修改且保留原文件 |
| AC-007 | `subagent_snapshots_reload_without_consuming_parent_step_identity`：子快照更新同一 item、不占用父步骤 ID、失败和结果重载一致；前端验证 token 汇总与父回答隔离 |
| AC-008 | Playwright 验证 `/subagent` 键盘选择、单行更新、步骤展开、保存重载、800px 视口、Trace 页实时事件与陈旧 runId 拒绝 |

模型测试使用内置 fake provider 的本地 JSON 脚本；未发送任何项目数据到模型服务。

## 验证命令和原始关键输出

```text
cargo test --offline -p kodo-core -p kodo-agent -p kodo-shell -p kodo-desktop -p kodo-cli --lib --bins
test result: ok. 743 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 138 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 71 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 106 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

上列依次为 Agent、CLI、Core、Desktop、Shell，共 1111 个测试通过。

```text
cargo check --workspace --offline
Finished `dev` profile [unoptimized + debuginfo] target(s) in 2m 38s

cargo fmt --all -- --check
# exit 0，无输出

npm run typecheck --prefix apps/desktop
> node ./node_modules/typescript/lib/tsc.js --noEmit
# exit 0

npm run build --prefix apps/desktop
✓ built in 1.13s

npm run test:visual --prefix apps/desktop -- subagent.spec.ts trace-map.spec.ts composer-context.spec.ts --project=behaviour --workers=2
29 passed (12.2s)

npm run test:visual --prefix apps/desktop -- subagent.spec.ts qa-status.spec.ts --project=behaviour --workers=2
35 passed (17.0s)

npm run test:visual --prefix apps/desktop -- subagent.spec.ts trace-map.spec.ts --project=behaviour --workers=2
7 passed (3.8s)

git diff --check
# exit 0
```

最后一组包含新增的 Trace 页实时更新、同一子任务身份和陈旧 runId 过滤。构建仍有大于 500kB 的资源提示，构建成功；本任务未进行无关拆包。截图来自本地 stub 浏览器，验证子任务身份、结果与中断状态；测试输出和构建产物均被忽略。

## 审查范围与限制

- 本地自审覆盖协议/执行/权限/取消、文件变更与 undo、日志映射、Desktop DTO 和界面消费者；对已改跟踪文件逐个运行 log/blame/status 新鲜度检查，确认没有基线后新提交。
- 未进行独立人工代码审查、远程 CI 或真实模型联网测试；未运行全量像素基线测试。验证目标为相关行为和本地构建。
- 120 秒是通过既有 alive 机制执行的合作式超时；同步审批/阻塞 I/O 返回前不强杀 Rust 线程。测试覆盖超时分类，未实际等待 120 秒实测。文件快照继续遵循原有容量限制，容量不足时保守处理，避免错误 undo。
- 首版为前台、顺序、单层委派；未实现并行执行、后台续聊、跨提供方切换、跨进程恢复和自动 worktree 管理。
- 没有新增依赖、遥测或上传逻辑。

## Git 现场与恢复

工作目录：`/Users/lijianxue/work/WorkSpace/kodo`。初始 HEAD 与本地 main：`91fd94ecc5013e826d433bb9f8365251843bf791`；缓存 origin/main：`b20e714e25a761c1760da952b3086a4db5da909c`，本次未 fetch，不代表远程已同步。

提交前现场：

```text
git status --short --branch
## feat/subagent-execution
# 本任务实现与 specs 的修改/新增；另有原样保留的未跟踪提示文件

git branch --no-merged main
# 无输出：此时任务分支尚无独有 commit

git worktree list --porcelain
worktree /Users/lijianxue/work/WorkSpace/kodo
HEAD 91fd94ecc5013e826d433bb9f8365251843bf791
branch refs/heads/feat/subagent-execution
```

`CLAUDE_CODE_SANDBOX_IMPLEMENTATION_PROMPT.md` 原样保留，不纳入提交。7 条原有 stash 全部保留，未新增/pop/drop。原技能查看器提交经用户确认保留在 `91fd94e` 基线；后续检查时原 `feat/skill-viewer` worktree/分支已不存在，本任务未执行删除。其余既有分支不操作。

上述为提交前现场快照。用户随后明确授权本地提交、`--no-ff` 合并和合并成功后的任务分支清理；未授权 push、PR 或远程修改。执行时先检查基线与 staged diff，再提交、合并并复核。合并前可在 `feat/subagent-execution` 继续；合并后以本地 `main` 为恢复入口，通过 `git log --oneline --grep='subagent'` 定位实现及合并边界。实际提交与合并哈希、验证结果和保留现场见最终交付报告。

最终的继承模型调整后，另运行 `cargo test -p kodo-agent --lib subagent:: --offline`，结果为 `11 passed; 0 failed`；新角色不覆盖父角色已选择的模型。

## 完成状态与自动路由修复验收（2026-10-22）

修复基线：`15a7c1ed87244d42449f8642b57d5a99af8c988f`，唯一工作分支 `fix/subagent-delivery-routing`。本节补充并更新前述“未进行真实模型测试”的历史限制。修复和验证已通过；提交、合并与清理待结果可复核后的独立授权。

### 修复及回归

- FR-009 / AC-009：共享执行器在工具验收 Finish 且无模型正文时，按本轮实际变化路径和成功校验命令生成完成摘要；没有校验入口时明确未运行验证。真正的模型空回答、失败、截断、取消仍走原失败分支。
- FR-010 / AC-010：共享分类器识别明确的已有命令执行动作，避免泛称测试、读取 README 材料误选为编写测试或文档；明确作者任务与显式技能保持原语义，未扩大技能/Scope/审批权限。
- 新增 3 个回归用例：`executing_existing_commands_does_not_request_test_or_docs_authoring`（含中英文原始提示及作者任务反例）、`accepted_child_tools_produce_a_completion_summary_without_a_model_final`（有/无校验入口）、`unpinned_script_task_keeps_command_tools_and_cancels_its_process`（真实脚本启动和取消）。修复前路由用例实际失败为 Test/Feature 不符；完成正文问题有上一轮真实模型原始证据。未放宽既有失败/只读用例。

```text
cargo test --offline -p kodo-core -p kodo-agent -p kodo-shell -p kodo-desktop -p kodo-cli --lib --bins
test result: ok. 746 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 138 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 71 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 106 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

cargo check --workspace --offline
Finished `dev` profile [unoptimized + debuginfo] target(s) in 3m 38s

cargo fmt --all -- --check
git diff --check
# 均 exit 0
```

共 1114 个相关 Rust 测试通过。本次未改前端，不重复运行前端构建或像素基线；未运行远程 CI。

### AC-011：真实模型端到端

用户授权使用真实模型和临时示例项目，且禁止发送现有源码。实际使用应用当前配置的 `custom / MiniMax-M3.1-Flash`；没有固定上次的 mimo 配置或修改用户模型设置。验证器只向真实模型提供合成任务与临时文件，清空历史、记忆、用户技能、MCP、角色和 decision，禁用 fallback 与详细私有 Trace。认证仅由原有加载接口在进程内使用，未输出/复制/另存凭据。无现有仓库源码作为测试上下文。

两次初始执行尝试被临时验证器过窄的命令/辅助文件白名单阻断，均记为 failed 并保留证据，没有计为成功。随后在新目录中明确使用文件工具与限定验证命令，仍不带任何技能芯片；临时验证器复用审批回调为未知命令提供逐项审阅入口。最终通过的两次运行均未用到额外审批票据，未更改应用策略或依赖。

```text
# 临时验证器调用实际 /subagent、SandboxContext、TurnLogger 和 session::load
kodo-subagent-live-check <temporary-root> execution-final
LIVE_execution-final_PASS

kodo-subagent-live-check <temporary-root> cancellation-final
CANCEL_SIGNAL_AFTER_STARTED_MARKER at_ms=9519
LIVE_cancellation-final_PASS

# 在产物目录独立运行
node verify.cjs
SYNTHETIC_RESULT_OK count=3 total=21
exitCode=0
```

- 执行：18.315 秒，实际结果 `count=3,total=21`，`npm run test` 成功。4 次模型调用，输入 16640 / 输出 764 tokens；父 delivery=ready、verification=passed，子 status=completed。实际返回：“已完成本轮改动：`result.json`。已通过验证：`npm run test`。”不再包含“任务尚未完成/未产出可用回答”。
- 取消：9.553 秒，真实脚本先写 started.txt，再进入 30 秒等待；标记出现 1 秒后取消。子 cancelled，父 stopped=true、done=false。启动 57 秒后，脚本 PID 已不存在，must-not-exist.txt 仍不存在。2 次模型调用，输入 7794 / 输出 330 tokens。
- Trace：分别 71 / 38 次公开快照，每个会话重载后只有 1 个子项，最终内容与最后快照完全相等；分别 31 / 17 个公开步骤，无省略和私有思考/LLM I/O。以 parentId + childId 核对归属和稳定性；不同测试进程同秒生成的 childId 可相同，未宣称跨进程全局唯一。
- 范围：真实模型 HTTP → /subagent → 工具沙箱 → 共享日志 → 重载；没有进行原生桌面窗口的真实模型点击验收。

所有原始日志、JSON、会话、合成项目和临时验证器保留在系统临时目录 `kodo-subagent-repair-live-vlsgnxbw`，不提交运行数据。完整路径与本地 Git 结果见本轮交付报告。未读取或改动原有提示文件、stash 内容及其他任务分支；未 fetch/push/PR，远程新鲜度未核验。

最终兼容性复核补充了 `Create unit tests` 和“添加单元测试”的反例，并用“作者动作 + 测试类型”组合保留明确的测试编写路由。之后再次执行 `cargo test --offline -p kodo-agent --lib`：`746 passed; 0 failed`；重新离线链接验证器，在全新 `execution-final` / `cancellation-final` 示例目录复跑，上述真实模型结果为这一最终版本的证据，源码 SHA-256 与本地测试记录一致。此前通过和失败的所有尝试另行保留。
