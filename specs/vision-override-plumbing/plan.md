# Plan — 图片附件多模态支持：per-model vision 透传 + 路径兜底明示

## 模块与改动面

### 1. `crates/agent/src/provider.rs`

- 给 `Provider` 结构体新增字段 `pub model_vision_override: Option<bool>`，注释说明：per-model override，覆盖 `capabilities()` 派生的 vision。
- `Provider::new` 与 `Provider::from_record`：`model_vision_override: None`。
- `Provider::capabilities()`：在派生 `ProviderCapabilities` 之后，若 `model_vision_override.is_some()`，把 `caps.vision` 改成 `model_vision_override.unwrap()`。
- 不动 `ProviderCapabilities::anthropic/deepseek/openai_compatible/conservative` 工厂。`Provider::new` 路径已用 `resolve_model_identity` 派生 spec，但 vision_override 不在那里实现，因为 provider 级 override 是单独通道——这点与 `ModelSpec::vision_override` 命名重合，注释里明示这是 `Provider` 级别入口，区别于 catalog 级别。

### 2. `crates/shell/src/config.rs`

- `ProviderJson` 加 `#[serde(default)] models: Option<Vec<ModelJson>>`，其中 `ModelJson` 含 `model_id: String`、`vision: Option<bool>`。
- `build` 闭包构造 `Provider` 后，按 `(p.model_id or p.model)` 在 `models[]` 里找到匹配条目，把 `Some(vision)` 写进 `provider.model_vision_override`；没找到或 `vision` 为 None 时不动 `model_vision_override`。
- 测试：在 `tests` 模块里加 `model_vision_override_round_trips_through_settings_log`，覆盖 (a) `models[]` 里有 vision=true 命中 → override Some(true)；(b) `models[]` 里有 vision=false 命中 → override Some(false)；(c) `models[]` 缺失 → override 仍为 None。

### 3. `crates/agent/src/lib.rs`

- 路径兜底 observation 文案（line 3223-3243 区域）拆成两条：
  - `vision_ok == false && attachment_mime_from_ext` 返回 image MIME：observation 说「image 附件，路径、字节数、当前模型不支持视觉输入，回答请基于这些信息」；**不**使用「已附带附件：内容未内联（二进制/Office 格式）」措辞（避免把 image 伪装成 office）。
  - 其他非内联原因（office/pdf/oversize）：保留既有措辞。
- 多模态内联成功（vision_ok=true 且 inline 成功）路径不动。
- 文案保持中文，与既有 T 函数/i18n.ts 风格一致（这条是 backend observation 文本不进 i18n.ts，沿用现状的「已附带……」风格）。

### 4. 接口契约

- `Provider::capabilities()` 现在是 backward-compatible：`model_vision_override == None` 时与改动前完全等价。
- `ProviderJson` 新字段 `#[serde(default)]`，旧 settings.log 不带 `models` 字段也不报错。
- `RunRequest.provider: Option<Provider>` 字段无变化；per-model vision 经由 Provider 自动到达 backend。

## 失败处理

- `Provider::capabilities()` 对 `model_vision_override == None` 走 template 默认（行为不变）。
- `ShellConfig::from_sources` 中 `models[]` 反序列化失败 → serde 默认跳过该字段 → `model_vision_override` 仍 None。
- image MIME 探测沿用既有 `attachment_mime_from_ext`，不动。

## 验证方式

### 单元测试

- `Provider::capabilities()` 加 3 个 test：
  - `model_vision_override_overrides_template_default_when_some`
  - `capabilities_unchanged_when_model_vision_override_is_none`
  - `model_vision_override_false_disables_vision_for_anthropic_template`
- shell config 加 `model_vision_override_round_trips_through_settings_log` 4 个子断言（命中 / 未命中 / 缺失 / vision=None）。
- image fallback 加：`image_attachment_emits_explicit_no_vision_observation_when_vision_ok_false` 和 `office_attachment_observation_unchanged`。

### 端到端验证（手动）

- `apps/desktop` 跑起来，`/ui-demo/conversation` 设置 dark mode，构造「自定义」provider + `mimo-v2.6-flash`（标 `vision: true`），跑一次带 PNG 的回合，看回复是否引用了像素内容；不足的跑同 provider 不标 vision 的回合，看回复是否承认限制而非编造内容。

## 影响面与回退

- 影响面：仅 `Provider::capabilities()` 路径；`ProviderConfigRecord` 不变（CLI 仍可用）；settings.log schema 向后兼容（新增可选 `models` 字段）。
- 回退：`git revert` 即可——所有改动均是新字段或新分支，不删修改既有行为。
- 不动视觉/闪存通道警告、provider 编辑器 UI、Composer 提示、MCP/CLI。