# 验收记录

首版历史状态：实现验收通过；用户已明确授权本地提交、`--no-ff` 合并及合并后清理本任务分支。实际集成哈希与 main 交付门结果记入最终交付报告。

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

## 普通任务自动委派验收（验证通过，待本地集成授权）

基线 main：`1faed29297986a52566841ab15cfbc545739eef0`。用户已授权创建唯一任务分支 `feat/automatic-subagent-routing`；原提示文件、7 条 stash 和其他分支保持不动。

验收目标为不含 `/subagent`、角色或委派要求的普通任务自动按需选择工具；简单任务直接完成。离线回归仅证明策略传播、权限过滤和主子执行协议；模型自主判断需单独的合成项目真实模型证据。AC-012 至 AC-017 已由最终版本的离线及真实合成项目验证覆盖，最终证据见文末。本任务提交与 `--no-ff` 合并记录见文末「本地集成与交付记录」。


### 自动委派初版与离线验证（否定动作修复前）

- `protocol.rs` 的唯一工具描述同时进入主系统提示、native schema 和 JSON fallback；`lib.rs` 仅在项目与可用 registry 允许时提前注入。普通多模块任务按语义拆分一个模块或一个聚焦问题，简单任务直接处理，用户不委派要求优先；不增加关键词调度器、额外路由请求或依赖。
- `plan.rs` 只将 child 实际 `changed_files` 作为父编辑证据，不把 child 的命令或正文冒充父验证。向后兼容的 `has_delegated` 默认字段要求新的父级回答。
- `state.rs` 清除委派前的完成声明，验证后回到 Execute 完成剩余工作；失败工具已有真实变更时也使旧验证失效。真实文件工具回归覆盖 child 写入、父级再写入和新汇总，另覆盖只读委派与部分失败。
- `subagent.rs` 新增 4 个执行/策略回归，`state.rs` 新增 3 个状态回归，旧检查点测试增加缺省兼容断言；权限、深度、4 次上限、取消和超时限制保持原实现。

```text
cargo test --offline -p kodo-core -p kodo-agent -p kodo-shell -p kodo-desktop -p kodo-cli --lib --bins
test result: ok. 753 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.80s
test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
test result: ok. 138 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s
test result: ok. 71 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
test result: ok. 106 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s

cargo check --workspace --offline
Finished `dev` profile [unoptimized + debuginfo] target(s) in 3m 42s

cargo fmt --all -- --check
git diff --check
# 均 exit 0
```

共 1121 个相关 Rust 测试通过。未改前端，未重新运行前端构建、像素基线或原生桌面点击验收；未运行远程 CI。最终 Rust 源码散列记录为临时证据目录的 `evidence/focused-source-hashes.json`，真实验证器已离线重新链接这一版本。

### 真实模型边界与保留的失败证据

实际使用当前配置的 `custom / MiniMax-M3.1-Flash`。仅向模型发送新建的合成任务及示例文件；请求清空历史、记忆、用户技能、MCP、角色和 decision，禁用 fallback 与私有模型 I/O Trace。认证仅由原有加载接口在进程内使用，未输出、复制或保存凭据，未修改用户模型设置。验证器单次输出上限临时设置为 8192 tokens、外层最多等待 360 秒；产品 child 的 120 秒限制未扩大。所有真实调用顺序执行。

原始证据保留于系统临时目录 `kodo-subagent-auto-live-4dvgck7d`，不提交测试运行数据。早期仅在工具描述中提供策略的尝试未触发可靠委派；提高临时输出限额仍未形成完整结果，相关失败没有计为通过。`attempt-3` 已自动启动 child，但 child 超时，外层 160 秒等待到期；其取消及禁止委派试验也未形成完整通过结果。早期验证器曾把外层超时记为 Complete，现已纠正为 Stopped；旧样本的 passed 始终为 false，未作为产品成功证据。

