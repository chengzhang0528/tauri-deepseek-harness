# DSH Desktop 当前技术设计

Status: Active
Kind: CurrentDesign
Scope: atlas-dsh-desktop / Windows x64 桌面壳与私有运行时
Owner: 项目维护者
Updated: 2026-08-20
Depends On:
- ProductContract.md

## 评审结论

采用稳定的 Tauri 2 Native Host/Launcher 管理版本化私有 Node 24 与固定 DeepSeek Harness 运行时闭包。首版使用联网薄安装器和单一 OSS 制品闭包；不在用户机器执行 `npm install @latest`，不复用系统 Node，不固定 3080 端口。已批准方向完整，可进入编码；本设计不授权独立 SystemTest 或正式发布。

| 结果/决策 | 当前支持 | 必需变化 | 用户表面 | 所有者 | 持久化影响 | 证据 | 开放决策 |
|---|---|---|---|---|---|---|---|
| 桌面宿主 | 工作区无产品源码；参考实现证明 Tauri 壳可行 | 建立无前端 bundle 的 Tauri 2 原生宿主、托盘、单实例和隐藏后台进程 | runtime ready 后只显示完整 dsh Web UI | Native Host | 只保存原生客户端偏好；不拥有业务数据 | 上游已有 `dsh web`；参考项目 `a8f32fdb` | 无 |
| 私有运行时 | 上游要求 Node `^22.19.0 || >=24.0.0` | 固定 Node 24、dsh、pnpm、原生模块和 ripgrep 的 win-x64 闭包 | 原生 TaskDialog、托盘与系统通知 | Launcher | 新增版本化 runtime/cache/staging | Windows 测量闭包下限超过 282 MiB | 无 |
| 服务启动 | 上游支持随机端口和 `--no-open` | 解析 readiness，验证 Harness bootstrap 后才创建外部 URL WebView | 完整 dsh 工作台，无桌面端页面 | Process Supervisor | 无业务持久化变化 | 上游 `dsh web --port 0 --no-open` | 无 |
| 退出与托盘 | 参考实现只会 `taskkill /T /F` | Job Object、桌面桥、活动任务 drain、原生确认和明确强退 | 托盘与原生 TaskDialog | Native Host + Bridge | 只保存客户端偏好 | Windows 不能可靠向隐藏进程投递上游依赖的 `SIGTERM` | 无 |
| 更新与恢复 | 当前无客户端更新实现 | manifest 下载、校验、doctor、暂存、确认激活和 forward repair | 原生托盘命令、TaskDialog 与系统通知 | Launcher + Native Host | 新增 current/staged 指针；用户数据不迁入 | 上游 developer preview 不保证数据向后兼容 | 无 |
| 分发 | 当前无本项目制品 | 当前用户 MSI 和 `atlas-dsh-desktop/` OSS 闭包 | Setup、修复、卸载 | Installer + Release tooling | 安装注册与用户数据分离 | 已批准薄安装器、自用分发和唯一前缀 | 无 |

## 目标架构

```text
Windows current-user MSI
  -> stable Tauri Native Host / Launcher
       -> Bootstrap + manifest client
       -> Runtime manager (download / verify / unpack / doctor / activate)
       -> Process supervisor (hidden process + Windows Job Object)
       -> native TaskDialog / tray / system notification
       -> after verified ready: one WebView (exact dsh runtime loopback URL only)
  -> versioned private runtime
       -> Node.js 24 win-x64
       -> pinned @deepseek-ai/dsh closure
       -> pinned pnpm + native modules + ripgrep + third-party notices
       -> minimal Cordis desktop bridge
```

项目不包含 frontend bundle、HTML/CSS/JavaScript 页面入口或前端框架。Tauri 配置使用 `windows: []` 和 `withGlobalTauri: false`；启动期间不创建 WebView。Harness 身份验证 ready 后，Rust 后端通过 `WebviewWindowBuilder` 动态创建唯一窗口并将 `WebviewUrl::External` 指向本次 supervisor 验证的精确 dsh 回环 URL。

该远程页面不配置 Tauri capability、IPC 或 `dangerousRemoteDomainIpcAccess`，不持有 shell、文件系统、进程、下载或更新权限。桌面端不注入 preload、初始化脚本、DOM、CSS、更新控件或诊断组件。WebView 只承载 dsh origin；离开该 origin 的普通外部 HTTP(S) 链接交给系统浏览器，不在应用层新增 TLS、证书、协议、Origin/Host、重定向或网络来源策略。

