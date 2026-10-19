# 实现方案

- 原则：遵守 AGENTS.md，单目录单分支 feat/scheduled-task-experience；原有改动不提交、不清理；不新增依赖。
- React：复用页面与 modal/input/button 样式，新建独立编辑器、表单模型和局部 CSS；创建编辑调用现有 API；项目来自已有 workspace；结果通过 App 现有 openConversation 路径打开并刷新会话列表。
- 时间：Daily/Weekly 保留 IANA 时区；Once 的墙上时间按所选时区转 UTC，拒绝 DST 空缺，重复时刻取最早；Interval 不编辑时保留 anchor。预览走 Rust，竞态用 effect 清理与请求序号屏蔽。
- 同步：监听 scheduler:task_changed / scheduler:run_changed，合并刷新；聚焦和可见页面周期刷新作为事件丢失恢复；历史按任务与序号隔离。
- Rust：枚举字段输出 camelCase，别名读取旧 snake_case；preview 缺省从当前时间开始；create/update 返回已存储快照。只修复接入实际阻碍，不重写调度器。
- 兼容：存储表结构不变，旧记录继续可读；旧配置字段、persona、budget、资源和策略在编辑时保留。无迁移命令。
- 验证：离线 Rust check / 调度模块测试，前端 typecheck/build，Playwright 隔离验证创建编辑、时区、冲突、事件更新、历史竞态、会话跳转与窄屏；不调用真实 provider 或用户数据库。

一致性分析：FR-001~006 均有对应任务和验收项；网络权限仅覆盖公开官方资料；提交、合并待实现和验证后另行确认。

实施中发现并纳入同范围修复：旧调度器把手动试跑当作计划执行推进游标，补跑只取前六次，Skip 会跳过正常执行。现以持久化游标计算最近已到期时刻；Interval 直接计算，Daily/Weekly 沿用 Core 时区计算器。状态改变增加 revision，避免编辑覆盖暂停操作；拒绝归档中的执行和复活已归档任务。资源准入被拒绝时写入跳过结果，避免永久显示排队。重叠策略配置暂不扩展，仅保留旧配置；本次不宣称实现 QueueLatest 队列语义。
