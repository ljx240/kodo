# Follow-up — Custom provider 默认 vision + 粘贴支持

日期：2026-10-19
承接：vision-override-plumbing/spec.md + 同目录 plan/tasks/acceptance.md

## 用户后续反馈

1. 即便 plumbing 已通，用户配的「自定义 (OpenAI 兼容)」+ MiniMax-M3.1-Flash 模型，Composer 仍弹「当前模型不支持图像」首屏警告。
2. 用户希望 Composer 支持直接复制图片、粘贴图片、粘贴附件到输入框——目前粘贴剪贴板里的图片会被静默丢弃。

## 修复内容

### A. Custom provider 默认 vision=true（前后端一致）

| 层 | 文件 | 改动 |
|---|---|---|
| backend | `crates/agent/src/provider.rs` | `Provider::capabilities()` 把 `custom`/`openai_compatible` 提升到 `ProviderCapabilities::openai_compatible()` 工厂；`provider_template_vision()` 同步返回 true |
| frontend | `apps/desktop/src/pages/ConversationPage.tsx` | `activeModelVision` 在 `match.vision` 为 undefined 时按模板名 fallback：`openai`/`openai_compatible`/`custom` 返回 true；其它保持 undefined |

测试同步：
- backend：3 个原测试更新语义（`custom_provider_capabilities_are_conservative_by_default` 改名为 `custom_openai_compatible_provider_inherits_openai_compatible_capabilities`；`model_vision_override_*` 改成新期望；`provider_template_vision_anthropic_and_openai_on_deepseek_off` 增加 `custom` 断言）
- shell：`model_vision_override_round_trips_through_settings_log` legacy-record 子断言改成 `assert!(provider.capabilities().vision)`（legacy 记录现在也享受新默认）

### B. 粘贴图片 / 附件支持

| 层 | 文件 | 改动 |
|---|---|---|
| shell | `apps/desktop/src-tauri/src/main.rs` | 新增 Tauri 命令 `save_pasted_blob(bytes, suggested_filename)`：写 bytes 到 `temp_dir/kodo-pasted/pasted-<ms>-<safe_name>`，返回绝对路径。16 MiB cap，文件名 sanitize，ms 防同秒冲突 |
| frontend | `apps/desktop/src/api.ts` | 导出 `savePastedBlob(bytes, suggestedFilename)` |
| frontend | `apps/desktop/src/conversation/Composer.tsx` | 扩展 `onComposerPaste`：<br>1. `clipboardData.files` 有 `.path`（Tauri Finder 拖放 / 文件对话框）→ 走原有 `addExternalPaths` <br>2. `clipboardData.files` 无 `.path`（截图粘贴 / 浏览器 shell 文件复制）→ 走 `savePastedBlob` 落临时文件，再 attach <br>3. `clipboardData.items` 含 `kind=file` + `image/*`（部分浏览器/旧 WebKit 路径）→ 同上，按 MIME 推扩展名 <br>4. text 看起来像绝对路径 → 原有逻辑 <br>每个分支独立 try/catch，单项失败不阻塞整批 |

## 验收场景

### AC-A1 — Custom openai_compatible provider 默认 vision=true

**前提**：Mimo / GLM / Qwen 等 OpenAI 兼容 provider，模板 `custom`（或 `openai_compatible`），活跃模型 `mimo-v2.6-flash`，用户在 provider 编辑器没显式改过 vision 字段。

**期望**：
1. 后端 `Provider::capabilities().vision == true`（不再 conservative=false）
2. 前端 `activeModelVision === true`（按模板 fallback）
3. Composer 不再弹「当前模型不支持图像」首屏警告
4. 图片附件走多模态内联分支（与 vision-override-plumbing/acceptance.md 的 AC-001 一致）

**证据**：`cargo test -p kodo-agent -p kodo-shell --lib` 全过；更新后的 `custom_openai_compatible_provider_inherits_openai_compatible_capabilities`、`provider_template_vision_anthropic_and_openai_on_deepseek_off`、`model_vision_override_round_trips_through_settings_log` legacy-record 子断言全过。

### AC-A2 — 已知 text-only 仍可被显式关掉

**前提**：deepseek 模板，活跃 `deepseek-reasoner`。

**期望**：`vision == false`，行为与之前一致。`provider_template_vision("deepseek")` 仍返回 false；既有 `binary_attachment_produces_a_descriptor_observation` 通过。

### AC-A3 — 用户在 custom provider 上显式标 vision=false 仍生效

**前提**：custom provider，活跃模型，用户手动 `vision: false`。

**期望**：per-model 覆盖胜出模板默认，`Provider.capabilities().vision == false`。`model_vision_override_overrides_template_default_when_some` 通过。

### AC-B1 — 粘贴截图

**步骤**：在 Tauri 桌面里截屏（macOS `Cmd+Shift+4` → 复制到剪贴板），切到 Composer，焦点在输入框，`Cmd+V`。

**期望**：截图出现在 attachment card 上（缩略图 + IMG 标签 + 文件名 + `×` 移除键），后续 send 走多模态分支（前提是模型视觉 OK）。

### AC-B2 — 粘贴纯文本路径

**步骤**：复制 `/Users/.../Desktop/report.docx`，粘贴到 Composer 输入框。

**期望**：附件进入 attachment 列表，原有行为不变。

### AC-B3 — Finder 拖放保留

**步骤**：从 Finder 拖文件到 Composer（保留 Tauri 暴露 `.path`）。

**期望**：与之前一致，不经过 `save_pasted_blob`。

### AC-B4 — 超大粘贴

**粘贴 50 MiB 截图**：返回错误 `pasted blob too large: ...`，onContextError 弹一次，其它粘贴不受影响。

## 全量回归

- `cargo test -p kodo-agent --lib` — 551 passed, 0 failed
- `cargo test -p kodo-shell --lib` — 67 passed, 0 failed
- `cargo build --workspace` — Finished
- `apps/desktop` `npm run typecheck` — Finished, no errors

## 端到端限制

AC-B1（实际粘贴截图）的 e2e 验证需 Tauri 运行时 + 截图工具；单元测试 + 类型检查只覆盖 plumbing 与编译正确性。仓库 fixture 不模拟剪贴板事件。