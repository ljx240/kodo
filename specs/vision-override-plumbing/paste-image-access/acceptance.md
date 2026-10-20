# Acceptance — 图片附件可分析性：粘贴/拖入的图永远可被模型读到

承接：`specs/vision-override-plumbing/{spec,plan,tasks,follow-up}.md` + 同目录 `paste-image-access/{spec,plan,tasks}.md`

## 验收场景

### AC-001 — pasted image 永远被模型读到（无论 vision_ok）

**证据**：单测 `image_attachment_force_inline_bypasses_vision_gate` 通过 + 端到端 composer 流程 + `RunRequest.pinned_force_inline` 字段就位。

- backend `crates/agent/src/lib.rs` 图像内联分支：`vision_ok || pinned_force_inline.contains(rel)` —— pasted 路径绕过闸门直接 inline。✓
- 新增单测 `tests::image_attachment_force_inline_bypasses_vision_gate` 锁定 wire shape（vision_incapable 走 deepseek preset，构造 ContentBlock::Image 验证字段一致）。✓
- composer `onComposerPaste` 把 paste 保存的路径加入 `pastedPathsRef`；`send` 时通过 `onSendWithForceInline` 回调透传到 `RunRequest.pinned_force_inline`。✓

### AC-002 — 项目内 paste 也走 inline

**证据**：Tauri `save_pasted_blob` 接受 `save_dir: Option<String>`；composer 计算 `pastedSaveDir = projectPath ? <project>/.pasted : null`，传给 `savePastedBlob`。

- 项目内 paste → 文件落到 `<project>/.pasted/pasted-<ms>-<safe_name>`，Tauri `create_dir_all(save_dir)` 自动建目录。✓
- 同时 `pinned_force_inline` 把字节 base64 内联给模型，agent 无需走 `read_file`。✓

### AC-003 — Finder 拖放保留原行为

**证据**：composer `onComposerPaste` 第一支检测 `clipboardData.files[i].path` 命中（Tauri Finder 拖放 / 文件对话框），立即走 `addExternalPaths`，**不**入 `pastedPathsRef`。✓

- 这些路径走原有的 vision_ok 闸门（与改动前一致）。
- 没经过 `save_pasted_blob`，不创建临时文件。

### AC-004 — 无 `.path` 的拖入图片

**证据**：composer `onComposerDrop` 与 `onComposerPaste` 第二支对称处理；image clipboard item 或 image transfer item → `savePastedBlobToProject(bytes, name)`，**且**入 `pastedPathsRef`。✓

### AC-005 — settings reconciliation 自动升级 undefined vision

**证据**：shell config loader 的 `build` 闭包对命中但 `vision == None` 的条目，在 `openai` / `openai_compatible` / `custom` 模板下升级为 `Some(true)`。

- `model_vision_override_round_trips_through_settings_log` 单测更新：vision 缺失 + custom 模板 → `Some(true)`；vision 缺失 + 未知模板 → `None`（保持保守）。✓
- 用户显式 `vision: false` 不动（保留意图）。✓
- 用户显式 `vision: true` 不动。✓

### AC-007 — SettingsPage.addModel 默认 vision=true

**证据**：`apps/desktop/src/pages/SettingsPage.tsx` `addModel` 按 template 选 default vision：

| template | 默认 vision |
|---|---|
| `openai` / `openai_compatible` / `custom` | `true` |
| `deepseek` / `text_only` / `conservative` | `false` |
| 其它 | `undefined`（opt-in） |

`npm run typecheck` 通过。✓

### AC-008 — projectless paste cleanup

**证据**：`pastedSaveDir = projectPath ? ... : null`；Tauri `save_pasted_blob` 在 `save_dir.is_empty()` 或 `create_dir_all` 失败时 fallback OS temp（当前行为）。✓

- 无项目 paste → 落到 OS temp `kodo-pasted/`，不污染任何项目。
- 时间戳前缀 (`pasted-<ms>-<safe_name>`) 防同秒冲突。

## 全量回归

- `cargo build --workspace` — Finished。✓
- `cargo test -p kodo-agent --lib` — 552 passed, 0 failed（551 既有 + 1 force-inline bypass）。
- `cargo test -p kodo-shell --lib` — 67 passed, 0 failed（既有 + 1 vision reconciliation 增强）。
- `apps/desktop` `npm run typecheck` — Finished, no errors。✓

## 端到端限制

AC-001 / AC-002 的真实粘贴 PNG → 模型回复需要：
- 在桌面 Settings 里把自定义 provider 的 mimo 模型标 `vision: true`（或留空让 reconciliation 自动升级）
- 粘贴一张图 → 文件落到 `<project>/.pasted/`（项目有）或 OS temp（项目无）
- `RunRequest.pinned_force_inline` 把路径递给 backend → 走 ContentBlock::Image 内联
- 模型回复里能描述画面元素

仓库 fixture 不模拟剪贴板事件；e2e 需用户在 Tauri 桌面运行后实测。