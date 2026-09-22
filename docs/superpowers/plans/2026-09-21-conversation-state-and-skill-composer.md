# Kodo Conversation State and Skill Composer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Kodo 的空会话、运行中断/停止会话、trace 和技能输入使用一致且可解释的 UI 状态，同时保持现有 Rust/Tauri 协议不变。

**Architecture:** 在现有 `conversation/trace.ts` 增加一个纯函数生命周期映射，并让消息回复、trace 行和 Inspector 消费其结果。Composer 将技能从 textarea 普通字符串提升为局部 chip state，提交时仍序列化成现有标签文本；所有改动保持在 React/CSS/Playwright 层。

**Tech Stack:** React 19, TypeScript 7, Vite, lucide-react, CSS variables, Playwright.

**Spec:** `docs/superpowers/specs/2026-09-21-conversation-state-and-skill-composer.md`

## Global Constraints

- 保持 Kodo 现有三栏 shell、视觉 token、侧边栏和 Inspector 宽度不变。
- 不修改 Rust session 日志、Tauri command、agent 协议或 provider 行为。
- 不添加 npm 依赖，不读取或改动 secrets、credentials、`.env*` 或生产配置。
- 不覆盖工作树已有未提交改动；只在本计划列出的文件中追加本需求改动。
- 不修改 Git 历史；本轮只保留工作树改动，提交由用户后续决定。
- Primary visual viewport remains `1586 × 992`; also smoke-test `1440 × 900` and `1920 × 1080` where the existing Playwright setup supports it.

### Task 1: Define and test the shared reply lifecycle

**Files:**
- Modify: `apps/desktop/src/conversation/trace.ts`
- Modify: `apps/desktop/src/data/types.ts`
- Test: `apps/desktop/tests/visual/behaviour.spec.ts`

**Interfaces:**
- Produces `ReplyStatus = "empty" | "working" | "completed" | "stopped" | "interrupted" | "failed"` from `conversation/trace.ts`.
- `Reply` exposes `status` and `error` alongside its existing trace/final/change data.
- `TraceStep.status` accepts terminal display statuses `"interrupted"` and `"stopped"` in addition to backend `"done" | "running" | "failed"`.

- [x] **Step 1: Extend the test fixture for an interrupted turn and assert terminal status.**

  In the existing killed-run test, keep the persisted item as `status: "running"`, `done: false`, `stopped: false`, then assert the rendered row uses `.trace-mark--interrupted`, the reply shows the existing interruption message, and no `.trace-mark--running` remains. Add a stopped-turn case with `stopped: true` and assert `.trace-mark--stopped`, the stop message, and no interruption message.

- [x] **Step 2: Run the focused behavior tests and verify the new assertions fail.**

  Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/behaviour.spec.ts -g "killed run|stopped run"
  ```

  Expected: the current killed-run assertion fails because the orphaned item is still rendered as `trace-mark--running`; the new stopped-turn assertion is not yet satisfied.

- [x] **Step 3: Implement the pure lifecycle mapping.**

  In `trace.ts`, add `replyStatus(turn, running)` using this exact order: `running`, `error`, `done`, `stopped`, orphaned running item, otherwise `empty`. Build steps with `toStep`, then map only `status === "running"` steps to the matching `"interrupted"`, `"stopped"`, or `"failed"` terminal display status. Preserve `turn.error` as `Reply.error`. Set `formatDuration(null)` to return `"—"`.

- [x] **Step 4: Run the focused behavior tests and verify the pure state path is ready for UI consumers.**

  Run:

  ```bash
  npm --prefix apps/desktop run typecheck
  ```

  Expected: TypeScript reports only the expected consumer errors for `Reply` construction/reads; resolve those consumers in Tasks 2 and 3 before broad verification.

### Task 2: Render terminal trace states and prevent row overflow

**Files:**
- Modify: `apps/desktop/src/conversation/AgentTrace.tsx`
- Modify: `apps/desktop/src/conversation/AssistantReply.tsx`
- Modify: `apps/desktop/src/styles/conversation.css`
- Modify: `apps/desktop/src/styles/app.css`
- Test: `apps/desktop/tests/visual/behaviour.spec.ts`

**Interfaces:**
- `AgentTrace` renders the expanded row state from `TraceStep.status`; only `running` remains animated.
- `AssistantReply` renders `Reply.status` without reimplementing lifecycle inference.
- CSS adds terminal mark/status variants and flex shrink boundaries without changing the shell grid.

- [x] **Step 1: Add behavior assertions for terminal row accessibility and layout-safe details.**

  Extend the killed/stopped tests to assert the terminal trace row has `aria-expanded="false"` when it has no output. Add an assertion that the stopped/interrupted reply does not contain `.reply-working`; keep the existing completed-run assertions for a green mark, duration, and output.

- [x] **Step 2: Run the focused behavior tests to capture the current rendering failure.**

  Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/behaviour.spec.ts -g "run draws|killed run|stopped run"
  ```

  Expected: terminal rows still use the running mark and spinner, and the new terminal accessibility assertions fail.

