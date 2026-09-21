# Kodo 会话状态与技能输入体验设计

## 目标

修复 Kodo 初始会话、执行中断会话和技能选择输入框中可见的状态矛盾，并保持现有 Kodo 的三栏布局、密度、颜色 token、后端 session 日志格式和技能标签协议不变。

## 现状与问题

- `apps/desktop/src/conversation/trace.ts` 只把“遗留 running item 且当前没有运行”识别为中断，但没有把该状态传递给 trace 行本身，因此 `AssistantReply` 显示中断文案的同时，最后一步仍显示 running spinner 并默认展开。
- `ResponseInspector.tsx` 对 live turn 只区分 working、stopped、done 和兜底 `—`，异常中断会落入绿色成功胶囊的视觉分支，不能表达 Codex 源码中明确的 terminal `Interrupted` 状态。
- 空的新会话没有 turn，但 Inspector 仍显示 “This response / Status — / Total steps 0” 以及依赖响应的空 section，视觉上像一轮已经开始但没有步骤的执行。
- `Composer.tsx` 把技能直接追加进 textarea 的普通字符串。Codex composer 选择 mention 后会把它视为原子元素；在 Kodo 的 textarea 实现中，等价的最小方案是把 skill 保存在独立 state，显示为可移除 chip，提交时再序列化回现有 `【技能：<id>】` 文本协议。
- trace 的 detail、chip、output、duration 共享一个不允许收缩的横向 flex 行，长描述会挤压 duration 和 disclosure，导致窄窗口出现截断或重叠。

## 设计决策

### 1. 统一响应生命周期

在 `conversation/trace.ts` 定义 `ReplyStatus`：

```text
empty | working | completed | stopped | interrupted | failed
```

`toReply(turn, running)` 按以下优先级计算：

1. `running === true` → `working`
2. `turn.error` → `failed`
3. `turn.done` → `completed`
4. `turn.stopped` → `stopped`
5. 存在 `item.status === "running"` → `interrupted`
6. 其他无可见结果的 turn → `empty`

当生命周期为 `stopped`、`interrupted` 或 `failed` 时，仅把遗留 running step 的显示状态转换为对应的终态；不修改后端 DTO，也不伪造完成时间。trace 行使用非动画的终态标记，默认关闭无输出的遗留步骤。`formatDuration(null)` 显示 `—`，避免未完成步骤看起来像 0ms。

`AssistantReply` 和 live Inspector 复用同一个 `ReplyStatus`，分别显示“已停止”“运行中断”“运行失败”或“Completed”，不会出现文案、spinner 和状态胶囊互相矛盾的组合。

### 2. 空会话 Inspector

当 live session 尚无 turn 且没有运行中的请求时，Inspector 只保留：

- 一个不可误认为成功响应的空态 section，文案为“尚未开始响应，发送消息后会在这里显示执行摘要。”；
- `Current project` section。

Changed files、Tools used 和 LLM calls 等响应依赖 section 在第一轮 turn 出现前不渲染。demo 路由继续使用 fixture 的完整 Inspector，不改变视觉回归基准数据。

### 3. 技能 chip

Composer 使用 `selectedSkills: string[]` 保存选择结果：

- 从技能菜单选择后在 textarea 上方显示紧凑 chip，显示技能 id，附带可访问名称和删除按钮；
- 重复选择同一 skill 不重复添加；
- 删除 chip 不修改普通草稿；
- textarea 保持空白 placeholder，不再把协议标记画成普通正文；
- 提交时按选择顺序把 `【技能：id】` 加到提交文本前，再拼接普通草稿；提交成功触发后清空 chip 和草稿；
- 现有运行时继续收到同一 `text` 字符串，因此不需要改变 Rust agent 或 session 存储。

### 4. Trace 横向布局

保留当前 inline trace 结构，只补充 flex 收缩边界：

- detail 为唯一主要可收缩文本，并使用 ellipsis；
- code chip、token、duration 和 disclosure 保持可见的最小宽度；
- expanded output 允许在限定宽度内收缩，不能把 duration 推出主列；
- 新增 interrupted/stopped 的终态样式，使用现有 warning/secondary token，不引入新颜色体系。

## 非目标

- 不改变 Rust session 日志、Tauri command、agent 协议或 provider 行为。
- 不改造 Kodo 的三栏 shell、侧边栏项目树、Inspector 宽度或其他页面。
- 不引入新的 npm 依赖。
- 不把技能菜单改造成完整 Codex mention 编辑器；本次只覆盖技能选择、显示、移除和提交序列化。

## 验收标准

- 空 live 会话 Inspector 不显示绿色成功状态、不显示 `Total steps 0` 的响应摘要；项目摘要仍可见。
- 重新打开一个遗留 running item 的会话时，消息区显示中断文案、终态标记、无 spinner，Inspector 显示 `Interrupted`。
- 用户主动停止运行时，消息区显示停止文案、终态标记、无 spinner，Inspector 显示 `Stopped`。
- 运行失败时，消息区显示失败原因、终态标记、无 spinner，Inspector 显示 `Failed`。
- 完成中的 step 仍保持现有绿色 check、输出展开和 duration 行为；真实 running step 仍显示 spinner。
- 选择 `bug-fix` 后 textarea 为空、chip 可见且可移除；发送时 payload 仍包含 `【技能：bug-fix】`。
- 长 trace detail 不覆盖 duration/disclosure，且 demo 视觉布局保持在 Kodo 的既有规范内。
- `typecheck`、production import audit、contrast audit 和相关 Playwright 行为测试通过；在 1586×992 下检查初始、完成和中断状态截图。