| 生命周期场景 | 唯一可见表面 |
|---|---|
| 安装、修复、卸载 | Windows Installer / WiX 原生界面 |
| 首启下载、校验、解压、doctor | Rust 调用的原生 TaskDialog；必要时系统通知 |
| 正常工作 | `dsh web` 返回的完整页面 |
| 检查、暂存、激活更新 | 原生托盘菜单、TaskDialog 与系统通知 |
| 退出 drain、强退确认、runtime 故障 | 原生 TaskDialog；失效 WebView 隐藏或销毁 |

任何场景都不得使用 DSH Desktop 自有 Web 页面兜底。runtime 恢复并重新通过身份验证后，重新创建 WebView 或将现有 WebView 导航到新的 dsh URL。

## 制品与版本契约

Installer、Launcher 与 runtime 分别版本化。实现时建立一个规范版本文件作为 Installer/Launcher 当前版本唯一所有者，release manifest 作为 runtime 组件版本唯一所有者。普通 runtime 更新不得重建或改写稳定 Installer 版本。

```text
scheme: https
host: shared-public-assets.oss-cn-beijing.aliyuncs.com
root: atlas-dsh-desktop/
  bootstrap/windows-x64.json
  installers/<installer-version>/windows-x64/<installer>.msi
  releases/<release-version>/windows-x64/manifest.json
  releases/<release-version>/windows-x64/<component>.zip
  third-party/<component>/windows-x64/<sha256>/<upstream-file>
```

Bootstrap 是唯一可变指针。Manifest 至少包含 schema、release、platform、arch、minimumLauncher、组件 ID/版本、精确 object key、归档规则、安装根、字节数、SHA-256、来源/签名要求、doctor 命令与超时、许可证引用。对象键不可由文件名猜测，不使用目录 List、npm registry 或 GitHub Release 作为运行期回退源。

所有组件在冻结候选前于 Windows x64 构建：`node-pty`、`koffi`、`sharp` 等原生依赖必须在目标 Node ABI 上加载通过；ripgrep 执行 `--version`；dsh 执行 `--version` 并完成一次随机端口启动 doctor。用户机器不运行编译器和 `npm install`。

`src-tauri/resources/runtime-versions.windows-x64.json` 是 Windows x64 Node、dsh、pnpm、ripgrep 上游版本、对象 URL 和适用 SHA-256 的唯一所有者；`runtime/package-lock.json` 固定 dsh 的完整 npm 依赖图。`prepare:runtime` 仅在构建机执行：下载并校验 Node/ripgrep，使用 private Node 对锁文件执行 `npm ci`，再物化 bridge、doctor、许可证通知和版本元数据。它接受已取得的 Node/ripgrep ZIP 作为本地构建输入，但仍要求同一固定 SHA-256；用户安装后的 Launcher 不读取这些构建机输入，也不从 npm、GitHub 或系统 Node 回退。

## 本地目录和状态

```text
%LOCALAPPDATA%\DSH Desktop\
  launcher\                 # Installer/Launcher 拥有
  runtimes\<release>\       # 不可变已验证组件树
  cache\                    # 按 object key + digest 复用
  staging\                  # 未激活候选
  state\current.json        # 原子替换的当前 release 指针与已校验 manifest 快照
  state\staged.json         # 已通过校验、等待用户确认的 release 指针
  state\repair.json         # forward repair 的失败阶段、版本和脱敏错误
  logs\                     # 有上限、轮转、脱敏的客户端诊断

%APPDATA%\DSH Desktop\
  dsh-home\                 # 独立 DSH_HOME，Harness 拥有
  client-settings.json      # 托盘、通知和更新偏好
```

API Key、Token、任务正文、审批理由和完整模型输出不得进入客户端日志或锁屏通知。客户端日志只保存组件、阶段、版本、退出码和脱敏错误；Harness 自身日志仍由 Harness 负责。卸载不删除 `%APPDATA%\DSH Desktop\dsh-home`。

## 启动与身份确认

