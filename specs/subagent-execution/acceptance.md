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
