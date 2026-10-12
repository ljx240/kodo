# 权限功能评审与修复记录（2026-10-12）

## 结论

截图中的“项目内自动执行”对应 `Permission::Full`。原实现只把 `git --version` 视为安全 Git 命令，`git status --short` 被分类为 `Unknown`，因此仍生成审批请求。这是命令分类过严，并非截图能证明设置保存失败。

已修复有限 Git 元数据查询的分类和执行边界、权限选择的界面同步与启动读取竞态，并补充权限实际范围和生效时机的说明。未增加依赖、遥测、联网行为或操作系统沙箱。

## 评审范围与发现

本地检查覆盖：Composer → 共享设置 hook → ShellConfig → 桌面运行参数 → `Permission::needs_approval` → 工具执行器；同时检查 CLI、验证命令执行器和审批指纹的调用路径。截图里的 reviewer 角色没有覆盖这条权限配置链路；角色模型默认值与工具权限分开处理。

| 优先级 | 问题与影响 | 处理 |
| --- | --- | --- |
| P1 | Full 自动放行部分 shell 文件写入，但 `cwd` 在项目内不代表目标路径在项目内，`mkdir ../outside` 等可越界。 | shell 文件写入改为审批；已有项目文件工具保留规范化路径和敏感路径检查，Full 仍可自动编辑项目文件。 |
| P2 | `git status --short` 等元数据查询落入 Unknown，使项目自动模式产生不必要审批。 | 严格匹配子命令与参数，并在共享执行器内约束 Git 的潜在副作用。 |
| P2 | Composer 独立持有权限状态，设置页改动不会同步；延迟启动读取可覆盖用户刚选择的权限。 | 复用已有 `useSetting` 事件同步与晚到读取保护；增加双向同步和竞态测试。 |
| P2 | 旧 Full 文案暗示多数项目命令均可自动执行，与 Unknown、网络、安装依赖等审批规则不符。 | 同步修改菜单说明、设置说明与输入框提示，明确从下一轮生效。 |

## 修复后的权限行为

| 操作 | 请求批准（ask） | 自动批准安全操作（auto） | 项目内自动执行（full） |
| --- | --- | --- | --- |
| 内置项目文件读取、搜索 | 自动 | 自动 | 自动 |
| 已识别的只读 shell / Git 元数据查询 | 审批 | 自动 | 自动 |
| 经路径检查的项目文件编辑工具 | 审批 | 审批 | 自动 |
| shell 文件写入、未知程序或脚本、复合命令 | 审批 | 审批 | 审批 |
| 网络、安装依赖、敏感操作、破坏性 Git、外部工具 | 审批 | 审批 | 审批 |

自动 Git 查询支持严格限定的 `status`、`branch`、`rev-parse`、`log --oneline`、`diff --stat/--name-only/...` 及安全参数。`-C`、任意 `-c`、输出重定向、追加 shell 命令、写操作不进入自动白名单。`cargo check`、`npm test`、`rg`、裸 `git diff` 等仍需要批准；它们没有操作系统级隔离保证。

权限配置在一轮开始时读取并固定。修改选择影响下一轮，已出现的审批卡不会被自动撤销。“本会话允许”按步骤类型和完整命令生成指纹，不表示放行任意后续命令；桌面端清理待审批票据时保留会话允许记录。本次不调整这一生命周期。

## Git 自动查询的执行边界

- 分类和执行复用同一严格参数匹配函数，禁止把任意命令尾部拼入自动路径。
- 禁用 pager、fsmonitor，以及 Git 的可选索引写入；差异统计禁用 external diff 和 textconv。
- `status` / 差异统计可能运行 clean/process 过滤器。先读取过滤器配置的键名，使用子进程环境覆盖为关闭，不把配置值提供给模型，也不修改仓库配置。
- 配置查询失败、截断、乱码或不完整时停止，避免未经检查的回退执行。查询与实际命令共享超时预算及取消检查，沿用进程树终止、输出上限和敏感内容脱敏。
- 子模块具有独立配置，自动 `status` / 差异统计使用 `--ignore-submodules=all`，并在结果中提示不包含子模块内部状态。禁用过滤器时也提示文件状态可能与过滤器归一化后的结果不同。