- [x] **Step 3: Render terminal marks and lifecycle messages.**

  In `AgentTrace.tsx`, handle `interrupted` and `stopped` before the running fallback with a non-animated minus/square mark. Compute default expansion as `Boolean(step.output) || step.status === "running"`; terminal rows with no output start collapsed. In `AssistantReply.tsx`, derive messages from `reply.status`: keep the existing interruption copy, add a stopped copy and an error copy, and never render `reply-working` unless status is `working`.

- [x] **Step 4: Add CSS constraints for trace row flex children.**

  Make `.trace-row-inner` and `.trace-line` hide horizontal overflow; make `.trace-detail` the primary shrinking child; constrain `.code-chip` and `.trace-tokens` with ellipsis; allow `.trace-output` to shrink within its max width; keep `.trace-duration` and `.trace-disclosure` `flex: none`. Add non-animated `.trace-mark--interrupted` and `.trace-mark--stopped` colors based on existing warning/secondary tokens, plus `.reply-stopped` and `.reply-failed` text styles.

- [x] **Step 5: Run the focused behavior tests again.**

  Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/behaviour.spec.ts -g "run draws|killed run|stopped run"
  ```

  Expected: PASS; completed and genuinely running rows keep their current behavior while terminal rows no longer show a spinner.

### Task 3: Make empty Inspector state and status presentation consistent

**Files:**
- Modify: `apps/desktop/src/data/liveContext.ts`
- Modify: `apps/desktop/src/inspector/ResponseInspector.tsx`
- Modify: `apps/desktop/src/styles/app.css`
- Modify: `apps/desktop/src/styles/inspector.css`
- Test: `apps/desktop/tests/visual/behaviour.spec.ts`

**Interfaces:**
- `LiveSnapshot.reply.status` is the only live response status source for Inspector content.
- `ResponseOverview` keeps demo content unchanged, but for a live session with no turn renders only empty response state plus current project.
- Status pills expose `completed`, `working`, `stopped`, `interrupted`, and `failed` with matching icon/color treatment.

- [x] **Step 1: Add live empty and status assertions.**

  Add a live empty-session test using `stubShell` with a session whose `turns` is empty. Assert the Inspector contains “尚未开始响应”, does not contain `Total steps`, `.status-pill`, or response-dependent file/tool/LLM sections, and still contains `Current project`. In the killed/stopped tests assert Inspector text contains `Interrupted` and `Stopped` respectively.

- [x] **Step 2: Run the focused Inspector tests and verify they fail.**

  Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/behaviour.spec.ts -g "empty Inspector|killed run|stopped run"
  ```

  Expected: the current empty session renders `Status — / Total steps 0`; killed and stopped sessions do not expose the required terminal status labels.

- [x] **Step 3: Implement live empty branching and shared status pill rendering.**

  Keep `snapshotFromTurn` passing through `toReply`. In `ResponseInspector.tsx`, render an empty response section when `live?.turn` is null and `live?.running` is false; return from `ResponseOverview` before Changed files/Tools/LLM for that live case, then render `CurrentProjectSection`. For a non-empty live response, map `live.reply.status` to the exact labels `Working`, `Completed`, `Stopped`, `Interrupted`, and `Failed`, and render a green check only for `completed`; use warning/error/blue variants for the other states.

