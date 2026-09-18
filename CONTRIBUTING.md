# Contributing

感谢关注 Kodo。

## 开发流程

1. 阅读根目录 `CLAUDE.md` 与 `docs/design/`。
2. GUI 改动须对照参考图与 `UI_ACCEPTANCE.md`；页面“能编译”不算完成。
3. 运行：

```bash
cargo test -p kodo-core -p kodo-agent
cd apps/desktop && npm run typecheck && npx playwright test
```

## 原则

- 忠实实现设计，不做二次创意改版。
- 不要增加未要求的导航层级、装饰卡片或后台式仪表盘。
- 左侧永远是**单一**项目树（项目下嵌套会话）。
- 默认安全：权限默认 `ask`；密钥不进 `settings.log`。

## 提交

- 说明“为什么改”，而不是罗列文件。
- 勿提交 `credentials.log`、`settings.log`、`node_modules/`、`target/`。
