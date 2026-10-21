# Security

## Reporting

若发现安全问题，请勿在公开 Issue 中粘贴密钥、完整日志或可利用细节。请通过 GitHub Security Advisories 或仓库维护者私信报告。

## 本地数据

| 文件 | 内容 |
|---|---|
| `settings.log` | 普通配置（**不含** API Key） |
| `credentials.log` | Provider 密钥，Unix 权限 `0600` |

macOS 默认路径：`~/Library/Application Support/Kodo`

请勿提交或外传上述目录中的日志文件。

## Agent 权限

- 默认 `ask`：命令与写文件需人工批准。
- 写文件路径限制在项目根内（拒绝 `..` 与绝对路径）。
- 危险命令在审批 UI 中标注原因。

配置 Provider 后，对话与本地笔记可能发送到**你自己配置的**模型端点；Kodo 不会内置遥测服务。

## 沙箱边界

审批只回答"这条命令能不能开始"，不回答"它能碰到什么"。
沙箱把第二个问题交给操作系统，并且对所有后代进程生效。

- 默认 `workspace-write`：可写工作区，项目外写入、`.git`、`.kodo` 被系统拒绝。
- 不可信 Shell 子进程默认不能出站网络（macOS `deny network*`，Linux 独立网络命名空间）。
- 符号链接逃逸在操作系统层被阻止。
- 沙箱不可用时受限命令被**拒绝执行**，不会静默改为无沙箱运行。
- 越权需要单次授权，授权只对当前会话的当前命令生效一次。

已知限制：文件读在 macOS 上是拒绝清单（能力报告显示为"部分"）；
硬链接与 TOCTOU 不在承诺范围内；外部 MCP 服务不受沙箱限制，审批时会明确标注。
完整能力矩阵与验证方式见 `docs/design/SECURE_EXECUTION.md`。
