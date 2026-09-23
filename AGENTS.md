# Kodo Git 开发与合并规则

本文档是 Kodo 项目的 Git、分支和 worktree 操作规范。所有 Code Agent、开发者和自动化任务都必须遵守。

## 1. 仓库主线

- `main` 是唯一集成主线，始终代表可交付代码。
- 不在 `main` 上直接开发、修改业务代码或堆积临时提交。
- `main` 合并前必须通过与改动范围匹配的验证；未验证的代码不得合入。
- 远程 `origin/main` 是团队共享基线。开始新任务前先同步远程信息，并确认本地基线没有落后。
- 不允许使用 `git reset --hard`、`git checkout --`、强制推送等方式覆盖他人或用户改动。

## 2. 分支命名

每个任务只创建一个工作分支，分支名必须能表达任务目的：

```text
feat/<short-name>       新功能
fix/<short-name>        缺陷修复
refactor/<short-name>   重构
test/<short-name>       测试或验证
docs/<short-name>       文档
chore/<short-name>      工程维护
```

规则：

- 分支必须从最新 `origin/main` 创建，不从其他功能分支、旧 worktree 或未确认的本地分支创建。
- 一个任务只能有一个主工作分支。不得为同一个任务同时创建 `feat/a`、`fix/a2`、`qa/a` 等平行实现。
- 分支名使用小写英文和短横线，避免中文、空格和含义不清的编号。
- 分支完成后必须合入 `main`，再删除分支；不保留已经完成的“备份分支”作为隐性版本源。

创建新任务的标准流程：

```bash
git fetch origin --prune
git switch main
git pull --ff-only origin main
git switch -c feat/<short-name>
```

如果工作区不干净，先暂停并记录现有改动归属，不得直接切换或覆盖。

## 3. worktree 规则

- 默认不使用 worktree；普通任务直接使用一个工作目录和一个工作分支。
- 只有确实需要并行处理两个相互独立任务时才创建 worktree。
- 一个 worktree 只能绑定一个分支，一个分支只能绑定一个 worktree。
- worktree 目录统一放在仓库同级目录，例如：

```text
../kodo-worktrees/<short-name>
```

- 禁止在多个 worktree 中同时修改同一个任务的同一批文件。
- 禁止在 worktree 中直接 checkout 另一个已被其他 worktree 占用的分支。
- 创建 worktree 时必须从最新 `origin/main` 的新分支开始：

```bash
git fetch origin --prune
git worktree add -b feat/<short-name> ../kodo-worktrees/<short-name> origin/main
```

- 任务结束后，确认分支已经合入 `main` 且 worktree 没有需要保留的改动，再清理：

```bash
git worktree remove ../kodo-worktrees/<short-name>
git branch -d feat/<short-name>
git worktree prune
```

- 不得用 `git worktree remove --force` 清理含有未提交代码的 worktree。必须先检查、提交、转移或明确放弃这些改动。
- `node_modules`、`target`、构建产物和测试结果不是业务改动，不得提交；应通过 `.gitignore` 或显式排除处理。
- `.kodo/` 是本地运行数据，可能包含会话、配置或敏感信息，默认不得提交、合并或上传。

每次开始和结束任务都要检查：

```bash
git worktree list
git status --short --branch
```

## 4. 开发和提交规则

- 修改前先确认当前分支、worktree、基线和工作区状态。
- 一个提交只表达一个完整逻辑单元，不把多个无关任务混在一起。
- 提交信息使用统一格式：

```text
<type>(<scope>): <short description>
```

例如：

```text
feat(desktop): add skills navigation
fix(agent): preserve interrupted run status
test(ui): cover empty task creation
```

- 不提交临时调试代码、日志、密钥、`.env`、本地路径配置、用户运行数据或依赖目录。
- 不使用 `git add .` 盲目加入整个仓库；优先按文件或目录显式暂存，并在提交前检查 staged diff：

```bash
git diff --cached --stat
git diff --cached --check
git status --short
```

- 发现其他人或其他 Agent 的未提交改动时，不得覆盖、重置、清理或顺手提交；先区分归属并暂停相关操作。
- 修复冲突时必须逐个阅读冲突上下文，保留双方有效行为；禁止用 `ours` 或 `theirs` 对整个文件一键覆盖。

## 5. 合并策略

