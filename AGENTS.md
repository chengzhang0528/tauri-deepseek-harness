# coder_driver 工作空间智能体入口

Status: Active
Scope: coder_driver-template
Owner: 项目维护者
Updated: 2026-08-13
Depends On:
- none

本模板只提供文档驱动协同控制面，不预设产品、技术栈、服务、端口或源码根。

## 启动

1. 先读 `文档/TASK_CONTROL.md`；仅在用户询问后续或路线图且文件存在时读 `文档/` 下的 `WORK_CANDIDATES.md`。
2. 命中项目时读 `文档/项目/项目_<id>/AGENTS.md`；涉及代码再读目标源码根 `AGENTS.md`、类型和测试，并先用 `rg` 查现有实现。
3. 改变工作空间、制品或交付环境时，读 `文档/工作流/WORKFLOW_CONTRACT.md` 并选择唯一主 Workflow；收口读 WF-0004。
4. 创建、移动、删除或无法判断文档位置时，读 `文档/WORKSPACE_STRUCTURE.md`。

事实优先级：当前用户要求 > 源码、类型、迁移与测试 > 唯一 ProductContract、CurrentDesign、Decision、Runbook > Git 与 Archive。来源冲突时停止扩张并修复唯一事实所有者。

## 门禁

- Codex 自动发现 `.agents/skills`；任务命中 skill 时按其方法执行，根入口不复制 skill 触发表。
- 任务类型只由用户目标决定；不得从测试规模、CI、候选、环境或 Git 状态推断 SystemTest 或 Deployment。
- 调用代码或命令前查真实契约；公共接口、DTO、数据库、权限或跨项目变更先识别消费者，保持最小改动。
- Development 完成范围匹配的源码或类型检查及必要定向测试；独立系统测试和部署必须由用户明确建立。
- 仅需跨会话恢复的受控任务登记 `TASK_CONTROL.md` 并绑定对应活动计划；长期能力事实变化时更新唯一正式所有者。
- 服务命令、配置、迁移与恢复只由项目 Runbook 定义；正式文档或治理入口变化后运行 `npm run check:docs`。
- 应用层不得新增或代行 HTTPS/TLS 强制、证书策略、协议限制、Origin/Host 校验、重定向策略或网络来源白名单，也不得拒绝部署已配置的 HTTP 入口；这些职责属于部署入口、反向代理、操作系统/浏览器或项目明确的平台所有者。应用只保留业务鉴权、授权以及数据和制品完整性校验，不把传输安全策略伪装成业务规则。
- 不写入或提交密钥、连接串、Token、密码、私钥、客户数据、日志或生成物；不发送可选过程播报。

## 收口

最终先给出 `通过` 或 `不通过`，并报告本次任务类型、改动、受影响项目、证据和 Git 状态；不得声明项目或会话阶段。独立工作通过后使用 `.agents/skills/git-closeout/SKILL.md` 精确提交并推送，Git 状态不反转功能结论。