`attempt-5/automatic` 在 92.816 秒遇到连接重置，未创建 child；`attempt-5/simple` 24.361 秒直接回答、0 child。`attempt-6/automatic` 自动创建了独立复核 child，实际读取文件，120.095 秒后正确标为 timed_out；父级 282.092 秒自行完成并明示子任务超时，112 次公开快照重载相等。这条试验仍判为未通过，正文也超过临时任务的长度要求。最终策略因此进一步限定每次只委派一个模块或一个问题，避免整单重复复核；最终版本的真实模型结果尚待本节后续记录。


最终提示版本的 `attempt-7/automatic` 实际自动委派，但 child 反复搜集精确行号，97.744 秒后因工具额度不足而 failed（已用 30/32 次，下一批申请 5 次，未执行）；父级 285.955 秒后自行汇总，144 次 Trace 快照重载一致。该样本仍未通过。`attempt-8` 将证据要求改为函数名，但其“给出具体数值”的要求促使 child 请求命令执行，被用例的仅文件工具限制拒绝，19.534 秒后正确标为 failed；同样不计为通过。后续正例明确仅静态分析，不要求行号或运行样例，保留真实执行、child 完成、父级结果和 Trace 重载的全部通过门。项目实现没有因这些用例扩大工具权限或执行预算。

否定动作修复前已通过的反例与取消：

| 用例 | 真实模型结果 |
| --- | --- |
| `attempt-7/simple` | 28.921 秒，4 次父模型调用，0 child，父 ready；正确解释 count 与 total，但回复未严格限制为一句话 |
| `attempt-7/no-delegation` | 103.525 秒，4 次父模型调用，0 child，父 ready；显式不委派要求得到遵守 |
| `attempt-7/cancellation` | 40.385 秒，实际 child 文件读取后于 39.403 秒发出取消；child cancelled，父 stopped=true、done=false |

取消用例记录了 32 次公开快照、15 个子步骤，无省略，输入/输出 tokens 为 3165/110；实际读取 SPEC.md、checkout.js、orders.js、inventory.js。重载后的完整子 Trace 与最后快照相等，父子 ID 对应正确，无私有思考/模型 I/O。全组 20 个合成输入文件的 SHA-256 均未变化。此轮为模型任务取消，未另起长时间脚本；脚本进程取消证据见前一节 AC-011，不混称为本轮桌面点击或进程树验收。


### 只读解释误选 docs 的实际根因与修复

`attempt-9/automatic` 使用静态分析任务，自动 child 聚焦 checkout.js 的一个函数；child 61.811 秒后 blocked，父级 245.502 秒后自行汇总，80 次公开快照重载一致。实际阻塞原文指出：README.md / SPEC.md 与任务都禁止写入，但自动选中的 docs 技能要求“在项目内写出文档文件”。此样本仍为 failed，不计为自动完成正例。

根因是共享问答判定把“禁止执行”“不要修改”及交付说明中的“未执行”视为正向任务动作，材料名 README.md 又使分类落入 docs。修复以同一个 `requests_action` 判断常见紧邻否定前缀，供问答判定、文档输出意图与文件输出契约复用；原消息和约束继续完整传给模型。没有按只读 Scope 撤销技能，没有增加权限、模型调用或依赖。显式技能、明确文件输出、真正修复/更新和正式代码审阅继续保留原路径。

新增 4 个回归：中英文问答/否定动作与真实正向动作；否定文档输出与真实保存；否定动作的输出契约；Code/Work 实际只读 child 读取 README.md 后交付正文。实现前先运行其中两条，实际得到 `7 passed; 2 failed`；最初修复后针对性运行 `11 passed; 0 failed`，既有只读写入拒绝和正式代码审阅回归仍通过。随后补齐真实任务里的“未执行/没有修改”表述，全量测试与同任务新合成目录复验待下节记录。


