# DSH Desktop 当前技术设计

Status: Active
Kind: CurrentDesign
Scope: atlas-dsh-desktop / Windows x64 桌面宿主与私有运行时
Owner: 项目维护者
Updated: 2026-09-16
Depends On:
- ProductContract.md

## 架构与责任

采用 Tauri 2 Native Host、当前用户薄 MSI、应用私有运行时和独立 DSH_HOME。官方依赖直接使用官方发行与安装方式；宿主管理本机副本、受管运行、版本呈现、目标确认和恢复。产品要求及领域增量采纳范围由 [ProductContract](ProductContract.md) 唯一持有；本机命令由 [Runbook](Runbook.md) 持有。

```text
当前用户 MSI -> Tauri Native Host
  -> 官方 npm 版本发现 / 私有 Node 与精确 npm 安装 -> current / staged
  -> 本轮受管运行 -> dsh web + 最小桌面 bridge
  -> Windows Job Object / 全树观察
  -> 原生菜单、TaskDialog、托盘
  -> 就绪后唯一 WebView，直接呈现 Harness 页面
独立 DSH_HOME -> Harness 拥有业务配置、会话和结果
```

`official.rs` 接官方元数据、Node ZIP 和 npm；`runtime.rs` 管副本及双指针；`process.rs` 管一次启动、带认证参数的端点及 bridge；`job.rs` 管所属进程集合；`host.rs` 管原生交互与唯一调度；`update_ui.rs` 按检查、已下载与设置事实生成更新结果及唯一动作；`dialogs.rs` 管原生反馈。没有自有 Web 页面、更新服务或业务镜像。

保持 `windows: []`、`withGlobalTauri: false`，启动不预建 WebView。官方页面不获得 Tauri IPC、shell、文件系统或更新权限，不注入宿主 DOM、样式、脚本或控件。应用不新增 TLS、证书、Origin/Host 或网络来源策略。

## 官方依赖与自有制品

Launcher 版本由 `version.json` 提供开发默认值，Release Tag 可提供 MSI 构建版本；Harness 版本来自官方包，不能用旧 runtime release 代替。0.2.0 Launcher 使用 schema 2 本机副本记录，并有界读取已发布的旧指针。无法解析时保留原文件，显示错误，不猜测目标。

官方 `@deepseek-ai/dsh` 包按解析后的精确版本，使用私有 Node 的 npm 安装到新私有目录。版本发现读取所配置 registry 的包版本表；固定版本缺失不换成最新版。来源失败保留检查缺口，不能报告全局最新。本机旧 OSS 副本可继续使用，旧 OSS 网络闭包不再是新官方版本的获取入口。

没有可复用私有 Node 时获取 Node 官方 Windows x64 ZIP；不修改系统 PATH 或全局 registry。npm 子进程只调整自己的 PATH，官方安装工具的既有校验仍由 npm 负责。宿主不做自有摘要、闭包等价认证、doctor 或重新打包发布门禁。路径归属、安全解压及本机文件存在性属于操作边界，不是官方内容认证。

官方 0.1.6-alpha.1 已在 Windows 私有目录通过 npm 精确安装并启动官方页面。npm 的原生脚本策略保持官方默认；此证据覆盖准备、入口与页面，不代表全部可选 Harness 能力或业务认证成功。

GitHub Release 工作流现只构建并发布自有 MSI，保留 Tag 构建器和当前用户 WiX 链。历史 `runtime-versions.windows-x64.json`、runtime 锁文件及 prepare/build/publish-runtime 脚本保留为历史工具，退出 Launcher 和 Release 主路径。不会删除或改写已发布的 OSS 对象。

## 本机状态与更新

应用自有状态仍在 `%LOCALAPPDATA%\DSH Desktop` 的 launcher、runtimes、cache、staging、state、logs 子目录。Harness 用户数据独立位于 `%APPDATA%\DSH Desktop\dsh-home`；设置文件仍在同一 roaming 根。安装、恢复和官方目标准备不迁移 CLI home、不回滚用户数据。

副本保存安装内位置、发行、Harness 版本和来源；本轮运行持有启动时副本，不随 current 或 staged 改变。current 是启用目标，staged 是已准备目标，二者都不证明存活运行。Harness 版本不可读显示未知；没有运行显示未运行。

旧指针只读取并映射真实目录；通过包元数据识别 Harness 版本，不改写旧源文件。旧固定版本不擅自翻译为 npm 版本。设置中的未知字段与来源/固定版本偏好保留，修改来源不清除固定版本。

