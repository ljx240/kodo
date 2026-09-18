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
