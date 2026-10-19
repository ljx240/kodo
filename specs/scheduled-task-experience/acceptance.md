# 定时任务体验验收记录

日期：2026-10-19（Asia/Shanghai）。状态：功能实现与定向验收通过；用户已明确授权提交、合并和推送。最终集成与推送状态以 main 提交和远程引用核验为准。

## 交付范围

- `apps/desktop/src/pages/SchedulerPage.tsx`：创建编辑入口、模板、状态/项目/类型与文本过滤、归档查看、运行历史、实时和恢复同步、取消反馈。
- `apps/desktop/src/pages/SchedulerEditor.tsx`、`schedulerModel.ts`、`scheduler.css`：原生 dialog、焦点恢复、四种时间规则、IANA 时区、DST、后端五次预览、表单校验、稳定创建 ID、冲突保留草稿及局部样式。
- `apps/desktop/src/App.tsx`、`i18n.ts`：项目与结果会话接入、中文状态/原因；保留原有 LLM 文案改动。
- `crates/core/src/scheduler.rs`：修正枚举字段 camelCase 契约，保留旧 snake_case 读取别名与回归测试。
- `crates/shell/src/scheduler.rs`：持久化响应、手动试跑游标、漏执行选择、宽限与 Skip、一次任务结束、归档与 revision 保护及回归测试。
- `apps/desktop/src-tauri/src/scheduler.rs`、`scheduler_cmds.rs`：当前时间默认预览、输入/作业校验、资源拒绝的终态；原有 force_inline 字段完整保留。
- `apps/desktop/tests/visual/scheduler.spec.ts`、`page-polish.spec.ts`、`playwright.scheduler.config.ts`：9 项新隔离交互场景、既有 1 项定时任务视觉回归适配、独立服务器。

## 验收对应

| 验收 | 证据 | 结论 |
|---|---|---|
| AC-001 | Agent 模板创建；Job JSON 错误及修正；revision 冲突及存储失败后草稿/创建 ID 保留 | 隔离浏览器通过 |
| AC-002 | 四种表单规则；未来预览；上海 UTC 转换；洛杉矶 DST 空缺拒绝和重复时刻取早；Rust 时间计算 | 浏览器与 Core 通过 |
| AC-003 | 暂停、手动执行、取消、归档、搜索；后端归档拒绝活跃运行；归档不可恢复启用 | 浏览器与 Shell 通过 |
| AC-004 | 延迟 A 历史后展开 B，A 返回不覆盖；run_changed 事件刷新 B | 隔离浏览器通过；5 秒和焦点恢复路径已实现，未单独模拟真实事件丢失 |
| AC-005 | 中文结果、摘要和报告引用；结果入口触发指定 load_session 并导航会话 | 隔离浏览器通过 |
| AC-006 | 旧 JSON 与前端 JSON 对应；序列化为 camelCase；创建/编辑返回实际下一次时间 | Core/Shell 通过 |
| AC-007 | 手动执行和标题编辑不改游标；超过六次补跑及自定义宽限；Skip 正常准时触发；一次结束 | Shell 通过 |

## 验证命令与原始结果摘要

以下均在本机执行，没有调用真实 Provider、用户调度数据库或外部业务服务。

```text
npm --prefix apps/desktop run typecheck
> kodo-desktop@0.0.1 typecheck
> node ./node_modules/typescript/lib/tsc.js --noEmit
(exit 0)

cargo check --offline -p kodo-desktop
    Checking kodo-desktop v0.0.1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.17s

cargo test --offline -p kodo-core --lib scheduler::tests
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 110 filtered out; finished in 0.01s

cargo test --offline -p kodo-shell --lib scheduler::tests
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 0.08s

cargo test --offline -p kodo-desktop --bin kodo-desktop scheduler::tests
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 35 filtered out; finished in 0.00s

rustfmt --edition 2021 --check crates/core/src/scheduler.rs crates/shell/src/scheduler.rs apps/desktop/src-tauri/src/scheduler.rs apps/desktop/src-tauri/src/scheduler_cmds.rs
(exit 0)

git diff --check
(exit 0)
```

浏览器命令（apps/desktop 中执行，使用已有本地 Node，不安装依赖）：