临时仓库验证先证明普通 Git 查询会触发 fsmonitor、过滤器和子模块过滤器；再断言自动查询不触发这些脚本、父子仓库索引字节均保持不变。补充夹具时曾因修改前后文件大小不同而没有触发 Git 内容转换，已改为等长内容变化，最终回归通过。

## 改动文件

- `crates/agent/src/tools.rs`：风险分类、受限 Git 查询执行、shell 写入审批与回归测试。
- `crates/agent/src/lib.rs`：权限说明与真实工具执行的 Ask / Auto / Full 测试。
- `apps/desktop/src/conversation/Composer.tsx`：复用共享权限状态与生效时机提示。
- `apps/desktop/src/data/models.ts`、`apps/desktop/src/i18n.ts`：权限范围说明。
- `apps/desktop/tests/visual/behaviour.spec.ts`：权限同步与启动读取竞态测试。
- 本文档：评审、验证与剩余限制。

## 验证命令与原始结果摘要

全部使用已有本地依赖；Cargo 使用 `--offline`。前端使用已安装的 Node 24，未安装新依赖。

```text
cargo test --offline -p kodo-agent tools::tests --lib
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 470 filtered out; finished in 2.96s

cargo test --offline -p kodo-agent permission --lib
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 487 filtered out; finished in 0.21s

cargo check --offline -p kodo-desktop -p kodo-cli
    Checking kodo-agent v0.0.1
    Checking kodo-shell v0.0.1
    Checking kodo-desktop v0.0.1
    Checking kodo-cli v0.0.1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.26s

cargo fmt --all -- --check
exit 0，无输出

git diff --check
exit 0，无输出

cd apps/desktop
npm run typecheck
> tsc --noEmit
exit 0

npm run build
✓ 4095 modules transformed.
✓ built in 1.84s
exit 0；已有部分 bundle 超过 500 kB 的提示

npm run test:visual -- --project=behaviour behaviour.spec.ts --grep 'permission settings|late permission|tools & permissions|an approval request'
4 passed (3.1s)
```

界面测试覆盖设置与输入框双向同步、延迟启动读取、权限设置页，以及原有审批卡允许/拒绝流程。Rust 测试覆盖 Git 状态真实执行、危险参数拦截、命令取消与超时、敏感路径、外部工具审批等。

## Git 现场与交付边界

- 唯一 worktree：`/Users/lijianxue/work/WorkSpace/kodo`。
- 本任务分支：`fix/project-auto-permissions`，从已检查的本地 `main` 创建。
- 开发基线：`9dcabae46d3c627575660b79ad565d71c2f8731f`。
- 缓存 `origin/main`：`211d0413d3b227b9bb36971db9a97a5e8fde3a34`；未联网，不能据此宣称远程已同步。
- 原有两个未跟踪文件 `apps/desktop/scripts/capture-welcome.mjs`、`docs/总体架构设计.md` 原样保留，不纳入本任务。
- 原有 stash 两条原样保留：`stash@{0}` 为 `9abd8060247ecfb8e2e0155fa8d7f06e6177fa15`；`stash@{1}` 为 `81465bee9001d4b6a119622be9060ab7d788ffce`。
- 用户已明确授权本任务七个文件的本地提交、以 `--no-ff` 合入本地 `main`、验证后删除任务分支；实际提交与合并哈希见交付报告中的 Git 原始输出。未做 fetch、push、PR 或远程变更。

## 剩余限制

- 自动分类依赖可信的主机 Git 可执行文件与 PATH，不能替代操作系统沙箱。经过人工批准的任意 shell 命令仍可能越出项目。
- 禁用过滤器、忽略子模块会影响自动查询的完整性，结果中已提示；完整状态需批准带明确参数的查询，例如 `git status --short --ignore-submodules=none`。
- 共享设置 hook 仍乐观更新、异步保存，保存失败尚无专门界面反馈；极端保存失败或立即发送时，显示值与下一轮实际读取值仍可能暂时不一致。本次没有修改通用设置持久化协议。
- 未启动或重新打包安装用户当前运行的 Kodo，未检查真实会话和本地敏感运行数据。界面验证采用本地 Tauri stub，安全验证采用隔离临时仓库。
- 未运行完整 Rust / 全量截图套件、Windows 运行验证、联网模型调用或远程 CI；本次为本地源码审查与针对性回归，未宣称独立审查通过。