- 优先通过 PR 合并到 `main`；本地直接合并只用于明确授权的单人仓库操作。
- PR 的源分支只能是当前任务分支，目标分支固定为 `main`。
- 合并前必须确认：
  - 当前分支只包含本任务改动；
  - 所有相关提交已推送到 PR 分支；
  - PR 通过代码审查和 CI；
  - 没有未提交业务代码；
  - 没有遗漏的 worktree 分支或悬空提交。
- 默认使用 `--no-ff` 保留任务合并边界，除非仓库维护者明确要求线性历史。
- 禁止把多个长期功能分支一次性互相合并后再整体合入 `main`。正确顺序是：每个任务分支独立完成验证，再逐个合入 `main`。
- 禁止把 `main` 合并回功能分支后，再把该功能分支反向合并回 `main` 作为“同步”；需要更新功能分支时使用 `git rebase origin/main` 或明确的一次性 merge，并在 PR 中说明。
- 不使用 cherry-pick 代替正常分支合并；只有在恢复单个明确提交且无法正常合并时才允许，并记录原因。
- 合并后立即验证：

```bash
git status --short --branch
git branch --no-merged main
git log --oneline --decorate -n 10 main
```

## 6. PR、远程和审批

- 未经用户明确授权，不执行 `git push`、创建 PR、修改远程分支或调用远程代码托管 API。
- 获得授权后，推送任务分支，不直接强制推送 `main`：

```bash
git push -u origin feat/<short-name>
```

- PR 描述必须包含：变更范围、关键设计决策、验证命令、已知风险和未包含内容。
- PR 必须以 `main` 为目标分支，不得以另一个功能分支为长期目标。
- PR 作者不能审批自己的 PR；需要其他有权限的审阅者审批。Code Agent 不得伪造审批、绕过分支保护或修改审查记录。
- CI 失败时不得以“本地通过”为理由直接合并；必须先定位失败原因并修复或获得维护者明确豁免。
- PR 合并完成后，刷新本地引用并确认远程状态：

```bash
git fetch origin --prune
git log --oneline --decorate -n 5 origin/main
git branch -r --merged origin/main
```

## 7. 合并前检查清单

所有任务在合并前必须逐项确认：

```text
[ ] 当前工作在正确的任务分支和唯一 worktree
[ ] 分支从最新 origin/main 创建，或已明确同步基线
[ ] 没有覆盖其他 Agent 或用户未提交改动
[ ] 没有提交 .kodo、.env、密钥、node_modules、target、测试结果
[ ] git diff --check 通过
[ ] TypeScript / 前端构建通过（如涉及 apps/desktop）
[ ] Rust fmt、check 和相关测试通过（如涉及 Rust）
[ ] 相关 UI / 行为 / 单元测试通过
[ ] PR 描述包含验证结果和剩余风险
[ ] CI 通过，且已完成独立代码审查
[ ] 合并后 main 和 worktree 状态已复核
```

Kodo 当前常用验证命令：

```bash
cd apps/desktop
npm run typecheck
npm run build
npm run test:visual

cd ../..
cargo fmt --all -- --check
cargo check -p kodo-desktop
cargo test -p kodo-desktop --bin kodo-desktop
cargo test -p kodo-core --lib
```

## 8. 异常和恢复

- 如果发现多个分支都包含同一任务的不同版本，先停止合并，生成分支关系和提交清单，再由维护者选择唯一事实来源。
- 如果发现 worktree 有未提交改动，先按“保留、提交、转移、放弃”四种结果之一明确处理，不得直接删除。
- 如果合并冲突涉及业务逻辑，必须保留冲突前后差异并运行相关测试；无法判断时暂停并请求决策。
- 如果误提交了敏感文件，立即停止继续推送，记录受影响提交和远程范围，由维护者决定历史清理、凭据轮换和通知范围。
- 任何清理动作完成后都必须报告：清理了什么、是否可恢复、哪些分支和 worktree 被保留。

## 9. Code Agent 的强制汇报格式

涉及 Git 的任务完成时，必须报告：

1. 当前分支和 worktree。
2. 新建、合并、删除或保留的分支。
3. 新增的提交及其目的。
4. 是否修改远程、创建 PR 或推送代码。
5. 验证命令和结果。
6. 剩余未提交文件、未合并分支、未解决冲突和风险。

没有明确授权时，Code Agent 只做本地检查和本地修改，不擅自推送、建 PR、合并远程分支或删除 worktree。