- [x] **Step 4: Add status pill CSS and run focused tests.**

  Add class variants in the existing global/Inspector CSS without changing the default completed appearance. Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/behaviour.spec.ts -g "empty Inspector|killed run|stopped run"
  ```

  Expected: PASS with no green success styling for empty, stopped, interrupted, or failed responses.

### Task 4: Replace raw skill text with removable Composer chips

**Files:**
- Modify: `apps/desktop/src/conversation/Composer.tsx`
- Modify: `apps/desktop/src/styles/conversation.css`
- Modify: `apps/desktop/tests/visual/composer-context.spec.ts`

**Interfaces:**
- Composer owns `selectedSkills: string[]` and does not expose a new prop or backend API.
- `applySkill(id)` de-duplicates and focuses the textarea without writing the protocol tag into `draft`.
- `send()` serializes skills as `【技能：<id>】` lines before trimmed draft text, then clears both draft and selected skills.

- [x] **Step 1: Update skill behavior tests for chip semantics and submission payload.**

  Change the existing skill test to assert `.composer-skill` is visible, the textarea value is empty, and the chip exposes a remove button. Add a remove assertion. Extend the send payload test so selecting `bug-fix`, typing `修复这个问题`, and pressing Enter records exactly `【技能：bug-fix】\n修复这个问题` in `__sendLog[0].text`.

- [x] **Step 2: Run the focused Composer tests and verify the old raw-text assertion fails.**

  Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/composer-context.spec.ts -g "skill|send passes structured"
  ```

  Expected: the current test still finds a raw `【技能：bug-fix】 ` value and fails the new chip/empty-text expectation.

- [x] **Step 3: Implement selected skill state and serialization.**

  Add `selectedSkills` state and update `applySkill` to append an id only when absent. Render a `composer-skills` row between the textarea row and context row; each chip shows `技能 · <id>` and a button with `Remove skill <id>`. Add `removeSkill`. In `send`, build `skillText` from selected ids, combine it with draft, reject only when the combined text is empty, clear draft and skills before calling `onSend`, and leave context handling unchanged.

- [x] **Step 4: Add compact chip styling and rerun Composer tests.**

  Add `.composer-skills` and `.composer-skill` styles that match existing `.chip` dimensions, keep the chip row wrapping, use the existing accent/mono tokens, and preserve composer minimum height. Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/composer-context.spec.ts -g "skill|send passes structured"
  ```

  Expected: PASS; selecting a skill leaves the textarea ready for task text and sending preserves the existing backend tag protocol.

### Task 5: Full validation and visual inspection

**Files:**
- Modify only files already listed above if validation exposes a regression.
- Inspect: `docs/design/references/01-conversation-main.png`, `docs/design/references/02-response-trace.png`, `docs/design/references/05-conversation-alt.png`

- [x] **Step 1: Run TypeScript and production-import checks.**

  Run:

  ```bash
  npm --prefix apps/desktop run typecheck
  npm --prefix apps/desktop run test:imports
  npm --prefix apps/desktop run test:contrast
  ```

  Expected: all commands exit 0; live route modules still do not import fixture/demo data.

- [x] **Step 2: Run the complete visual behavior and accessibility suites.**

  Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/behaviour.spec.ts tests/visual/composer-context.spec.ts tests/visual/a11y.spec.ts
  ```

  Expected: all selected tests pass without snapshot updates.

- [x] **Step 3: Capture deterministic screenshots at the required viewport.**

  Run:

  ```bash
  npm --prefix apps/desktop run test:visual -- tests/visual/pages.spec.ts --project=chromium
  ```

  Inspect the generated screenshots at `1586 × 992`, checking the three-column geometry, empty-state vertical balance, terminal trace mark, Inspector empty/status sections, and Composer chip density against the repository references. Do not regenerate baselines merely to hide a mismatch.

- [x] **Step 4: Report the final working-tree diff without committing.**

  Run:

  ```bash
  git status --short
  git diff --stat -- docs/superpowers apps/desktop/src/conversation apps/desktop/src/data/liveContext.ts apps/desktop/src/inspector apps/desktop/src/styles apps/desktop/tests/visual
  ```

  Expected: only the plan/spec plus the listed UI and test files contain this task's changes; unrelated pre-existing changes remain untouched.
