# Kodo Git 开发与合并规则

本文档是 Kodo 项目的 Git、分支和 worktree 操作规范。所有 Code Agent、开发者和自动化任务都必须遵守。

## 1. 仓库主线

- `main` 是唯一集成主线，始终代表可交付代码。
- 不在 `main` 上直接开发、修改业务代码或堆积临时提交。
- `main` 合并前必须通过与改动范围匹配的验证；未验证的代码不得合入。
- 本地 `main` 是本地开发基线，远程 `origin/main` 是团队共享基线。开始新任务先检查本地状态和已有引用；未获联网授权时，不把远程同步作为本地评审、开发和验证的前置条件。
- `origin/main` 是最近一次 fetch 的本地缓存，不能仅凭它与 `main` 相同就宣称远程已同步。必须记录所用基线哈希，以及远程是否已在本次任务中核验。
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

- 分支默认从已检查并确认的本地 `main` 创建；已获联网授权并完成同步时，从已核验的 `origin/main` 创建。不从其他功能分支、旧 worktree 或未确认的本地分支创建。
- 一个任务只能有一个主工作分支。不得为同一个任务同时创建 `feat/a`、`fix/a2`、`qa/a` 等平行实现。
- 分支名使用小写英文和短横线，避免中文、空格和含义不清的编号。
- 分支完成后必须合入 `main`，再删除分支；不保留已经完成的“备份分支”作为隐性版本源。

创建新任务的默认本地流程（先按第 3.1 节确认现有改动归属）：

```bash
git status --porcelain=v1
git rev-parse main
git switch -c feat/<short-name> main
```

需要远程协作、用户要求同步或准备远程交付时，再说明必要性并取得联网授权，执行 `git fetch origin --prune`，检查 `main...origin/main` 的差异。更新本地 `main`、rebase 或 merge 仍按 Git 历史修改的审批规则执行，不自动执行 `git pull`。

如果工作区不干净，先记录并确认现有改动归属。已确认与本任务无关、明确原样保留且不会被分支切换覆盖的改动，可以保留现场继续；未确认归属或存在覆盖风险时，暂停相关变更。

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
- 创建 worktree 时使用第 2 节已确认的基线，默认本地流程如下；只有已获联网授权并完成同步时才改用 `origin/main`：

