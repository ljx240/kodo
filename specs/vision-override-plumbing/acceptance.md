# Acceptance — 图片附件多模态支持：per-model vision 透传 + 路径兜底明示

## AC-001 (per-model vision 透传生效)

**证据**：`cargo test -p kodo-shell --lib model_vision_override_round_trips_through_settings_log`

子断言：
1. `custom` 模板 + 用户在 `models[]` 里标 `vision: true` → `Provider.model_vision_override == Some(true)` → `capabilities().vision == true`。✓
2. `custom` 模板 + 用户标 `vision: false` → `Some(false)` → `vision == false`。✓
3. `models[]` 缺失（legacy 记录）→ `model_vision_override == None`，模板默认（conservative → false）生效。✓
4. `models[]` 有但活跃 model id 不在里面 → `model_vision_override == None`。✓
5. `models[]` 命中但 `vision` 字段缺失 → `model_vision_override == None`。✓

`cargo test -p kodo-agent --lib provider::tests::model_vision_override`：
- `model_vision_override_none_keeps_template_default` ✓
- `model_vision_override_overrides_template_default_when_some` ✓
- `model_vision_override_applies_through_capabilities_override` ✓

## AC-002 (自定义模型未标 vision 时兜底)

**证据**：
1. `cargo test -p kodo-agent --lib tests::binary_attachment_produces_a_descriptor_observation` — PNG + `provider: None` (vision=false) 走兜底 observation，`notes` 仍进 canned answer 文本（`已附带附件 ... （内容未内联）`），pre_observation 文案改为 image-specific 「当前模型不接受图像输入，本轮未读取像素」格式。✓
2. `cargo test -p kodo-agent --lib tests::office_attachment_observation_keeps_existing_wording` — `.docx` + `provider: None` 走兜底，但 office/binary 分支文案保留「已附带附件：内容未内联（二进制/Office 格式）」措辞。✓

新文案（来自 `crates/agent/src/lib.rs:3244-3248`）：
```
<attachment path="{rel}" inlined="false" kind="image" bytes="{bytes}" />
[已附带图像附件（{bytes} 字节）。当前模型不接受图像输入，本轮未读取像素；
 请基于文件名/大小回答，需要像素分析请换用支持视觉的模型或粘贴报错文字。]
```

## AC-003 (硬编码目录行为不变)

**证据**：`cargo test -p kodo-agent --lib provider::tests::model_vision_override_none_keeps_template_default`
- `Provider::new("anthropic", ...)` + `model_vision_override = None` → `vision == true`。✓
- `Provider::new("deepseek", ...)` + `model_vision_override = None` → `vision == false`（deepseek() 工厂本身 vision=false）。✓

既有测试 `custom_provider_capabilities_are_conservative_by_default`、`capabilities_gate_native_tools` 等全过。

## AC-004 (硬编码目录 vision=false 行为不变)

**证据**：既有 `binary_attachment_produces_a_descriptor_observation`（provider=None，PNG 走兜底）通过；`office_attachment_observation_keeps_existing_wording` 通过。

## AC-005 (settings.log 无 models 字段的旧记录)

**证据**：`model_vision_override_round_trips_through_settings_log` 第三子断言：providers 记录只含 `id/name/template/endpoint/model/modelId` → `model_vision_override == None`，模板默认（conservative → vision=false）→ 与改动前完全一致。✓

## 全量回归

- `cargo test -p kodo-agent --lib` — 551 passed, 0 failed（547 既有 + 3 vision_override + 1 image attachment 调整）。
- `cargo test -p kodo-shell --lib` — 67 passed, 0 failed（66 既有 + 1 vision override round-trip）。
- `cargo build --workspace` — Finished。✓
- `apps/desktop` `npm run typecheck` — Finished, no errors. ✓

## 端到端限制

AC-001/AC-002 的「模型回复是否引用像素」实际需要真实 API（mimo-v2.6-flash 不在仓库 fixture 范围），单元测试 + shell 配置加载已经覆盖 plumbing 完整路径。运行端验证留给用户在配置该 provider + 视觉模型后实测。