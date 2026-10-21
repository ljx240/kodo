# 方案 — 编程任务可靠性修复

对应需求见 `spec.md`（FR-A/B/C/D/E）。本轮不引入新模块、新依赖、新抽象层；全部落在已有文件的最小安全改动上。

## 1. 涉及模块与数据

| 模块 | 角色 | 本轮改动 |
| --- | --- | --- |
| `evals/src/main.rs` | Layer B 夹具基线 | 套件形态判定（FR-A） |
| `evals/src/decision.rs` | 分类语料评分器 | 改用运行时统一标签函数、分列报告、保留旧口径（FR-B/C） |
| `crates/agent/src/classify.rs` | 路由 | 文档短路不再吞掉打分；`route_label()` 输出含 `unknown` 的统一词表（FR-B） |
| `crates/agent/src/verify.rs` | 归因 | 新增分层证据 `FailureTriage`；`classify_verify_failure` 保持兼容 |
| `crates/agent/src/state.rs` | 预算 | 估算 vs 实测、输出预留、剩余空间 |
| `crates/agent/src/compaction.rs` | 压缩 | 无 usage 也触发；收缩保留配对与约束 |
| `crates/agent/src/lib.rs` | 运行循环 | 发送前适配、超窗有界恢复、压缩失败可见回退 |
| `crates/agent/{recovery,plan}.rs` | 连续性 | 显式约束进入 plan；轮内文件变更使旧验证失效 |

不触碰：`crates/agent/src/decision.rs`、`crates/shell/src/decision.rs`、`specs/jev-decision/*`、desktop decision 文件（他人未提交现场）。

## 2. 关键设计决策

**D-A1 按形态判定，不按名字判定。** 编程夹具套件的本质特征是"每个条目是一个带 `prompt.md`/`oracle.sh` 的仓库目录"；分类语料的本质特征是"条目是 `.jsonl` 案例文件"。判定函数 `classify_corpus_dir(&Path) -> CorpusShape { FixtureSuite { fixtures }, ClassificationCorpus { cases }, Empty }` 只读目录结构，不读名字，因此新增任一目录都不会被误识别。`Empty` 一律 FAIL，不静默跳过（FR-A5）。

**D-A2 分类语料有自己的门，而不是没有门。** 对分类语料施加：可解析行数、`id`/`label`/`split` 齐备、tune 与 holdout 均非空、标签集合是该语料类型的合法标签。检查名保留 `_decision` 后缀，与历史报告可比。

**D-B1 单一标签事实来源在运行时。** `ClassificationEvidence::route_label()` 由 `hit_count`/`source`/`baseline` 推导；评测器不再自己 `match`。这样 UI、评测、日志共用一份口径。

**D-B2 文档短路降级为"仅在无代码产出命中时生效"。** 先跑关键词打分；只要 `Feature/BugFix/Refactor/Test` 有命中，就交给打分结果。理由：用户请求里的"Markdown/.md"是**产出格式**，不是**任务类别**；`is_document_output` 原本用来兜住"纯粹让我把答案存成文件"。`ClassificationSource::DocumentOutput` 与既有测试语义保持不变。

**D-C1 归因改为证据分层，不改 `FailureClass`。** `FailureClass` 是持久化枚举，直接加变体会让旧记录与新代码交叉失败。新增 `FailureTriage { Product, Infrastructure, Transient, Unknown }`（serde + `#[serde(default)]` 挂到 `VerificationEvidence`），`classify_verify_failure` 原样保留作为兼容入口，评测口径切换在同一份报告里并列输出新旧数字。

**D-C2 判据绑证据族，不绑宽泛词。** 四个族：errno/系统码名（`ENOSPC`/`EACCES`/`ENOENT`/`ECONNRESET`…）、退出码约定（127/126/137/139）、源码诊断形态（`error[Exxxx]`、`path:line:col`、`src/**.rs|ts|tsx`、`test result:`、`assertion failed`、`panicked at`）、网络瞬断形态（`connection reset`、`name resolution`、`rate limit`、`retry after`、`upstream connect`、`was canceled`）。无任一族命中且输出不含诊断内容 → `Unknown`；有内容但无族命中 → 维持 `Product`（保守，继续修复）。

**D-D1 估算用字符/4，与 `memory/retrieval.rs:223`、`knowledge/store.rs:1339` 一致。** 不引入 tokenizer。估算值与实测值在 `Budget` 里是两个字段，`percent_used()` 明确返回 `source`（`Measured` / `Estimated`）。

**D-D2 发送前适配 + 超窗有界恢复。** 新增 `fit_history_to_window()`：以服务商窗口减去输出预留为预算，先按 `tool_group_boundary` 收缩中段观测（不切调用/结果配对），并保证系统块、最后一条用户消息、plan 约束不被丢弃。它同时用于首轮（覆盖无 usage 的空白）与 `ContextTooLong` 后的第 1、2 次重试（`MAX_CONTEXT_RECOVERY_ATTEMPTS = 2`）。

**D-E1 复用 checkpoint，不新建机制。** 约束提取在 `TaskPlan::from_task` 里做一次，写入既有 `constraints`；`RecoveryCheckpoint` 已持久化它，`revalidate` 已保留文字契约。旧格式兼容靠 `#[serde(default)]`。

**D-E2 轮内失效。** 复用 `ContextManager::invalidate` 的调用点：文件写入成功后，把本轮已记录的 `verify_ok` 标记为"需重验"，不删除历史证据本身。

## 3. 兼容与回退

- 公开入口 `classify()`、`classify_verify_failure()`、`FailureClass`、`Provider` 请求体、工具协议签名不变。
- `VerificationEvidence` / `RecoveryCheckpoint` 只新增带 `#[serde(default)]` 的字段：旧记录可读，未知字段按 serde 默认忽略。
- 评测阈值不降；`default`=30、`real`=4 保持。
- 语料文件一行未改（`git diff` 为空即为证据）。

## 4. 验证方式

```bash
cargo test -p kodo-agent --offline            # 路由/归因/预算/连续性单测
cargo test -p kodo-evals --offline            # 目录形态回归测试
cargo run  -p kodo-evals --offline            # 默认离线评测，需 exit 0
cargo fmt --all -- --check
git diff --check
```

真实模型评测：`--live` 不运行，报告保持 `NOT RUN`。
