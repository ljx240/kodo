# Plan — 图片附件可分析性：让粘贴/拖入的图永远可被模型读到

## 改动面

### 1. `crates/agent/src/lib.rs` — `RunRequest`

新增字段：
```rust
/// Paths the user explicitly attached (paste / drag with no `.path`).
/// Inlined as multimodal blocks even when the active model lacks vision —
/// the user's paste action is the strong intent; vision_ok gates only
/// the auto-attached / inherited attachments.
pub pinned_force_inline: Vec<String>,
```

### 2. `crates/agent/src/lib.rs` — image inlining 分支

把 `vision_ok` 闸门放松成「vision_ok OR force_inline」：

```rust
let force_inline = request.pinned_force_inline.iter().any(|p| p == rel);
let inlined_image = if vision_ok || force_inline {
    attachment_mime_from_ext(rel).and_then(|media_type| {
        ...
    })
} else {
    None
};
```

后置观察：当 `force_inline=true && vision_ok=false` 且内联成功时，记录一条 note「已为用户粘贴强制内联 image X」——可观测性，不是新依赖。失败/oversize 仍然走 path-only。

### 3. `crates/shell/src/config.rs` — settings reconciliation

`build` 闭包里把 reconciliation 放在已有 `model_vision_override` 写入前：
- `models[]` 里命中 `vision` 字段**缺失**（`Option<bool>::None`）的条目 → 写 `Some(true)` 到 `provider.model_vision_override`
- 命中 `vision=true` 或 `vision=false` → 保留用户的显式选择
- 命中但 `model_id` 为空 → 跳过（容错）

测试：
- 新增 `settings_reconciliation_upgrades_undefined_vision_to_true_for_custom_template`
- 既有 `model_vision_override_round_trips_through_settings_log` 5 个子断言不动

### 4. `apps/desktop/src-tauri/src/main.rs` — `save_pasted_blob` 升级

新增 `save_dir: Option<String>`：
- 若 Some 且可写 → `std::fs::create_dir_all(save_dir)`，返回 save_dir 内的 pasted 文件
- 若 None 或不可写 → 走 OS temp（当前行为）
- 文件名 sanitize + 时间戳防冲突
- 保留 16 MiB cap 与 stderr 错误

### 5. `apps/desktop/src-tauri/src/main.rs` — `send_message` 参数

`force_inline: Option<Vec<String>>` 加到 `send_message`：
- 透传到 `run::RunArgs`
- `RunArgs` 加 `force_inline: Vec<String>`
- `RunRequest { pinned_force_inline: force_inline, .. }`

### 6. `apps/desktop/src/api.ts`

```typescript
savePastedBlob(bytes, suggestedFilename, saveDir?)
sendMessage({ ..., forceInline? })
```

### 7. `apps/desktop/src/conversation/Composer.tsx`

新增 `pastedPathsRef: useRef<Set<string>>`（粘的路径）。
新增「pasted context」概念：每次 paste / drop 写入 `pinned_context` 时，把路径加入 ref。
新增清理函数 hook，把 pasted path 在 send / 撤销时移出 ref。

`savePastedBlob` 调用：传入 `saveDir` —— 项目内路径（如果有）or undefined。

`onComposerPaste` / `onComposerDrop`：saved path 既入 `pinned_context` 又入 `pastedPathsRef.current`。

### 8. `apps/desktop/src/pages/ConversationPage.tsx`

`dispatchSend(text, context)` → 增 `forceInline` 第三参数（默认 `[]`）。
把 `forceInline` 传给 `send_message` invoke。
sent 之后清空 pasted 集合（pinned_context 已经清）。

### 9. `apps/desktop/src/pages/SettingsPage.tsx`

`addModel(option)`：根据 template 判断 default vision：
- `template === "openai" | "openai_compatible" | "custom"` → `vision: true`
- `template === "deepseek" | "text_only" | "conservative"` → `vision: false`
- 其它 → `undefined`（保持 opt-in）

让用户在编辑器里再点一下开关覆盖（保留主动权）。

## 验证方式

### 单测
- backend `pinned_force_inline_round_trips_through_lib.rs` — force_inline 命中 + 不命中两支
- shell `settings_reconciliation_*` 4 个子断言
- composer paste handler 的 path-to-save-dir 转换

### 端到端（验收 doc 里列）
- 启动 desktop，project 无，粘贴 PNG，断言 backend 把 inline 多模态块发出
- project 有，粘贴 PNG，断言文件落到 `<project>/.pasted/`
- provider 编辑器，添加 mimo 模型，断言默认 vision=true
- 视觉验收 Playwright 截屏

## 风险与回退

- force_inline 命中但 provider 真不收 image：API 会 400/404。当前 backend 没有重试降级；可以记录 note「image 强制内联但 provider 拒绝：fallback path-only」即可，不影响 send 成功。
- project 内 `.kodo/.pasted/` 占用空间：临时文件，每次都时间戳前缀，OS 不主动清，需要定时清理任务（out-of-scope，本 spec 不做）。
- 已存 settings.log 有 `vision: false` 显式 → 我们的 reconciliation 不动，保留用户意图。**新条目**默认 true 是修旧模型添加流程的唯一入口。

## 不动

- 不动 4 MiB image inline cap
- 不动 vision_ok 闸门（改成 `vision_ok OR force_inline`）
- 不动 image MIME 类型集合
- 不动 i18n / composer 输入框以外的 UI