1. Launcher 获取进程级互斥锁，读取内置 seed Bootstrap，再尝试读取公共 Bootstrap。
2. 按 manifest 探测当前 release；缺失时下载至私有 `.part`，限制最大字节数并支持取消。
3. 依次校验字节数、SHA-256、平台、架构和 provenance；用生产解包器拒绝绝对路径、`..`、重复项、链接和越界目标。
4. 在全新 staging 中执行全部 doctor，成功后原子写入 `current.json`；失败只清理当前 staging。
5. Rust 后端以 `CREATE_NO_WINDOW` 启动私有 Node，设置独立 `DSH_HOME`，保留 stdin 控制通道，并传入 `dsh web --port 0 --no-open`。
6. 从结构化 bridge/readiness 消息取得实际端口，再请求首页并验证 `window.__DSH_BOOT__` 等 Harness 身份信号；TCP 连通本身不算 ready。
7. 动态创建唯一 WebView 窗口并直接加载验证后的精确 origin，随后持续监控拥有的子进程；不得附着未知的 3080 或其他回环服务。

## 进程与退出生命周期

每个 Harness 根进程启动即加入带 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 Job Object。Job Object 负责 Launcher 崩溃和最终强退的进程树回收，不作为日常退出手段。

最小 Cordis desktop bridge 是无持久状态的 managed-process lease 控制面，只接受 Launcher 私有 stdin 上的 NDJSON，请求和响应通过固定 sentinel 与普通 dsh 日志分离。每条消息最多 64 KiB，包含 `protocolVersion`、`requestId` 和固定 operation；响应只返回结构化状态或稳定错误，不接受路径、URL、命令、凭据或项目身份。

| Operation | 语义 | 边界 |
|---|---|---|
| `status` | 返回 bridge 版本、是否接收新工作和活动工作数 | 只读、幂等、2 秒超时 |
| `beginDrain` | 停止接收新工作并返回剩余活动数 | 幂等、可重复查询 |
| `appExit` | 调用 Harness `ctx.appExit(0)`，等待 flush/dispose | 仅 drain 后调用、10 秒响应上限 |

未知 operation、协议不兼容、非法消息、超时和 bridge 不可用分别返回稳定错误；Rust 后端限制控制输出和 dsh stdout/stderr 的内存及落盘大小。bridge 不创建网络监听、文件、子进程或新的权限边界，其协议版本和 minimum Launcher 进入 release manifest。

- 关闭窗口且存在活动任务：隐藏到托盘，不停止服务。
- 关闭窗口且无活动任务：执行正常退出。
- 用户显式退出：停止接收新任务，以原生 TaskDialog 显示等待状态，等待活动任务归零后请求 `appExit`。
- drain 超时：原生 TaskDialog 提供“继续等待”和明确的“强制退出”；只有后者关闭 Job Object。
- runtime 异常退出：隐藏或销毁 WebView，以原生 TaskDialog 显示诊断和恢复动作；短时间自动重试有界次数，但不得把端口被占用误判为恢复成功。

bridge 只为桌面进程退出和更新激活提供活动工作数与 drain，不订阅、不解析也不镜像任务正文、模型配置、审批、提问、会话或其他业务事件。这些状态及其全部交互继续由 dsh 页面拥有。

## 更新和恢复

Launcher 启动时以及 Native Host 运行期间约每六小时检查一次。后台只下载、校验、doctor 和暂存兼容 runtime；不创建 Service 或计划任务，不自动激活，不强关活动任务。

托盘中的单一更新菜单项按 `检查更新 -> 下载/暂存 -> 等待任务 -> 确认重启 -> 激活` 变换意图，进度、确认和失败由原生 TaskDialog/系统通知表达，不在 dsh 页面增加控件。激活前重新校验 manifest、digest、Launcher 兼容性和活动任务数，原子切换 `current.json` 后启动新 runtime，完成 Harness 身份检查并确认 dsh 页面可加载。

恢复模式固定为 forward repair：激活前保留旧版本目录以便诊断，但持久数据一旦由新版本接管，旧版本不得自动作为回滚目标。新版本启动失败时保留用户数据和失败记录，通过原生对话框进入修复/重新准备状态。Launcher 或 Bootstrap schema 不兼容时以原生对话框报告 `setup-required`，由用户运行新 MSI；不得由运行中的宿主替换自身。

## 实施顺序与变更边界

