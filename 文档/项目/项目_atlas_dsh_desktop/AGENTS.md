# DSH Desktop 项目智能体入口

Status: Active
Kind: AgentEntry
Scope: atlas-dsh-desktop
Owner: 项目维护者
Updated: 2026-09-16
Depends On:
- ../../../AGENTS.md

本项目交付 DeepSeek Harness 的 Windows 安装与运行宿主和应用私有运行时。已有 Rust 宿主位于 `src-tauri/src/`，桌面 bridge 位于 `src-tauri/resources/`，构建和定向测试位于 `scripts/`。开始编码前先读 [ProductContract.md](ProductContract.md) 和 [CurrentDesign.md](CurrentDesign.md)，再用 `rg` 核对实际实现及消费者；设计中的目标行为不等于代码已实现。

## 路由

- 领域模型、层次确认与需求增量：[模型源目录](../../../../xrain-ontology/建模档案/tauri-deepseek-harness/)及[模型索引](../../../../xrain-ontology/建模档案/tauri-deepseek-harness/模型索引.md)；按当前问题直接读取，不依赖挂载。

- 产品名、平台、安装形态、分发模式、OSS 前缀和用户可见行为：`ProductContract.md`。
- Tauri、官方依赖接入、本机副本、进程生命周期、更新、状态与实施门禁：`CurrentDesign.md`。
- 本机安装、启动与恢复命令：`Runbook.md`。
- 安装器、更新或发布技术工作命中 `client-application-development`；实现受管运行时能力同时命中 `develop-managed-client-capability`。

## 项目门禁

- 必须直接加载 `dsh web` 输出的完整页面；不得创建任何 DSH Desktop 自有 Web 页面、前端入口、业务 UI 或替代页面，不得复制、改写或覆盖 dsh 的 DOM、样式、脚本与交互。
- Tauri 只作为原生生命周期后端和 WebView 宿主：启动配置不预建窗口，runtime ready 后才动态创建 WebView 并加载验证过的 dsh 回环 URL；远程 dsh 页面不得获得 Tauri IPC 或 capability。
- 安装、runtime 准备、版本查看、更新、退出等待和错误恢复使用 Windows Installer、窗口原生菜单、原生对话框、托盘和系统通知表达，不向 WebView 注入桌面端控件。
- 官方依赖直接采用官方版本、发布位置和安装方式，不增加自有摘要、闭包认证、doctor 或重新发布门槛。宿主管理私有副本、精确目标与本机操作结果，不把系统 Node 当作产品运行时。
- Harness Web 页面不获得 Tauri shell、文件系统、进程或更新权限；桌面特权只在 Rust 后端。
- 不用固定端口、TCP 可连接或未知回环服务充当 Harness 身份证明。
- 正常退出消费 Harness 提供的完整待完成工作、实际停止接纳及收尾证据；未知不按零处理。全树结束与正常收尾分别判断；强退、切换确认及安装器替换边界由 ProductContract 持有。
- 不把 Git push、构建成功或 Development 验证当成 SystemTest 或 Deployment。
