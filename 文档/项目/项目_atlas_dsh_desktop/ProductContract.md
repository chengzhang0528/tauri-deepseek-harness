# DSH Desktop 产品契约

Status: Active
Kind: ProductContract
Scope: atlas-dsh-desktop / 首版桌面交付
Owner: 项目维护者
Updated: 2026-08-20
Depends On:
- AGENTS.md

## 产品结果

DSH Desktop 是 DeepSeek Harness 的非官方 Windows 安装与运行宿主。它安装并管理应用私有运行时，启动 `dsh web`，再把 dsh 自己输出的完整 Web UI 原样加载到桌面 WebView，使用户无需预装 Node.js、pnpm 或 `dsh`。产品名使用 `DSH Desktop`；不得使用 `DeepSeek Harness Desktop` 暗示官方产品，非官方声明写入安装器元数据、发行说明和许可证材料，不为此创建页面。

## 职责边界

| 所有者 | 拥有 | 不得拥有 |
|---|---|---|
| DSH Desktop | 安装、WebView2 前置探测、私有 runtime、版本与制品校验、安全解包、doctor、进程树、启动/退出、托盘和更新生命周期 | 任何 Web 页面、业务 UI、页面路由、模型配置、工作区、任务、审批、会话、问题交互或 Harness 业务状态 |
| DeepSeek Harness / `dsh web` | WebView 中的全部 HTML、CSS、JavaScript、DOM、路由和业务交互，包括模型配置、工作区、任务、审批与会话 | Windows 安装、桌面 runtime 获取、制品激活、原生进程树和系统集成 |

WebView 中唯一允许呈现的 Web 内容是当前受管 runtime 执行 `dsh web --port 0 --no-open` 后返回的 dsh 页面。DSH Desktop 不创建启动页、等待页、错误页、更新页、关于页或任何兜底页面，也不复制、修改、覆盖或向该页面注入桌面端 DOM、CSS、脚本和控件。

## 首版边界

| 项目 | 已批准契约 |
|---|---|
| 平台 | Windows 10/11 x64 |
| 桌面框架 | Tauri 2，复用上游 Harness Web UI |
| 安装形态 | 当前用户级薄安装器；首次启动允许联网准备私有运行时 |
| 分发模式 | `self-use`；应用自有二进制可不做发布者签名，但必须绑定源码提交、对象键、大小、SHA-256 与 doctor 结果 |
| 应用自有二进制来源 | scheme `https`, host `shared-public-assets.oss-cn-beijing.aliyuncs.com`, prefix `atlas-dsh-desktop/` |
| 模型 | 不内置本地模型；模型与 API Key 仍由 Harness 的用户配置入口管理 |
| 窗口关闭 | 有活动任务时隐藏到托盘；无活动任务时可正常退出；托盘“退出”执行受控 drain |

首版不要求系统 Node、全局 PATH、Git、编译器或用户机器上的原生模块构建环境。系统 Git、外部 MCP 程序和其他第三方工具只有在 Harness 对具体能力明确要求时才作为可诊断的外部前置，不冒充内置运行时。

Windows WebView2 是由 Microsoft 服务的系统前置，不属于应用私有 runtime；Installer 必须先探测并通过 Tauri 官方安装模式补足缺失版本，随后重新探测。应用自有 Installer、Launcher、helper、manifest 与 runtime payload 仍只从上述 OSS 前缀交付。

## 用户可见行为

1. 安装、修复和卸载状态由 Windows Installer 表达；首次启动的下载、校验、解压、doctor 与失败恢复由原生 TaskDialog、托盘和系统通知表达，不先打开 WebView，也不出现桌面端页面。
2. 运行时身份验证 ready 后，DSH Desktop 动态创建唯一 WebView 窗口并直接加载本次启动产生的 dsh 回环 URL；窗口内完整内容归 dsh，不额外打开浏览器或终端窗口。
3. 下载失败、空间不足、校验失败、doctor 失败或 runtime 异常退出时，关闭或隐藏失效的 WebView，保留当前可运行版本与用户数据，并通过原生对话框显示失败组件、阶段、诊断位置和重试/退出动作；不得显示替代页面。
4. 应用可后台检查并暂存兼容更新；检查入口和确认动作只存在于原生托盘菜单与对话框。切换版本、重启或退出不得打断活动任务，必须等待 drain 并由用户确认。
5. 卸载默认删除安装器和运行时拥有的文件，保留 Harness 配置、会话、工作区登记和用户结果；删除用户数据必须是单独的明确选择。

## 数据与兼容性

- 应用二进制、运行时版本、暂存区与用户状态分开存放；运行时目录不可成为用户数据所有者。
- 桌面端使用独立 `DSH_HOME`，不静默接管或迁移用户已有 CLI Harness home。
- 首版不承诺历史 Launcher 协议或私有缓存布局的原位兼容。Launcher 不兼容时显示 `setup-required`，由新 Setup 执行官方替换路径。
- 上游处于 developer preview，持久数据采用 forward repair；未证明数据向后兼容前不得宣传或执行自动二进制回滚。

## 条件分支与非目标

- 明确要求严格离线安装时，另行构建约 300 MiB 级完整安装包；它不是首版薄安装器的一部分。
- 明确要求公开生产分发时，应用自有 Installer、Launcher 和 helper 必须增加 Windows Authenticode 代码签名；该授权不由自用构建或发布候选自动产生。
- 首版不交付 macOS、Linux、ARM64、后台 Service、计划任务、多窗口管理器、自有 Web 前端、内置模型或任意第三方插件市场。