1. 建立无 frontend bundle 的 Tauri 2 原生宿主：配置 `windows: []`、`withGlobalTauri: false`、单实例、托盘和原生 TaskDialog，并以构建门禁证明没有页面入口或 remote Tauri IPC。
2. 建立 release manifest 类型、OSS 精确读取、下载限制、SHA-256、安全解包、doctor 与版本指针；用合成归档覆盖失败路径。
3. 构建固定 Windows x64 runtime 闭包和 desktop bridge，冻结 Node ABI 与原生模块；禁止用户机器 npm 安装。
4. 实现随机端口启动、Harness 身份确认、隐藏进程、Job Object、健康监控和有界重启；只有 ready 后才动态创建直接加载 dsh URL 的 WebView。
5. 实现原生托盘、TaskDialog、活动任务 drain、显式退出与强退边界，再接入更新暂存和确认激活；不向 dsh 页面添加桌面端交互。
6. 使用 Tauri 官方 WiX/MSI 构建链和项目级 WiX 模板生成当前用户薄安装器，加入 seed Bootstrap、WebView2 探测、许可证、修复和卸载边界；只产出本地 Development 候选，不发布。生成 MSI 若注册为 per-machine 或要求提升权限，立即阻断打包，不并行增加第二套安装器。

不得照搬参考项目的系统 Node 复用、Node 最新版解析、`@latest`、固定 3080、TCP-only ready、`taskkill /T /F` 日常退出、完整 stdout 日志或 GitHub 二进制发布。

## 编码约束与停止条件

- 编码前读取本项目 ProductContract、CurrentDesign、上游目标 dsh 版本的真实 CLI/退出契约及目标 Tauri 2 API；实际源码路径与本文设想冲突时先修正唯一设计所有者。
- 只新增 DSH Desktop 原生宿主、runtime 构建工具和定向测试；不得新增任何页面源码或前端构建工具，不得修改上游 DeepSeek Harness 页面和公共接口，除非用户另行授权并枚举消费者。
- 任何密钥、OSS 写凭据、API Key、日志或生成制品不得进入 Git。运行客户端不持有 OSS 写权限。
- 如果固定 dsh 版本无法加载目标原生模块、无法提供可验证 readiness/退出桥、WiX 不能形成可验证的 per-user MSI、OSS 前缀冲突，或实现必须扩大到 macOS/Linux/公开生产签名，则停止实施并报告证据。

## Development 完成标准

- manifest 解析、版本兼容、大小/digest、路径穿越、重复项、取消、staging 清理和原子 current 指针均有定向测试。
- 私有 Node 与固定 dsh closure 在目标 ABI 上完成有界机器可读 `--version`、原生模块加载、ripgrep、bridge 协议和随机端口启动 release doctor；日常启动只运行缓存完整性与轻量 availability doctor。
- bridge 定向测试覆盖原始 stdin 字节、未知 operation、协议版本、消息上限、超时、幂等 drain、非零退出、输出上限、Launcher 退出和完整进程树清理。
- 启动命令包含 `--port 0 --no-open`；Tauri 配置为 `windows: []`、`withGlobalTauri: false`，仓库不存在产品页面入口或前端 bundle，启动阶段不创建 WebView。
- WebView 只在 Harness 身份验证后创建并直接加载 dsh URL；加载文档具有 dsh 的 `window.__DSH_BOOT__`，不存在桌面端自有 DOM、样式、脚本或注入，远程页面调用 Tauri API 被拒绝。
- runtime 未 ready 或异常退出时不显示 WebView 兜底页；下载、doctor、更新、退出和故障动作均可从 Windows Installer、原生 TaskDialog、托盘或系统通知完成。
- Windows 后台过程不闪终端；Launcher 正常退出和崩溃后均无其拥有的残留进程。
- 活动任务时关闭窗口不终止任务；显式退出完成 drain；强退必须来自单独用户意图。
- 更新失败不改变 current 或用户数据；兼容候选可暂存，激活需用户确认；不兼容 Launcher 显示 `setup-required`。
- 当前用户 MSI 的安装、修复和卸载边界可由构建产物检查证明，Installer 字节数、首启下载量和最终占用分别报告。
- 执行范围匹配的格式、静态检查、Rust 单元/组件测试、manifest/bridge/归档 verifier、WebView 宿主定向测试与本地打包 smoke；不得建立前端测试套件。独立干净机候选验收和 OSS 正式发布分别留给用户另行建立的 SystemTest 与 Deployment。