只处理否定动作后的 `attempt-10/automatic` 再次自动委派，但 child 在 120.037 秒超时，父级 308.919 秒后自行完成，98 次快照重载一致，仍未通过。生成的子任务提到了 `repository.save / catch`；共享输出意图又把方法名内的 save 当成保存指令。为此在现有 4 条回归中加入方法名、函数调用和路径材料案例，实际先得到 `7 passed; 4 failed`，修复英文单词/方法/路径边界后得到 `11 passed; 0 failed`，包括 Code/Work 实际 child。真实 save/write 指令与正向创建仍有反例保护。没有移除文档技能、审阅技能或文件交付验收门。

`attempt-10` 的简单任务（9.820 秒、1 次模型调用、0 child）、禁止委派（90.969 秒、3 次模型调用、0 child）和取消（87.434 秒、child 实际读取后于 86.241 秒取消）已通过。该组保留为方法名修复前证据，不代替最终源码的完整端到端验收。


项目概览入口也存在独立的旧动作判定；使用真实任务的“只读复核/不一致/逐步说明”措辞后，针对性回归实际为 `9 passed; 2 failed`。最后将该入口接入同一 `requests_action`，四处意图判断一致，不增加语义模型调用，不按 Scope 绕过技能。

### 收敛版本离线证据

最终 6 个 Rust 文件散列为临时目录 `evidence/converged-source-hashes.json`。默认并发的首次全量运行得到 `756 passed; 1 failed`：既有 MCP 管道取消测试未满足 1 秒耗时断言。保留原日志，未修改该模块或断言；同一源码单独复验为 `1 passed; 0 failed`（0.25 秒），随后以 4 个测试线程重跑整个相关范围通过：

```text
cargo test --offline -p kodo-core -p kodo-agent -p kodo-shell -p kodo-desktop -p kodo-cli --lib --bins -- --test-threads=4
test result: ok. 757 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 17.79s
test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test result: ok. 138 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.57s
test result: ok. 71 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.32s
test result: ok. 106 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.73s

cargo check --workspace --offline
Finished `dev` profile [unoptimized + debuginfo] target(s) in 56.16s
```

共 1125 项通过。计时失败、单独复验和完整重跑日志分别为 `converged-broad-tests.log`、`converged-timing-recheck.log`、`converged-broad-recheck.log`；未将首次失败改记为通过。下一节真实模型验收采用与 `attempt-9/10` 相同的普通静态任务，输入不含委派指令。


### 只读能力与自动技能工作流一致性

`attempt-11/automatic` 再次自动委派 checkout 静态复核，但“不要分析其他模块内部实现”中的名词触发写入工作流；child 64.498 秒后因 10/10 轮预算耗尽而 failed，父级 229.799 秒后自行汇总，119 次快照重载一致。简单任务和禁止委派仍通过；该组取消用例在创建 child 前遇到连接重置，119.307 秒后失败，不能作为取消通过证据。

据此增加 Scope 能力检查：只对未显式选技能、未明确要求保存产物、自动技能含 Edit 工作流的只读 child 使用分析交付，清除误推的文件产物门。父白名单、Scope 工具过滤、120 秒/32 工具等上限不变；显式 docs、`.md/.txt` 保存要求和原只读审阅工作流不降级。名词“内部实现/输出为结论”的 Code/Work 回归先得到 `11 passed; 1 failed`，修复后 `12 passed; 0 failed`。新增反例还验证正式审阅不能用没有评分/证据的普通回答冒充完成。

Scope 版本 6 个 Rust 文件散列保存在 `evidence/scope-final-source-hashes.json`。完整相关测试以 4 线程运行，通过结果为：

```text
cargo test --offline -p kodo-core -p kodo-agent -p kodo-shell -p kodo-desktop -p kodo-cli --lib --bins -- --test-threads=4
test result: ok. 758 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 15.54s
test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
test result: ok. 138 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.40s
test result: ok. 71 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s
test result: ok. 106 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s
```

共 1126 项通过；原默认并发计时失败记录仍保留。后续真实模型复验仍使用同一普通静态任务、全新合成目录，不改变外层通过门。


### 明确文件产物与静态写入描述