```bash
export PATH="/Users/lijianxue/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH"
node node_modules/@playwright/test/cli.js test --config playwright.scheduler.config.ts --grep 'scheduled task details|template creates|editing keeps|invalid weekdays|revision conflict|late history|pause, manual|run results|timezone conversion|narrow editor' --workers=2
```

```text
Running 10 tests using 2 workers
10 passed (8.5s)
```

包括 680px 编辑器焦点/布局，以及 900px、1586px 列表与历史的深浅主题、对比度和溢出检查，截图已实际打开检查。

生产构建使用项目现有 Vite，显式设置 `envDir:false`，不读取 .env 文件；生成目录位于仓库外的本地临时目录，未覆盖已有 dist。构建通过，仍有应用原有大包体积提示；未为此改动无关打包方案。

## 检查中遇到的问题

- 系统 Node 18 无法运行当前 Playwright；切换已安装的 Codex bundled Node 后通过，没有安装或升级依赖。
- 首轮浏览器选择器同时匹配侧栏和页面新建按钮，已缩小到页面范围；修正后继续验证。
- 实际修复了编辑器焦点、后台作业选择器的可访问名称，以及过滤空状态文案。
- 原有 misfire 测试通过“创建任务”模拟两小时前的漏执行，已改为持久化真实过期游标后模拟恢复；保留其跳过记录断言并增加准时触发回归。
- `cargo fmt --all -- --check` 未通过：会话开始时已有文件的格式差异，包含 main.rs、agent/lib.rs、agent/provider.rs、shell/config.rs、shell/turn.rs。本任务 Rust 文件的定向格式检查通过；没有格式化这些原有现场。

## 实际验证的限制

未启动真实 Tauri 桌面会话或真实模型调用，前端以隔离 IPC 测试、后端以临时数据库和可控时钟验证；真实 macOS 休眠/唤醒和用户已有任务迁移没有做现场试跑。原有重叠策略仅保留，不新增 QueueLatest 配置入口，不宣称实现其队列语义。当前任务未包含云端、系统后台常驻、RRULE 或会话 heartbeat。

## 干净提交快照验证与交付授权

2026-10-19 用户明确要求“完成合并 提交 推送”，并再次要求继续完成任务。此前确认保留的原有改动和 7 条 stash 不纳入本任务提交。

已将只含本任务的暂存树导出到仓库外的临时目录，复用现有依赖并关闭 Vite 环境文件加载。该快照不包含原有未提交的 provider、图片接入、Inspector 等改动，用于核验任务可独立集成。

干净快照中的验证原始结果：

```text
> kodo-desktop@0.0.1 typecheck
> node ./node_modules/typescript/lib/tsc.js --noEmit
(exit 0)
Clean task build passed.
Running 10 tests using 2 workers
10 passed (9.2s)

cargo check --offline -p kodo-desktop
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 41.42s
cargo test --offline -p kodo-core --lib scheduler::tests
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 110 filtered out; finished in 0.03s
cargo test --offline -p kodo-shell --lib scheduler::tests
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 57 filtered out; finished in 0.05s
cargo test --offline -p kodo-desktop --bin kodo-desktop scheduler::tests
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 35 filtered out; finished in 0.00s
cargo fmt --all -- --check
(exit 0)
```

干净提交快照的全仓 Rust 格式检查通过；此前格式差异仅存在于保留的原有未提交现场，未进入本次提交。交付前校验 36 个保留文件的 SHA-256 与 7 条 stash 均未变化。

Git 基线为 `1499af8ec192c2a7c92105b7fc3fc981cf92c0f0`；本轮已获联网授权并执行 `git fetch origin --prune`，确认当时 main 与 origin/main 均为该提交。仅提交前述 17 个本任务文件；工作目录始终为原仓库单一 worktree，任务分支为 `feat/scheduled-task-experience`。

采用已提交树生成保留两个父提交的合并边界（等价于 `--no-ff`），并核对合并树与已验证的任务树一致。原有未提交文件保持原位，以交付前后逐文件 SHA-256 校验；不使用 stash pop、reset、强制 checkout 或工作区清理。详细工作区清单和恢复快照仅保存在本机会话交付目录，不纳入推送。

最终提交、main 集成、推送结果与保留分支/stash 清单在本次交付汇报中记录。没有创建 PR 或执行远程 CI，不将本地验证称为远程 CI 通过。
