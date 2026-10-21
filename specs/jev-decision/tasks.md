# Jev 决策辅助 · 任务

日期：2026-10-20。规格见 `spec.md`，方案见 `plan.md`，验收见 `acceptance.md`。

状态说明：`已验证` = 实现完成且有本地测试/命令证据；`已实现，验证待补` = 代码在位但缺少对应证据；`未开始` = 本期不做。

| 任务 | 目标文件 / 模块 | 依赖 | 覆盖需求 | 验证方式 | 状态 |
| --- | --- | --- | --- | --- | --- |
| T001 | `specs/jev-decision/{spec,plan,tasks,acceptance}.md` | 现场与分支确定 | 全部 | 文件映射、一致性检查 | 已验证 |
| T002 | `crates/agent/src/decision.rs`：固定 DTO、问题模板、响应校验、纯采纳策略 | T001 | FR-002/004/005/009 | `cargo test -p kodo-agent --lib decision::` | 已验证 |
| T003 | `crates/shell/src/decision.rs`：独立配置、独立凭据命名空间、配置代次 | T002 | FR-001/008/014 | `cargo test -p kodo-shell --lib decision::` | 已验证 |
| T004 | `crates/shell/src/decision.rs` + `apps/desktop/src-tauri/src/decision.rs`：最小状态、预览、授权、原子消费 | T003 | FR-006/007 | `cargo test -p kodo-shell --lib decision::`、桌面 `decision::tests` | 已验证 |
| T005 | `crates/agent/src/decision.rs` 客户端 + 壳层运行服务：超时、额度、并发、熔断 | T004 | FR-009/010 | `cargo test -p kodo-shell --test decision_transport` | 已验证 |
| T006 | `crates/agent/src/classify.rs`（基线评估）+ `crates/agent/src/lib.rs`（Observe 接入） | T005 | FR-003/004 | `cargo test -p kodo-agent --test decision_scenarios` | 已验证 |
| T007 | `crates/agent/src/verify.rs`（证据来源）+ `crates/agent/src/lib.rs`（失败 Observe） | T005 | FR-005 | `cargo test -p kodo-agent --lib verify::` | 已验证（单元级） |
| T008 | `crates/cli/src/{main,turn,decision}.rs`、`apps/desktop/src-tauri/src/{run,scheduler,main}.rs`、`evals/src/*` 的全部 RunRequest 构造点 | T006/T007 | FR-012 | `cargo check --workspace --all-targets` | 已验证 |
| T009 | 设置小节、发送预览、Inspector 决策记录、i18n | T004/T008 | FR-002/011/014 | `npm run typecheck`、`npm run build` | 已实现，视觉用例待补 |
| T010 | `evals/src/decision.rs` + `evals/fixtures-decision/`：虚构语料、标签、离线对照与契约检查 | T002/T008 | FR-013 | `cargo run -p kodo-evals` | 已验证（离线部分） |
| T011 | 冻结策略后按场景实现 Assist 与硬约束复核 | T009/T010 且质量门通过 | FR-003/004/005 | 质量门评测 | **未开始 — 质量门未通过** |
| T012 | 兼容、回退、完整验收与交付记录 | T011 | 全部 | AC-001～020 | 已实现，验收记录见 `acceptance.md` |
| T013 | OpenRouter 来源：目的地、凭据隔离、单字母作答与 logprobs 概率、模型一致性、无概率不采纳 | T012 | FR-015 | `cargo test -p kodo-agent --lib decision::`、`cargo test -p kodo-shell --test decision_transport`、`cargo test -p kodo-agent --test decision_scenarios` | 已验证 |
| T014 | 腾讯 EdgeOne 来源：固定目的地 `/v1/systemone`、`decision.tencent` 凭据、原生 System One、`@makers/jev` 别名与解析版本校验、设置页锁定模型 | T013 | FR-015 | `cargo test -p kodo-agent --lib decision::`、`cargo test -p kodo-shell --lib decision::`、`cargo test -p kodo-shell --test decision_transport`、`npm run typecheck`、视觉 `decision.spec.ts` | 已验证（回环模拟） |

## 关于 T011

Assist 的**代码路径**已经存在并通过模拟验收（`decision_scenarios` 的 `assist_changes_only_the_task_category`），但真实启用受 `spec.md` 第 6 节的质量门控制。离线对照已经给出第一组证据：

- 任务分类留出集：本地规则 Macro-F1 0.604，准确率 0.608。
- 失败分流：本地规则准确率 0.383，Macro-F1 0.203。
- 设计建议的 margin=4 会让 91.7% 的语料被判为"含糊"。

这些数字是**本地规则**的，不是模型的。模型的对照需要一次单独授权的云端评测；在那之前，Assist 保持关闭。

## 2026-10-21 评审修复

| 任务 | 范围 | 覆盖 | 验证 | 状态 |
| --- | --- | --- | --- | --- |
| R001 | Shell 代次与取消校验、Agent 采纳前复核 | RF-001 | 预览前换代、在途换代/取消回归 | 已本地验证 |
| R002 | 后端质量门降为 Observe、设置与提示 | RF-002 | Assist 配置不可采纳、界面禁用 | 已本地验证 |
| R003 | OpenRouter 严格单字母及完整概率 | RF-003 | 拒答、错误 Token、缺失选项回归 | 已本地验证 |
| R004 | 配置恢复单行回写 | RF-004 | 第二次读取大小不变且 recovered=false | 已本地验证 |
| R005 | 共享并发与按来源/凭据代次熔断 | RF-005 | 跨运行并发、401 与冷却探测 | 已本地验证 |
| R006 | 对应编译、回归与交付记录 | RF-001～005 | 离线 Rust、前端检查和模拟边界 | 本地验证通过，已获提交与本地集成授权；最终 Git 结果见交付说明 |

## 未纳入本期的范围

- 知识片段重排、记忆价值筛选、模型路由（`spec.md` 第 2 节 P3）。
- 连接检查按钮的真实请求（命令已预留，真实外发需要单独授权）。
- 全量规模的冻结语料（600 条任务分类 + 300 条失败分流）：本次落地的是 120 + 60 条虚构语料，足以跑通离线对照与契约检查，但不足以支撑质量门的统计结论。
