# DSH Desktop 项目智能体入口

Status: Active
Kind: AgentEntry
Scope: atlas-dsh-desktop
Owner: 项目维护者
Updated: 2026-08-20
Depends On:
- ../../../AGENTS.md

本项目交付 DeepSeek Harness 的 Windows 安装与运行宿主和应用私有运行时。当前工作空间尚无产品源码；开始编码前先读 [ProductContract.md](ProductContract.md) 和 [CurrentDesign.md](CurrentDesign.md)，再用 `rg` 确认实际源码、类型、测试和消费者，不从本文推断尚未创建的目录。

## 路由

- 产品名、平台、安装形态、分发模式、OSS 前缀和用户可见行为：`ProductContract.md`。
- Tauri、运行时闭包、进程生命周期、更新、状态与实施门禁：`CurrentDesign.md`。
- 安装器、更新或发布技术工作命中 `client-application-development`；实现受管运行时能力同时命中 `develop-managed-client-capability`。

## 项目门禁

- 必须直接加载 `dsh web` 输出的完整页面；不得创建任何 DSH Desktop 自有 Web 页面、前端入口、业务 UI 或替代页面，不得复制、改写或覆盖 dsh 的 DOM、样式、脚本与交互。
- Tauri 只作为原生生命周期后端和 WebView 宿主：启动配置不预建窗口，runtime ready 后才动态创建 WebView 并加载验证过的 dsh 回环 URL；远程 dsh 页面不得获得 Tauri IPC 或 capability。
- 安装、runtime 准备、doctor、更新、退出等待和错误恢复只使用 Windows Installer、原生对话框、托盘菜单与系统通知表达，不向 WebView 注入桌面端控件。
- 桌面端只启动 manifest 固定且 doctor 通过的私有运行时，不执行 `@latest`，不把系统 Node 当作产品运行时。
- Harness Web 页面不获得 Tauri shell、文件系统、进程或更新权限；桌面特权只在 Rust 后端。
- 不用固定端口、TCP 可连接或未知回环服务充当 Harness 身份证明。
- 正常退出先 drain 活动任务；强杀只用于用户明确选择或进程失控后的兜底。
- 不把 Git push、构建成功或 Development 验证当成 SystemTest 或 Deployment。