`attempt-12` 的自动正例在 130.515 秒完成，1 个 child 实际读取 inventory 与 SPEC，40.626 秒完成；父级 3 次模型调用、47 次公开快照重载相等，实际汇总三个模块。简单任务 45.309 秒、0 child。禁止委派和取消均在创建 child 前遇到连接超时（1.746 / 1.807 秒），不算通过。人工审阅发现父级额外假定了未提供的 repository.save 覆盖语义，该推断不计入已验证事实；流程通过不等于全部语义结论准确。

收尾反例进一步复现：静态描述“catch 分支写入 failed 记录”会阻止只读分析，而目标在动作前的“将 report.txt 保存”会漏判。实现前的实际 child 回归为 `10 passed; 2 failed`，结合输出契约目标与动作出现位置后为 `12 passed; 0 failed`。规则允许目标位于动作前后，保留明确匿名文档要求；这只是只读 child 的自动技能能力协调，不改变全局文件契约或工具权限。

加入英文冒号保存反例后，第一轮完整测试为 `757 passed; 1 failed`，失败输入为 `Save: "report.txt".`。原日志 `file-target-final-broad-tests.log` 保留；该版 workspace 检查和临时验收器构建因需修复而主动中断（exit 130），未记为通过。修正动词后冒号与路径字符的边界，保留文件名/方法名保护后，针对性复验为 `12 passed; 0 failed`，日志 `file-target-v2-routing.log`。最终源码以 `file-target-v2-source-hashes.json` 固定，后续完整验证及真实模型证据另记下节。

此局部文件目标规则不解析长距离指代或复杂自然语言；显式只读工具限制始终保留。未扩大 child 预算、权限或模型配置。


### 最终收敛结果（普通任务自动委派）

实现、回归与真实模型流程验收通过；Git 集成记录见文末「本地集成与交付记录」。仅修改 6 个 Agent Rust 文件与本规格目录的 4 份文档，未新增依赖，未改 UI。最终 6 个源码散列在本地测试、验收器链接与真实模型运行后完全一致，记录为临时证据目录 `evidence/file-target-v2-source-hashes.json`。

```text
cargo test --offline -p kodo-core -p kodo-agent -p kodo-shell -p kodo-desktop -p kodo-cli --lib --bins -- --test-threads=4
test result: ok. 758 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 16.23s
test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
test result: ok. 138 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
test result: ok. 71 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.32s
test result: ok. 106 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s

cargo check --workspace --offline
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4m 01s

cargo fmt --all -- --check
# exit 0
git diff --check
# exit 0
```

共 1126 项通过；针对性只读回归另为 12 项通过。临时验收器离线重建通过（51.96 秒）。完整日志为 `evidence/file-target-v2-{broad-tests,workspace-check,fmt,harness-build,routing}.log`，早期失败日志继续保留。

真实模型最终组为全新 `attempt-13`，仍使用同一条不含委派要求的普通静态分析任务；提供方为 `custom / MiniMax-M3.1-Flash`。四项顺序执行、全部通过流程门：

| 场景 | 耗时 | 父级模型调用 | 子任务及结果 | Trace |
| --- | --- | --- | --- | --- |
| 自动判断复杂任务 | 168.524 秒 | 3 | 自主委派 checkout，实际读取 5 个示例文件，81.427 秒 completed；父级新正文覆盖三个模块，ready/done | 40 次快照，重载相等 |
| 简单问答 | 23.522 秒 | 5 | 0 child，父级 ready/done | 无子 Trace |
| 明确禁止委派 | 172.907 秒 | 4 | 0 child，父级 ready/done，覆盖三个模块 | 无子 Trace |
| 子任务读取后取消 | 50.104 秒 | 2 | 在父级计时 48.586 秒触发取消；子 cancelled，父 stopped=true/done=false | 30 次快照，重载相等 |

