# 会话上下文洞察验收

功能分支 `feat/context-insights` 的开发基线为 `27a7f473b2aa28b155fc62eb1b43e74a3568a848`。主线集成以 `458ea0f856280614bb0932e576eb6a03f46455cc` 为基线。

- FR-001：Playwright 验证用量环打开面板，键盘选择 `/context` 打开空态，Escape 恢复焦点。命令前后发送记录数保持一致。
- FR-002、FR-003：Playwright 验证服务商用量、组成估算、无服务商用量时的估算标识、历史选择和实时更新。Agent 测试覆盖分类计量及实际响应的备用模型。
- FR-004、FR-005：核心日志测试覆盖有无组成字段的快照，Shell 测试覆盖关闭详细 Trace 时的快照与压缩回读。Playwright 覆盖旧摘要、切换和重开会话，检查面板只呈现统计值。

## 验证输出

前端命令使用本机 Node.js 22.15.1，在 `apps/desktop` 中执行。检查了浅色桌面和深色窄窗口的截图。

```text
npm run build
✓ 4150 modules transformed.
✓ built in 1.12s
(!) Some chunks are larger than 500 kB after minification.

npm run test:visual -- --project=behaviour composer-context.spec.ts behaviour.spec.ts --grep '上下文洞察|composer context ring' --workers=2
4 passed (4.6s)
```

```text
cargo check --locked -p kodo-desktop -p kodo-cli
Finished `dev` profile [unoptimized + debuginfo] target(s) in 54.32s

cargo test --locked --offline -p kodo-agent --lib context_composition_estimates_the_submitted_input
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 762 filtered out; finished in 0.00s

cargo test --locked --offline -p kodo-agent --lib context_budget_tracks_the_provider
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 762 filtered out; finished in 0.12s

cargo test --locked --offline -p kodo-agent --lib compaction::tests
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 744 filtered out; finished in 0.01s

cargo test --locked --offline -p kodo-core --lib context_budget_round_trips
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 137 filtered out; finished in 0.00s

cargo test --locked -p kodo-shell --lib turn::tests
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 99 filtered out; finished in 0.02s

cargo test --locked --offline -p kodo-shell --lib context_items_reload_without_detailed_capture
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 106 filtered out; finished in 0.00s

cargo test --locked -p kodo-desktop --bin kodo-desktop view::tests
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 53 filtered out; finished in 0.00s
```

`cargo fmt --all -- --check` 和 `git diff --check` 均以 0 退出。真实服务商、原生桌面包和远程 CI 待验证。

## 趋势对比、注入组成与裁剪

- FR-006：Playwright 覆盖正差值、负差值和零变化，验证切换计量来源或模型回退后缺少可比读数，验证趋势切换保留选择及输出 token 与耗时配对。
- FR-007：Agent 测试验证前置观测、记忆和知识库随实际发送的消息计量，文本工具结果归入工具用量。用户消息序列化保持原结构。Core 回读覆盖无组成、原六类组成及含注入明细的快照。Playwright 验证注入明细计入总量和已移除子项的差值。
- FR-008：Agent 测试验证发送前裁剪、服务商超限重试及消息上限裁剪的统计。Shell 和桌面 DTO 验证裁剪类型和条数经过会话日志与实时事件传递。Playwright 覆盖调整记录跳转、无后续请求的事件和旧压缩记录。检查浅色桌面及深色 `430 × 620` 面板。

```text
npm run build
✓ 4150 modules transformed.
(!) Some chunks are larger than 500 kB after minification.
✓ built in 1.72s

npm run test:visual -- --project=behaviour composer-context.spec.ts behaviour.spec.ts --grep '上下文洞察|composer context ring' --workers=2
4 passed (5.0s)

cargo check --locked --offline -p kodo-desktop -p kodo-cli
Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.41s

cargo test --locked --offline -p kodo-agent --lib context_
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 747 filtered out; finished in 0.44s

cargo test --locked --offline -p kodo-agent --lib -- compaction::tests compaction_failure_falls_back_to_the_deterministic_pre_send_trim oversized_first_request_is_reduced_before_sending_without_any_usage
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 743 filtered out; finished in 0.35s

cargo test --locked --offline -p kodo-core --lib context_budget_round_trips
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 137 filtered out; finished in 0.01s

cargo test --locked --offline -p kodo-shell --lib turn::tests
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 99 filtered out; finished in 0.03s

cargo test --locked --offline -p kodo-desktop --bin kodo-desktop view::tests
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 53 filtered out; finished in 0.00s

cargo test --locked --offline -p kodo-desktop --bin kodo-desktop view::tests::a_run_event_carries_its_lifecycle_tag
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 70 filtered out; finished in 0.00s
```

上述验证在 `feat/context-insights` 的工作树执行，基线为 `27a7f473b2aa28b155fc62eb1b43e74a3568a848`。`git apply --check` 显示会话页面和 Memory outbox 的补丁需要集成时复核。结束检查时本地 `main` 为 `6d8ae275b4368f784a50abe233bfd412dfd8f461`，并行的长任务轮次预算修复涉及 Agent 入口。缓存 `origin/main` 为 `f20f5de9d4af05f1f46d42fd12743cd595dcde9a`，本次未刷新。

## 主线集成验证

合并保留 Office 附件预览、工具注册及已验证回合的 Memory 写入规则。更新原文件查看器用例，验证当前 Office 内嵌预览和系统打开入口。

```text
npm run build
✓ built in 10.94s

cargo check --locked --offline -p kodo-desktop -p kodo-cli
Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.94s

Agent: test result: ok. 40 passed; 0 failed
Core: test result: ok. 2 passed; 0 failed
Shell: test result: ok. 10 passed; 0 failed
Desktop view: test result: ok. 18 passed; 0 failed
```

Playwright 的 4 项上下文洞察、2 项 Memory、1 项附件卡片用例通过，更新后的 Office 预览用例复测通过。`cargo fmt --all -- --check` 和 `git diff --cached --check` 均通过。
