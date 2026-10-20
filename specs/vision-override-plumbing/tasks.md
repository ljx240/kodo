# Tasks — 图片附件多模态支持：per-model vision 透传 + 路径兜底明示

依赖顺序自上而下。

## T001 — Provider 加 model_vision_override 字段

- 目标文件：`crates/agent/src/provider.rs`
- 内容：给 `Provider` 结构体新增 `pub model_vision_override: Option<bool>`；`Provider::new`、`Provider::from_record` 默认 `None`；测试构造器（line 525 等）的字面量同步加 `model_vision_override: None`。
- 验收：单测 `capabilities_unchanged_when_model_vision_override_is_none` 通过；`cargo test -p kodo-agent provider::tests` 全部现有用例无回归。
- 依赖：无。

## T002 — Provider::capabilities() 应用 override

- 目标文件：`crates/agent/src/provider.rs`
- 内容：`capabilities()` 内部派生 caps 后，若 `model_vision_override.is_some()`，把 `caps.vision = *override`。
- 验收：3 个新单测全部通过（`model_vision_override_overrides_template_default_when_some` / `model_vision_override_false_disables_vision_for_anthropic_template` / `capabilities_unchanged_when_model_vision_override_is_none`）。
- 依赖：T001。

## T003 — ShellConfig 读 settings.log 的 models[].vision

- 目标文件：`crates/shell/src/config.rs`
- 内容：`ProviderJson` 加 `#[serde(default)] models: Option<Vec<ModelJson>>`；`ModelJson { model_id, vision }`；`build` 闭包内按 (model_id or model) 命中模型后写 `provider.model_vision_override`。
- 验收：单测 `model_vision_override_round_trips_through_settings_log` 通过（4 个子断言）。
- 依赖：T001（写 `model_vision_override`）。

## T004 — image 路径兜底 observation 文案明示

- 目标文件：`crates/agent/src/lib.rs`
- 内容：在 `vision_ok=false` 分支判断 `attachment_mime_from_ext` 是否 image；是 → 改写 observation 为「image 附件 + 路径 + bytes + 显式『当前模型不支持视觉输入』」；否 → 保留既有 office/binary 措辞。
- 验收：单测 `image_attachment_emits_explicit_no_vision_observation_when_vision_ok_false` 通过；`office_attachment_observation_unchanged` 通过。
- 依赖：无（独立改文案）。

## T005 — Analyze 复审与构建

- 检查 plan 与 spec 的覆盖一致性。
- 跑 `cargo build -p kodo-agent` 与 `cargo build -p kodo-shell`；`cargo test -p kodo-agent`；`pnpm typecheck`（desktop）。
- 验收：编译通过；现有单测无回归；新单测全过。
- 依赖：T001–T004。

## T006 — Converge：手动端到端 + UI 验收

- 启动 desktop dev server，`/ui-demo/conversation` 设置 dark mode + 自定义 provider + 视觉模型，发起带 PNG 的回合，截图记录；比对 AC-001/AC-002 的实际行为。
- 验收：AC-001 模型回复里含像素相关引用；AC-002 模型回复承认限制而非编造。
- 依赖：T005。