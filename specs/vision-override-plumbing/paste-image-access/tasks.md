# Tasks — 图片附件可分析性：让粘贴/拖入的图永远可被模型读到

依赖顺序自上而下。

## T001 — `RunRequest.pinned_force_inline` 字段

- 目标文件：`crates/agent/src/lib.rs`
- 内容：`pub pinned_force_inline: Vec<String>`，文档说明用途（用户强意图的附件路径集合，绕过 vision_ok 闸门）。
- 验收：编译过；既有 `RunRequest { ... }` 字面量加 `pinned_force_inline: Vec::new()` / `vec![]`，typecheck 过。
- 依赖：无。

## T002 — image-inlining 分支接受 force_inline

- 目标文件：`crates/agent/src/lib.rs`
- 内容：图像内联分支 `vision_ok` 闸门改为 `vision_ok || force_inline`；失败/oversize 仍走 path-only observation；新增测试 `pinned_force_inline_overrides_vision_gate_when_image_paste_attached`。
- 验收：`cargo test -p kodo-agent --lib tests::image_attachment_*` 全过；新增测试通过。
- 依赖：T001。

## T003 — settings reconciliation（undefined → true）

- 目标文件：`crates/shell/src/config.rs`
- 内容：`build` 闭包在写 `model_vision_override` 之前，把「命中但 `vision == None`」的模型条目视为 `Some(true)`。新增单测 `settings_reconciliation_upgrades_undefined_vision_to_true_for_custom_template`（含命中 + 未命中 + 显式 false + 显式 true 4 个子断言）。
- 验收：`cargo test -p kodo-shell --lib` 全过；既有 5 个 round-trip 子断言不动。
- 依赖：无。

## T004 — Tauri `save_pasted_blob` 加 `save_dir`

- 目标文件：`apps/desktop/src-tauri/src/main.rs`
- 内容：`fn save_pasted_blob(bytes, suggested_filename, save_dir: Option<String>) -> Result<String>`；Some 时 `create_dir_all(save_dir)` 后写盘，None 时走 OS temp。注册到 builder。
- 验收：`cargo build --workspace` 通过；Tauri 命令签名不变（仅多一个可选参数）。
- 依赖：无。

## T005 — `send_message` 透传 `force_inline`

- 目标文件：`apps/desktop/src-tauri/src/main.rs`、`apps/desktop/src-tauri/src/run.rs`
- 内容：`send_message` 命令加 `force_inline: Option<Vec<String>>` 参数；`RunArgs` 加 `force_inline: Vec<String>`；`agent::RunRequest { pinned_force_inline: args.force_inline, ... }`。
- 验收：`cargo build --workspace` 通过；既有调用点（默认 `Vec::new()`）不过。
- 依赖：T001。

## T006 — frontend API 暴露

- 目标文件：`apps/desktop/src/api.ts`
- 内容：`savePastedBlob(bytes, suggestedFilename, saveDir: string | null)`；`sendMessage({ ..., forceInline?: string[] })`。
- 验收：`npm run typecheck` 通过。
- 依赖：T004, T005。

## T007 — Composer paste handler 改造

- 目标文件：`apps/desktop/src/conversation/Composer.tsx`
- 内容：
  - `pastedPathsRef: useRef<Set<string>>(new Set())` 跟踪 paste / drop-no-path 路径
  - `savePastedBlob` 调用传 saveDir（项目内 `<project>/.kodo/.pasted/` 优先，否则 undefined → OS temp）
  - `onComposerPaste` / `onComposerDrop` 在 addExternalPaths 之后把 saved path 加 ref
  - `onSend` 时把 ref 收集出来作为 `forceInline` 传出去；ref 清空
- 验收：`npm run typecheck` 通过；编译过。
- 依赖：T006。

## T008 — `ConversationPage` dispatchSend

- 目标文件：`apps/desktop/src/pages/ConversationPage.tsx`
- 内容：`dispatchSend(text, context, forceInline = [])`；透传到 `sendMessage({ ..., forceInline })`。
- 验收：`npm run typecheck` 通过。
- 依赖：T006, T007。

## T009 — `SettingsPage.addModel` 默认 vision=true

- 目标文件：`apps/desktop/src/pages/SettingsPage.tsx`
- 内容：`addModel` 内部根据当前 template 设置 `vision` 默认：
  - `openai` / `openai_compatible` / `custom` → `vision: true`
  - `deepseek` / `text_only` / `conservative` → `vision: false`
  - 其它 → `undefined`
- 验收：`npm run typecheck` 通过。
- 依赖：无。

## T010 — Analyze 复审 + Converge

- 核对 spec / plan / tasks 的覆盖一致性。
- 跑 `cargo build --workspace` / `cargo test -p kodo-agent -p kodo-shell --lib` / `npm run typecheck`。
- 视觉验收 Playwright 截屏（如有条件）。
- 验收：所有 build / test 通过；spec 验收项 AC-001 至 AC-008 证据齐全。
- 依赖：T001–T009。