已有 current 时启动 current，不自动激活 staged。无 current 时准备初始目标；准备失败可从恢复入口重试。副本准备在新目录进行；失败保持 current、既有 staged 和用户数据。启用目标不允许降级。修复可用副本时同版启动；文件缺失且有明确官方版本时新建同版副本再前向激活。

宿主启动后及每六小时自动检查、下载和暂存，不自动切换。手动检查固定为只读发现；没有候选不清空 staged。手动下载使用已选具体目标。所有写操作通过同一 busy 状态串行，自动检查不会重入。

切换在弹出确认前保留具体副本；确认后使用该副本，不重新选择 staged。激活 C2 时仅当 staged 仍为 C2 才清除它，C3 不受影响。current 原子写入失败保留旧指针；提交后启动失败记录 start 修复线索，不自动回滚。运行、启用、已准备与偏好在原生信息入口分别显示。

## 启动、窗口与反馈

后台根进程以 suspended + no-window 创建，先附加本轮 Job Object 再恢复执行，避免启动子进程先于归属建立。私有 Node 执行官方 `dsh web --patch ... --port 0 --no-open`，使用独立 DSH_HOME。bridge 存在 launcher 目录，不修改官方包内容。

就绪探测只消费本轮进程输出端点，保留认证 query 并使用 cookie 会话跟随官方认证跳转，检查 Harness 页面标记和 bridge 协议。认证 URL 仅存内存，不进入宿主日志或状态文件。端口连通不是身份或业务健康证明。

主窗口标题显示本轮 Harness 版本；窗口顶部仅提供文件、视图、帮助，托盘共用动作。帮助仅有一个检查更新入口，结果放在原生 TaskDialog：有目标显示下载动作，匹配已准备副本显示重启更新，固定版本、本机范围、来源缺口分别解释，无可用目标不显示安装按钮。更新设置从帮助直接打开，来源及目标版本使用带当前选中项的原生单选列表；自动版本选项可解除固定，不写新状态模型。故障处理独立分组，启动/修复与强制停止按运行状态启用，进行中的串行操作禁用重复入口。进度以对应操作标题的 TaskDialog 展示，隐藏只关闭提示；进度窗口结束后才打开结果或确认框，避免重叠。菜单不承载长状态，详情在诊断中保留。

启动成功且窗口创建/导航调用成功后报告就绪；页面实际内容通过本机安装交互验证。呈现失败单独说明进程仍运行，可重开而不重建副本。根进程消失且子树仍存在时隐藏失效窗口并保留占用；全树结束后解除占用，允许同版恢复。

## 退出证据缺口与全树结束

已核对官方 0.1.6-alpha.1：公开 agent 列表只能给局部 running 数，AppExit 没有完成回执，shutdown 有强制退出兜底；没有公开的全入口接纳、全部待完成工作或完整 dispose/flush 汇总。U1 仍未解决，宿主不能承诺完整正常退出。

bridge 协议 2 如实返回这些事实为未知，running agent 仅作为观察值。beginDrain/appExit 明确返回证据不可用，不伪造局部 draining，不调用缺少收尾证明的退出钩子。未知不按零处理。

下载更新准备完成后绑定具体副本；存活运行缺少退出证据时说明中断任务的风险，用户取消则保留运行和下载，明确选择强制重启更新后才结束全树并激活。单一退出入口在退出证据缺失时提示任务可能未完成，用户取消则保留运行，明确确认后强制停止全树再退出。关窗隐藏至托盘，运行继续。用户可另选明确强制停止/强制退出；强制终止 Job 后须观察 ActiveProcesses 为零才清除运行。强制停止不被记录为正常收尾。

Job Object 的全树结束与正常收尾彼此独立。根退出不能替代树结束；树结束即释放运行占用，不等待死进程补回执。同版恢复使用原 DSH_HOME，切换发行仍要求具体目标确认。宿主崩溃时保留 KILL_ON_JOB_CLOSE 回收，安装器替换边界仍由 ProductContract 持有。

## 开发验证边界

Rust 定向测试覆盖旧指针、错误目标保留状态和数据、路径归属、防降级、固定版本、具体确认与新 staged 并存、实际 Windows 根退出/子进程存活及整树终止。bridge 测试覆盖未知事实、零 running 不等于无工作、拒绝虚假 drain/exit。源码检查必须包含非 test 宿主路径；安装交互检查原生入口与官方页面。

这些是 Development 证据，不替代独立 SystemTest 或 Deployment，也不补齐 U1。上游将来提供权威接口时，须核对该具体发行的事实语义后接入，不能仅增加字段或 mock 声称完整退出已实现。