自动 child `subagent-1792667568-0` 归属父会话 `tnb345-d0xh0w-0`，输入 9760 / 输出 3413 tokens；取消 child `subagent-1792667934-0` 归属 `tnb3eb-16b3sg-0`，输入 3578 / 输出 85 tokens。公开步骤过滤、实际 fileRead、父子 ID 与持久化末次快照逐项核对通过。取消是实际 child 读取之后触发，未把未启动或连接失败算成取消。20 个示例输入的前后 SHA-256 全部相同。

人工核对自动正例：child 正确发现支付在幂等检查前、失败不释放库存/撤销扣款及成功保存未 await；父级正确汇总金额校验、非原子批量预留、重复释放、共享快照与新完成正文。仍有模型语义限制：child 将非原子的 reserveBatch 误称为原子（父级相关描述正确）；仓储实现未提供，覆盖/写入顺序推断不能当成确定事实；负优惠比例不必然产生负总额。细节保存在 `attempt-13/evidence/manual-quality-review.json`，未认证全部结论准确，也未执行代码或数值样例。简单任务虽然没有委派，仍发生了 5 次模型调用，不宣称单次调用或延迟优化已验收。

原始合成任务、4 份 JSON 结果、公开 Trace/session、日志、输入散列与人工核对均仅在系统临时目录 `kodo-subagent-auto-live-4dvgck7d/attempt-13/evidence` 保留，不提交。未向模型发送现有项目源码、历史、用户技能或项目日志。未运行原生桌面点击、前端构建或远程 CI；未核验其他模型的稳定性。自动判断依赖模型，120 秒合作式 child 预算及所有权限上限不变。


### 本地集成与交付记录（Git 提交 / `--no-ff` 合并）

基线：`1faed29297986a52566841ab15cfbc545739eef0`（本地 `main`；本次核验时 `origin/main` 为 `b20e714`，本地 main 领先 6 个提交）。
任务分支：`feat/automatic-subagent-routing`，从上述基线创建，是本任务的唯一工作分支。

本任务提交（均在任务分支上）：

| 提交 | 内容 |
| --- | --- |
| `3c4ff49` | `feat(agent): delegate read-only work and respect negated actions` — 7 个 Agent Rust 文件 + 本规格目录 4 份文档 |
| `dbdf696` | `fix(sandbox): start the probe helper with the magic flag` — `crates/sandbox/src/probe.rs` + `docs/design/SECURE_EXECUTION.md` |
| `f688237` | `docs(sandbox): add the secure execution implementation brief` — 根目录实施提示文件 |

合并方式：任务分支以 `--no-ff` 合入本地 `main`（保留任务边界），合并后按仓库规范删除已完成任务分支；7 条 stash 与其它分支保持不动。合并前的本地验证：

```text
cargo fmt --all -- --check
# exit 0
git diff --check
# exit 0
cargo check -p kodo-agent -p kodo-sandbox
# Finished `dev` profile [unoptimized + debuginfo] target(s) in 42.44s
cargo test -p kodo-agent -p kodo-sandbox
# kodo-agent: 760 passed; 3 failed; 0 ignored; kodo-sandbox: 全部通过
```

3 个失败用例属环境原因，与本任务改动无关：在未改动的基线 `1faed29` 上以 `git worktree add --detach` 复跑同样 3 个用例、同样 3 个失败（对照 worktree 随后移除）。

- `tests::git_status_permission_runs_in_auto_and_full_but_ask_waits`：测试沙箱内 git 报 `couldn't create cache file '/var/folders/.../xcrun_db-...' (errno=Operation not permitted)`；
- `subagent::tests::accepted_child_tools_produce_a_completion_summary_without_a_model_final`：同一 git 错误加上 `/bin/sh: npm: command not found`，验证步骤失败使 child 状态落到 blocked；
- `verify::tests::a_missing_tool_blocks_same_binary_and_skips_the_doomed_chain`：断言依赖 `node -e`，本机未安装 node。

本机未安装 node/npm，因此本轮未运行前端构建、Playwright 视觉基线与原生桌面点击验收；本任务未改前端，也未运行远程 CI。合并后按用户授权推送远端，推送结果与最终合并哈希见交付报告。
