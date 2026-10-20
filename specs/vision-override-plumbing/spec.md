# Spec — 图片附件多模态支持：per-model vision 透传 + 路径兜底明示

## 用户目标

Kodo 用户为自己的「自定义 (OpenAI 兼容)」provider 配置一个视觉模型（如 `mimo-v2.6-flash`），并在前端 provider 编辑器里将该模型条目标记为支持图片（`vision = true`）。当前的运行链路里，这个标记不会传到 backend——`Provider::capabilities()` 只看 template 字符串，导致 backend 永远判定 vision=false，图片附件走 path-only 描述（"已附带附件：内容未内联"），模型只能看到路径和字节大小，无法分析像素。

附加目标：当模型实际不能看图时，路径兜底的 observation 文案要明确说明这是模型选择问题（不是文件本身的问题），避免模型自行发挥出截图里那种自我安慰式话术。

## 范围

### In-scope

- backend (`crates/agent`)：`Provider` 新增 per-model vision override 字段；`Provider::capabilities()` 在 template 默认之上应用该 override。
- shell (`crates/shell`)：`ShellConfig::load` 从 `settings.log` 中读出当前活跃 provider 配置中的 `models[].vision`，并写入 `Provider.model_vision_override`。
- backend (`crates/agent/src/lib.rs`)：当 image 附件路径存在但 `vision_ok == false` 时，observation 文案明示「已识别为 image，当前模型不支持视觉输入」并指出 bytes 数，禁止出现"已附带附件：内容未内联（二进制/Office 格式）"这种把图片伪装成 office 的兜底措辞。
- backend：保留现有 `vision_ok` 判断与 `<4 MiB` 内联门禁逻辑。

### Out-of-scope

- 前端 provider 编辑器 UI、Composer 提示、视觉/闪存通道警告等不动。
- CLI 端 `ProviderJson` 的 `vision_override` 模型规范（已存在）不重做，只透传。
- 不动 MCP / skill / persona 等无关模块。
- 不动 provider-template-vision 的 fallback 矩阵（anthropic/openai/deepseek 仍按既定 vision 默认）。

## 验收场景

### AC-001 (per-model vision 透传生效)

**前提**：自定义 (OpenAI 兼容) provider，活跃模型 `mimo-v2.6-flash`，用户在编辑器里把它标为 `vision: true`（保存在 `settings.log` 的 `models[]` 数组里）。

**步骤**：发起一次带 PNG 附件的对话回合（消息 = 「为我分析下附件图片」，附件 = `微信图片_2026-10-19_101221_927.png`，`<= 4 MiB`）。

**期望**：
1. backend `vision_ok` 计算结果为 `true`（不再是 template 派生的 `conservative().vision = false`）。
2. 图片走 `ContentBlock::Image` 多模态分支，模型实际收到 base64 图像数据。
3. 模型回复里能引用画面元素（截图、布局、报错文字之一），而不是路径/字节数。

### AC-002 (自定义模型未标 vision 时兜底)

**前提**：同一 provider，同一模型 `mimo-v2.6-flash`，但用户没标 `vision`（保存在 `settings.log` 的 `models[]` 里 `vision` 字段缺失或为 `false`）。

**步骤**：同 AC-001 的发送。

**期望**：
1. `vision_ok` 为 `false`。
2. observation 文案显式说明：附件是 image、字节数、当前模型不支持视觉输入、回答时只看到路径/大小/类型，不要伪造图像内容。
3. 文案不再使用"已附带附件：内容未内联（二进制/Office 格式）"这种把图片描述为 office 的措辞。
4. 模型回复要么基于文件名/大小讨论（比如提示用户上传截图文本），要么承认限制，不会自己编一段"我已经看到图片但是无法分析"的话术。

### AC-003 (硬编码目录行为不变)

**前提**：anthropic 模板，活跃模型 `claude-3-5-sonnet-latest`（catalog 默认 `vision: true`）。

**步骤**：带 PNG 附件发起回合。

**期望**：`vision_ok` 仍为 `true`，行为完全不变（按既有测试通过）。

### AC-004 (硬编码目录 vision=false 行为不变)

**前提**：deepseek 模板，活跃模型 `deepseek-reasoner`（catalog 显式 `vision: false`）。

**步骤**：带 PNG 附件发起回合。

**期望**：`vision_ok` 仍为 `false`，命中路径兜底 observation；observation 文案按 AC-002 的明示格式呈现。

### AC-005 (settings.log 无 models 字段的旧记录)

**前提**：`settings.log` 中 provider 条目没有 `models` 字段（旧记录迁移后）。

**步骤**：发起任意回合。

**期望**：`model_vision_override` 为 `None`，`Provider::capabilities()` 退回 template 默认——与改动前行为完全一致。

## 边界与失败行为

- 旧 settings.log 记录没有 `models`：视为未配置 per-model override，按 template 默认 → 不破坏向后兼容。
- `models[]` 里找不到活跃模型条目：override 为 None → 与 AC-005 相同路径。
- `models[].vision` 字段缺失：视为 None → 同上。
- `settings.log` 整体损坏或反序列化失败：`ShellConfig::load` 当前返回 `None`，frontend 走无 provider 路径，不引入新失败模式。
- 文件超过 4 MiB 或不是已识别图像 MIME：保留既有路径（即便 vision_ok=true，也走 path-only）。文案不动。

## 验证方式

- 单元测试：
  - `Provider::capabilities()` 在 `model_vision_override = Some(true/false)` 时正确覆盖 template 默认。
  - image-inlining branch 的文案生成函数在 `vision_ok=false` 且 MIME 为 image 时输出正确格式。
- 集成测试：
  - shell config loader 读 `settings.log` 中 `models[].vision` 并正确写入 `Provider.model_vision_override`（含 None / 缺失 / 命中 / 未命中 4 个分支）。
- 端到端验证（手动 + Playwright demo route）：
  - 在 `apps/desktop` 的 `/ui-demo/conversation` 设置 dark mode 和「自定义」provider + 视觉模型，跑一次带 PNG 的回合，断言回复包含像素相关字眼（不依赖 mimo 真实输出形状，断言曲线足够复杂证明模型看到了字节）。