```bash
git worktree add -b feat/<short-name> ../kodo-worktrees/<short-name> main
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

### 3.1 Worktree 卫生与陈旧检查

每次会话开始、结束、跨任务切换时，执行一次 `git worktree list --porcelain`、`git branch --no-merged main` 与 `git stash list`，确认没有遗留现场。出现以下任意一项，必须先停下来报告、再决定保留/提交/转移/放弃，不得直接清理：

- 任何 worktree 存在尚未确认归属的改动（`git status --porcelain=v1` 有文件记录；`--short --branch` 的分支标题不算改动）；
- 任何 worktree 的分支早于 7 天没有新提交、或最后提交不属于当前任务；
- 存在未合并到 `main` 的本地任务分支、却没有对应的活跃 worktree；
- `stash list` 非空，且最近一条 stash 晚于本会话开始时间。

本会话已确认原样保留的文件、分支、worktree 和 stash 不重复请求确认；只有出现新增改动、归属变化或需要清理时重新报告。新任务分支尚无独有提交时，其基线提交不属于当前任务是正常状态，不据此认定为陈旧现场。

清理约束：

- 不得用 `git worktree remove --force`、`git branch -D`、`git stash drop` 或 `git stash clear` 跳过归属判断；
- 删除或恢复前必须把"涉及哪些分支 / worktree / stash / 提交"清单交给用户复核；
- 任意一项没有归属结论前不得动手。

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

### 4.1 文件修改前的现场刷新

为了防止"基于旧上下文覆盖新代码"，修改任何**已存在**的文件前：

- 必须重新读取其当前磁盘内容（`Read` 或 `git show HEAD:<file>`），不得仅凭会话早期缓存的整段内容下笔。
- 提交前对每个改动文件跑一次新鲜度自检：

  ```bash
  git log --oneline -n 5 -- <file>           # 看最近谁改过、何时改
  git blame -L <start>,<end> <file>          # 看关键段落的作者/时间
  git status --short -- <file>               # 看是否还存在未提交改动
  ```

- 若发现目标文件在本会话开始后有非本任务的新提交、或自己工作区里已有未确认归属的他人修改，立即停下来报告并请求归属确认。会话开始前已检查的基线提交不重复触发确认。
- 写完一个文件后立即做最小自检：`cargo check -p <crate>`、`npm run typecheck` 等只覆盖本文件所在 crate / 模块的命令；不要因为"已经 add 了"就跳过。
- `Write` / `Edit` 之后立刻再 `git diff -- <file>` 一次，确认没有把不相关的改动带进来。

### 4.2 检测并避免覆盖其他 Agent 或用户的改动

工作区里出现以下任意信号，都按"可能踩到他人改动"处理：

- `git status` 出现本会话没创建过、且尚未确认归属的未跟踪文件或未提交修改；
- `git log --branches --remotes --since="<本会话开始时间>"` 出现非本任务的提交；
- `git stash list` 非空，且存在晚于本会话开始的 stash；
- 工作区里有 `.kodo/`、`settings.log`、`credentials.log`、`projects.log`、`sessions/` 等本地运行态文件（不得覆盖、也不得删除他人版本）；
- `git reflog` 在本会话窗口内出现 reset / checkout / merge / commit 之外的非常规条目。

收到响应后的处置：

- 不得执行 `git stash`、`git stash pop`、`git reset`、`git checkout -- <file>`、`git clean -fd` 等可能丢改动的命令。
- 立刻把信号整理为报告（文件清单 / 提交哈希 / stash 引用 / reflog 条目），交给用户确认归属。
- 在用户明确授权前保持现场；如果不放心，先在旁路分支（`stash@{N}` 或新建 `wip/<task>-<timestamp>`）保留快照，再决策。

## 5. 合并策略

- 优先通过 PR 合并到 `main`；本地直接合并只用于明确授权的单人仓库操作。
- PR 的源分支只能是当前任务分支，目标分支固定为 `main`。
- 合并前必须确认：
  - 当前分支只包含本任务改动；
  - 远程 PR 流程中，所有相关提交已推送到 PR 分支，PR 通过代码审查和 CI；
  - 已明确授权的本地合并完成对应本地验证，并如实报告审查范围和未运行的远程 CI；
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

### 5.1 合并前的基线新鲜度检查

经用户明确授权合并到本地 `main` 之前，先检查本地集成基线和本任务提交：

```bash
git rev-parse main
git merge-base HEAD main
git log --oneline main..HEAD
git log --oneline HEAD..main
```

准备远程 PR / 推送 / 合并时，必须在已有联网授权范围内刷新远程引用，再检查共享基线：

```bash
git fetch origin --prune
git rev-parse origin/main                  # 当前基线
git merge-base HEAD origin/main            # 共同祖先
git log --oneline origin/main..HEAD        # 本任务的独有提交
git log --oneline HEAD..origin/main | head # main 上本任务不掌握的新提交
```

- 未获联网授权时，可以完成已明确授权的本地合并，但必须报告“远程新鲜度未核验”；不得宣称已满足远程交付条件。已有授权覆盖的同一动作不重复确认；fetch 授权不代表 push、PR 或合并授权。
- 若 `origin/main` 领先共同祖先很多，先决定是 rebase 还是 merge commit；不要"自动跳过"。
- 任何 `--ours` / `--theirs` 选择都要在 PR 描述里写明语义理由。**`git merge X` 在 Y 分支执行时，`ours` = Y、`theirs` = X**；这是 2026-10-08 recover 事故中实际踩过的方向陷阱，必须显式确认。
- 合并完成后立即对关键文件做体积 / 行数 sanity check：

  ```bash
  git show HEAD --stat
  wc -l <关键文件> <关键文件>
  git diff --stat HEAD~1 HEAD -- <目录>
  ```

  若关键文件比预期小很多、或出现"功能消失"的报告，必须先比对 `git stash list`、reflog、`git fsck --lost-found`，再决定下一步；不要直接 reset。

- 不允许"Git 无冲突 = 语义正确"。跨大文件 / 跨 crate 的合并，跑改动范围对应的 `cargo check` / `cargo test -p` / `npm run typecheck` 至少一次。

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
[ ] 本地开发基线已检查并记录；远程交付时已同步并核验 origin/main
[ ] 没有覆盖其他 Agent 或用户未提交改动
[ ] 没有提交 .kodo、.env、密钥、node_modules、target、测试结果
[ ] git diff --check 通过
[ ] TypeScript / 前端构建通过（如涉及 apps/desktop）
[ ] Rust fmt、check 和相关测试通过（如涉及 Rust）
[ ] 相关 UI / 行为 / 单元测试通过
[ ] 交付说明包含验证结果和剩余风险；远程 PR 流程中写入 PR 描述
[ ] 审查范围已说明；远程 PR 流程中 CI 通过且已完成独立代码审查
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

### 8.1 Stash 与本地暂存保护

Stash 是不可见的"软副本"，必须按以下规则处理：

- `git stash` 默认只保留已跟踪文件；要保留新文件必须显式 `git stash -u`（含 untracked）；不写 `-u` 时新文件直接暴露在工作区，容易被后续 `git checkout .` / `git clean` 误删。
- 严禁在没看清 `git stash list` 的情况下执行 `git stash drop` 或 `git stash clear`；stash 一旦 drop，仅 reflog / `fsck --lost-found` 可救，且不保证完整。
- 任何"功能消失"类报告，第一动作是 `git stash list` 看是否有 in-flight stash（数量 / 文件数 / 总行数 / 时间）；不要先 reset / rebase / clean。
- 恢复 stash 时优先用 `git checkout stash@{N} -- <path>` 单文件取用，比 `git stash pop` 更可控；`pop` 之后只剩单边，做错就难撤回。
- 项目当前 `stash list` 中仍有未 drop 的条目；任何 drop / pop 必须由用户在最近会话里**显式**确认。Code Agent 不得以"已经合并到 main"为由擅自清理 stash。
- 长期保留 stash 时，建议每 7 天向用户确认一次是否还需要；不要让它静默躺几个月。

## 9. Code Agent 的强制汇报格式

涉及 Git 的任务完成时，必须报告：

1. 当前分支、worktree 路径，以及**基线 commit 哈希**（`git rev-parse HEAD` 与 `git rev-parse origin/main`）。
2. 新建、合并、删除或保留的分支；附当前 `git branch --no-merged main` 与 `git worktree list --porcelain` 输出。
3. 新增的提交哈希及其目的；同时说明 `git stash list` 是否变化（新增 / pop / drop / 保留）。
4. 是否修改远程、创建 PR 或推送代码；未经授权一律报告为"未做"。
5. 验证命令和结果：至少 `git status --short --branch` 与改动范围对应的 build / test。
6. 剩余未提交文件、未合并分支、未解决冲突、未 drop 的 stash 与任何遗留风险。

没有明确授权时，Code Agent 只做本地检查和本地修改，不擅自推送、建 PR、合并远程分支或删除 worktree。

### 9.1 任务完成门

报告"任务完成"时，"完成"的最低含义是：

- 本任务的代码**已经存在于 `main`**（本地 `main` 已包含本任务独有 commit，或已经推送并开了 PR 等用户合并）；**仅"代码在某个本地分支上"不算完成**。
- 已经运行改动范围对应的 test / build，结果在报告里给出原始输出。
- 任务开始时已有的 stash / worktree / 分支归属都已确认，没有静默遗留。
- 临时调试代码、未提交的 print / log、没有在 PR 描述里的 TODO 都已清理或显式说明。

若当前任务无法满足完成门，必须显式报告：

- 未完成的工作清单；
- 保留位置（哪个分支 / worktree / stash / reflog 条目）；
- 恢复路径（如何回到本任务的最新状态）。
