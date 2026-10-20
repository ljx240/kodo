# Spec — 图片附件可分析性：让粘贴/拖入的图永远可被模型读到

承接：vision-override-plumbing/{spec,plan,tasks,follow-up}.md

## 用户最新反馈

Paste 后模型仍然看不到像素。即便 plumbing 已通，模型还是只用 path-only 描述 + `read_file`/`cp` 试图救场，全部失败：图片在 OS temp dir，read_file 不接受绝对路径外的内容；cp 被 snapshot limit 拦截。

根因（**当前架构的四条短板**）：
1. **`vision_ok` 闸门耦合了用户意图和模型能力**：用户主动 paste/drag 的图，强行走 vision_ok 检查，对 vision=false 的"已模型"（其实是用户配置错误）就发不出字节。
2. **粘贴图保存位置 OS temp dir**：`/var/folders/.../kodo-pasted/` 不在项目里，agent 的 `read_file` 工具拒绝；模型要自救只能 `cp`，又被 snapshot limit 拦。
3. **Provider 配置 reconciliation 缺失**：用户 settings.log 里 `models[].vision: false` 是更早版本（前 UI 假象）写下的；现在 custom/openai_compatible 默认 vision=true 了，但旧的 persisted false 没被升级。
4. **Provider 编辑器新增模型默认 vision 字段为 undefined**：下次用户再添加自定义模型还会重蹈覆辙。

## 范围

### In-scope

- backend `RunRequest` 加 `pinned_force_inline: Vec<String>` 字段
- backend 图像内联分支：`force_inline` 命中的路径绕开 vision_ok 检查
- backend settings reconciliation：shell config loader 加载时，把 custom/openai_compatible 模板下、`models[]` 里 `vision` 字段缺失的模型条目，自动升级为 `Some(true)`；用户显式 false 不动
- Tauri `save_pasted_blob` 加 `save_dir` 参数：项目内 `.kodo/.pasted/`，项目外 fallback OS temp
- frontend `api.sendMessage` 加 `forceInline` 参数；`savePastedBlob` 加 saveDir 参数
- frontend `Composer.tsx`：paste 路径 = project-aware；跟踪"pasted"路径集合
- frontend `ConversationPage.tsx`：dispatchSend 把 pasted 路径放入 forceInline
- `SettingsPage.addModel`：custom/openai_compatible 模板下新加的模型默认 vision=true
- 全部更新 unit tests 与既有 fixture

### Out-of-scope

- 不动 vision_override 字段、不动 image-inlining 4 MiB cap
- 不动 mcp / skill / persona / run-state 等无关模块
- 不改 CLI（CLI 路径不走前端 paste 流程）
- 不改 console 默认（默认 `openai_compatible` 已经有 vision=true）

## 验收场景

### AC-001：pasted image 永远被模型读到（无论 vision_ok）

**前提**：项目无设置，current task，用户粘贴一张 PNG，模型在 saved settings 里 `vision: false`。

**步骤**：发起一次「为我分析图片」+ 粘贴 PNG。

**期望**：
1. 文件保存到 `<temp_dir>/kodo-pasted/pasted-<ms>-<safe_name>`（项目无，fallback temp）
2. backend 收到 `pinned_force_inline=[pasted_path]`，内联分支按 force_inline=true 处理 → 走 `ContentBlock::Image`
3. 模型看到字节，能描述画面元素
4. 模型不弹"我已经看到图片但无法分析"或尝试 read_file/cp

### AC-002：项目内 paste 也走 inline

**前提**：有项目，current task。

**步骤**：粘贴 PNG。

**期望**：
1. 文件保存到 `<project>/.kodo/.pasted/pasted-<ms>-<safe_name>`（Tauri 自动 mkdir）
2. 同样进 inline 多模态分支
3. 临时粘贴的清理：每次保存时覆盖/添加时间戳前缀，避免污染项目

### AC-003：drag-drop 文件不走 force_inline

**前提**：从 Finder 拖 PNG 进 composer（带 `.path`）。

**步骤**：发起回合。

**期望**：
1. 不进 `pinned_force_inline`（因为它不是 paste 显式触发的，是 OS 文件拖入）
2. 走原有 vision_ok 闸门
3. 与之前行为一致

### AC-004：拖入/拖动但不是 paste 的图片附件**

**前提**：Finder 文件路径有 `.png`，无 `.path`（如浏览器壳复制图片）。

**步骤**：发起回合。

**期望**：视为 paste → 走 inline；如实在不可读（比如文件不存在），fallback 到 path-only 描述。

### AC-005：settings reconciliation 自动生效

**前提**：settings.log 里 custom 模板 provider 的 `models[].vision = false`（用户显式）。

**期望**：**不动**——用户的显式选择被尊重。需要「为已存在的 vision=false 模型自动升级」的逻辑放在用户主动重写该模型时（model editor 提示）。

**边界补充**：对于**没有 vision 字段**的旧模型条目（`vision: undefined`），shell config loader 应升级为 `Some(true)`。这覆盖了 2026-10-19 之前的旧 settings.log。

### AC-007：SettingsPage 添加自定义模型默认 vision=true

**前提**：用户打开 provider 配置，选 custom 模板，添加 `mimo-v2.6-flash`。

**期望**：新条目 `vision: true`（不是 undefined）。用户可手动改回 false。

### AC-008：projectless paste 的 cleanup 不破坏项目

**前提**：项目无设置，连续 paste 10 张图。

**期望**：
1. 全部落到 OS temp dir 下 `kodo-pasted/`
2. 不污染项目（项目路径不变）
3. 文件名带时间戳避免冲突
4. 临时目录由 OS 自动清理（macOS 重启 / 用户手动清）

## 数据契约

### `RunRequest` 新字段

```rust
pub pinned_force_inline: Vec<String>,
```

为「用户意图强」的附件路径集合（粘贴 / 拖入但无 path / 显式拖入图像），绕过 vision_ok 闸门直接 inline。

### Tauri `save_pasted_blob` 新参数

```rust
fn save_pasted_blob(
    bytes: Vec<u8>,
    suggested_filename: Option<String>,
    save_dir: Option<String>,  // 项目内路径或 None（OS temp）
) -> Result<String, String>;
```

返回值：保存后的绝对路径。

### `send_message` 新参数

```typescript
sendMessage({
  ...
  forceInline?: string[]  // 需要 inline 的路径
})
```

## 失败处理

- `save_pasted_blob` 写盘失败：返回错误，composer 弹 onContextError
- project 路径无效/无写权限：fallback 到 OS temp dir
- 文件 > 16 MiB：返回错误
- backend `pinned_force_inline` 含不存在的路径：原 inline 分支 `std::fs::read(path).ok()` 已处理（None → 走 path-only），行为不变

## 验证方式

### 单元测试
- backend：force_inline 命中 → 走 inline；未命中 → 仍走 vision_ok 闸门
- shell config loader：vision=undefined 升级为 Some(true)；vision=Some(false) 不变
- shell config loader：vision=Some(true) 不变

### 集成 / 端到端
- desktop 启动 → SettingsPage 添加 mimo 模型 → vision=true 默认
- desktop 启动 → project 无 → paste PNG → 跑通 inline 分支
- desktop 启动 → project 有 → paste PNG → 落到 `<project>/.kodo/.pasted/`
- 视觉验收：Playwright 截屏验证 composer 粘贴/拖放流程