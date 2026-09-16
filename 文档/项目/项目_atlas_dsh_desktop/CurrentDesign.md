# DSH Desktop 当前技术设计

Status: Active
Kind: CurrentDesign
Scope: atlas-dsh-desktop / Windows x64 桌面宿主与私有运行时
Owner: 项目维护者
Updated: 2026-09-16
Depends On:
- ProductContract.md

## 评审结论

采用现有 Tauri 2 Native Host、当前用户薄 MSI、应用私有运行时和独立 DSH_HOME。官方依赖直接使用官方发行与安装方式；宿主管理本机副本、受管运行、版本呈现、更新确认和恢复，不再承担官方内容认证与重发布前置责任。产品要求及领域增量采纳范围由 [ProductContract](ProductContract.md) 唯一持有。

本设计描述已采纳目标及与源码的差异，不声明实现完成。源码已有宿主、更新器、bridge 和构建工具；旧闭包校验路径仍在代码中，退出事实仍有缺口。官方私有目录安装的 Windows 可运行性和完整退出接口是对应实施步骤的前置证据，不阻止版本展示、准备反馈等独立改动。

| 结果/决策 | 当前支持 | 必需变化 | 用户表面 | 所有者 | 持久化影响 | 证据 / 实施条件 |
|---|---|---|---|---|---|---|
| 官方依赖接入 | runtime.rs 要求 dshDesktopRuntime、manifest、digest 和 doctor | 官方精确版本与安装方式取代自有闭包认证门槛；保留本机准备和启动结果 | 原生准备进度、失败与重试 | RuntimeManager；内容归发布者 | current/staged 格式调整 | 官方 npm 入口已确认；私有目录安装须定向验证 |
| 当前版本可见 | host.rs 固定标题；勾选来自设置；PreparedRuntime 仅带 root | 本轮运行保存启动发行和 Harness 版本引用，各入口读取同一事实 | 标题、窗口原生菜单与托盘 | Native Host | 运行信息驻内存，复用发行引用 | 不由 current、staged 或选择标记推导运行版本 |
| 更新与反馈 | 已有启动后/每六小时检查、自动暂存、手动操作 | 来源覆盖、暂存与运行分别表达，确认绑定具体目标 | 检查、准备、等待、启动与失败可辨 | RuntimeManager + Native Host | 复用设置和双指针；检查与确认不落库 | 自动止于准备；首次准备与已有版本升级分开 |
| 正常退出 | bridge 仅有 running 数和局部 draining；退出回执早于 appExit | 消费完整工作、入口关闭和收尾证据；缺失按未知 | 原生等待、缺口说明、明确强退 | Harness 生产事实，宿主消费 | 无业务镜像 | U1 未解决，现有 bridge 不证明完整退出保证 |
| 异常恢复 | try_wait 只观察根进程；Job Object 负责回收 | 补充全树结束观察；结束与正常收尾分别判断 | 同版恢复、前向修复、窗口重开 | ProcessJob / HarnessProcess / Native Host | 复用 repair，无运行历史库 | OS 树结束证据须验证，不自动回滚数据 |

## 目标架构

```text
当前用户 MSI -> Tauri Native Host
  -> 官方发行发现与私有副本准备 -> current / staged
  -> 本轮受管运行 -> 私有 Node + dsh web + 最小桌面 bridge
  -> Windows Job Object / 运行与退出观察
  -> 窗口标题、原生菜单、TaskDialog、托盘、通知
  -> 就绪后唯一 WebView，直接呈现 dsh 页面
独立 DSH_HOME -> Harness 拥有业务配置、会话和结果
```

现有模块继续复用：`runtime.rs` 管发行发现、准备及指针；`process.rs` 管一次启动及 bridge；`job.rs` 管所属进程集合；`host.rs` 管原生交互和调度；`dialogs.rs` 承载原生反馈。无需新增更新服务、规则引擎、发行数据库或插件市场。

保持 `windows: []`、`withGlobalTauri: false`，启动时不创建 WebView；受管运行就绪后加载其实际回环 URL。远程页面不获得 Tauri IPC、shell、文件系统或更新权限。宿主不注入 DOM、样式、脚本或业务控件。窗口原生菜单复用托盘动作，不另建 Web 页面。普通外部 HTTP(S) 链接沿用系统浏览器入口。应用层不新增 TLS、证书、Origin/Host、重定向或网络来源策略。

## 制品与版本契约

Installer/Launcher 的开发版本仍由 `version.json` 持有，版本 Tag `vX.Y.Z` 是对应 MSI/Launcher 发布构建输入。Harness 版本取官方发行引用，不能用 Launcher 版本或旧 runtime release 号代替；只有真实发行明确声明的适用条件才作为兼容条件，不要求官方补交本产品 manifest 或 minimumLauncher。

发行身份由发布方及发行标识限定；副本由安装上下文与受管位置限定。同版本号不同来源不自行认定内容等价，运行保存启动发行，后续指针改变不重写该事实。准备完成只表示获取/安装成功，不表示已运行或业务健康。

官方发布位置和安装方式以实际发布者为准。2026-09-16 核对的 [Harness 官方入口](https://github.com/deepseek-ai/deepseek-harness#run-from-npm)提供 npm 包运行方式；[npm install 官方说明](https://docs.npmjs.com/cli/v11/commands/npm-install/)支持安装具体包版本。这支持官方包管理器方向，但不证明目标 Windows 机器上的原生依赖无需编译或完整退出接口可用。

拟接入方式是在未被运行占用的私有目录，使用私有 Node 的 npm 安装已解析的精确 Harness 版本。实施前核对目标 Node/npm、包入口、安装作用域和脚本行为；不修改系统 PATH、全局 registry 或 CLI home，不通过浮动版本启动原地更换运行内容。官方工具自身校验照常执行，宿主不复制摘要、闭包或 doctor 认证。Node 等依赖沿用实际官方发布方式；上游没有要求的工具不因旧闭包包含它就继续捆绑。

旧 `runtime-versions.windows-x64.json`、`runtime/package-lock.json` 及脚本是现有构建机闭包输入，不是新方案的官方版本所有者。保留 GitHub Release MSI 分发责任；项目 OSS 的历史 ZIP、manifest、catalog 与 Bootstrap 是旧实现事实，不再作为官方更新前提。构建消费者未解耦前不得直接删除其输入或宣布发布链已切换。本方案不上传、删除远程对象或更换已发布包。

## 本地目录和状态

继续使用 AppPaths 的现有用户目录：

```text
%LOCALAPPDATA%\DSH Desktop\
  launcher\          # Installer/Launcher
  runtimes\          # 私有副本，运行占用时不原位重建
  cache\             # 可复用准备输入
  staging\           # 未激活的准备目录
  state\current.json # 当前启用目标，不证明有存活运行
  state\staged.json  # 已准备目标，不携带确认
  state\repair.json  # 当前失败环节、目标与脱敏原因
  logs\              # 有界、脱敏的宿主诊断
%APPDATA%\DSH Desktop\
  dsh-home\          # Harness 拥有，卸载保留
  client-settings.json
```

当前和暂存各至多一个，首次准备可均为空；副本记录预期发行、位置与本次准备结果，实际文件存在性另行观察。重建不沿用旧准备结果。保留安装内路径归属与写入边界，不因删除内容认证而允许越界或覆盖使用中的副本。

API Key、Token、任务正文、审批理由和完整模型输出不得进入客户端日志或锁屏通知；宿主诊断仅保存组件、环节、版本、退出码和脱敏错误，Harness 自身日志由 Harness 负责。

本轮运行记录启动副本、启动发行/Harness 版本、数据家目录、进程句柄、端点与就绪；PID 或端口不是运行身份。检查触发、来源覆盖、确认目标、呈现结果仅是当次状态，不新增持久任务、确认表或观察历史。来源设置 `auto` 不表示允许自动切换。

### 迁移边界

CurrentRelease/StagedRelease 当前依赖 manifest 和摘要快照，PreparedRuntime 仅有 root。消费者包括 RuntimeManager 的准备/激活、HostState 与版本菜单、HarnessProcess 启动，以及构建/发布脚本和定向测试。修改类型时一并核对，不能只改展示文案。

推荐新指针具有可识别格式，并做一次性有界转换：旧记录中确定的安装内位置、发行与来源原样映射，无法确定的 Harness 版本显示未知，来源偏好不静默改写。副本可用性通过本机准备/启动结果回答，旧摘要/doctor 不升级为新准入条件。格式不可解析时保留原记录并提示修复，成功写入前不覆盖原指针。不建设多代永久兼容框架。

旧固定版本可能是项目 runtime release，不能直接当 Harness npm 版本；无对应关系时明确反馈，由用户选有效官方目标，不擅自取最新版。旧 staged 不携带确认，转换不激活它。DSH_HOME 定位和业务内容不迁移、清空或回滚。该有界转换是推荐实现选择，需实际旧状态样例验证，不新增覆盖所有历史布局的兼容承诺。

## 启动与身份确认

1. 获取单实例所有权，解析安装路径、设置与双指针；不附着未知端口服务。
2. 有可用 current 时启动 current，不因 staged 存在而切换；首次无 current 按官方目标准备并建立初始 current。发现既有失败线索则先走恢复分流。
3. 私有 Node 在所选副本启动 `dsh web --port 0 --no-open`，设置独立 DSH_HOME，隐藏后台进程并建立本轮归属。
4. 消费本轮端点、页面标记和 bridge 可用性观察；TCP 连通不等于就绪，就绪不等于完整退出能力或业务成功。
5. 就绪后呈现唯一 WebView，以本轮 Harness 版本更新标题。窗口失败单独报告，不认定副本损坏；健康运行重开窗口不重复启动进程。

## 进程与退出生命周期

复用现有私有 stdin NDJSON、requestId、protocolVersion 和消息上限，不新增网络控制面或任意命令/路径输入。消费者是 process.rs、host.rs、desktop-bridge.mjs 与 scripts/desktop-bridge.test.mjs。

当前 beginDrain 只设局部标记，activeWork 只数 running agent，appExit 先回复再调用 ctx.appExit。它们不证明全部入口关闭、审批/输入等待结束或 flush/dispose 完成。request_window_close 把读取失败映射为零的行为须改为未知并保留运行。

目标消费 Harness 权威汇总：实际入口接纳、完整待完成工作、退出收尾结果，并绑定本轮运行。先查选定发行的真实服务/插件接口及消费者；没有证据时保留未知，不靠新增字段名或 mock 声称实现。未知时不推进正常收尾/切换，明确强退仍可用。需要修改上游业务入口才能实现时，交由上游责任项目，不在宿主复制业务排空。

ProcessJob 已有 KILL_ON_JOB_CLOSE，但没有全树结束观察。需基于本轮持有的 Job Object 提供“仍有进程 / 已结束 / 未知”，验证根退出而子进程仍存活的情况。正常完成要求入口关闭、工作归零、收尾完成且全树结束；运行关联解除只依全树结束，旧收尾未知不使已死运行永久占位。强退也须观察到全树结束才报告完成。

普通关窗有工作时隐藏到托盘，未知时保留运行并说明；明确无工作才进入正常退出。显式退出或已确认更新才请求排空，既有工作及必要输入仍可完成。超时提供继续等待和明确强退，不自动杀进程。保留 Job Object 在宿主崩溃时的回收职责；安装器替换例外沿用 ProductContract。

## 更新和恢复

保留启动后及运行期间约六小时自动调度，不增设 Service 或计划任务。自动/手动共用发行发现和准备，同一进行中准备可复用；查询来源按有效偏好确定。来源全部成功才陈述“本次所选范围内未发现合格新目标”，失败/未完成显示缺口，不能声称上游全局最新。可读渠道的明确目标仍可准备，不开展跨来源内容等价认证。

窗口原生菜单固定提供版本与更新、检查更新、关于，托盘共用动作。查看只读；手动检查不排空/切换；自动准备写 staged，但 current 和现有运行不变。运行版本、已下载目标及检查失败可同时存在，无候选响应不清空有效 staged。

确认绑定具体副本与发行，不在确认后重新读取 staged 并当作原目标。背景暂存其他目标时，原目标仍可解析且合格就继续原目标；不可用则解释停止。激活只清除与已激活目标相符的 staged。确认仅当次有效，不跨宿主重启保存。

计划更新先完成旧运行正常退出，再原子写 current，启动目标并呈现。指针提交失败保持旧指针；提交后启动失败按实际环节记录，不宣称更新成功或自动回滚。目标接管 DSH_HOME 后，旧目录存在不构成数据向后兼容证据，继续前向修复。

明确需要新 Launcher 能力时报告 `setup-required`，由用户运行新 MSI；运行中的宿主不替换自身。MSI 继续使用 Tauri/WiX 当前用户构建链，保留 WebView2 探测、许可证、修复与卸载边界；产物要求管理员提升或注册为 per-machine 时阻断打包。运行客户端不持有 OSS 写权限。

异常恢复先查全树：仍存在/未知不得另开运行；已结束则释放占用，不等待死进程补收尾回执。副本可用时同版重启；重建先确保无运行占用；换发行仍需目标确认。窗口失败先处理呈现，证实 WebView2 缺失交 Installer/Microsoft，模型认证/业务错误交 Harness。相同失败无新证据不无限重试，验证故障消除后才清理对应修复线索。

## 实施顺序与变更边界

| 顺序 | A → B 与修改边界 | 完成证据 / 前置条件 |
|---|---|---|
| 1 | 项目入口、产品契约、当前设计从旧闭包规则调整为已采纳模型 | 正式规则一致，保留实现差异，文档检查通过；文档调整不代表代码改变 |
| 2 | 核对官方精确包安装和 Harness 退出接口 | 私有 Windows x64 目录安装、入口和原生依赖证据；入口、工作、收尾接口覆盖结论。缺口仅限制依赖它的切片 |
| 3 | runtime.rs/paths.rs 的发行、副本、准备、双指针；process.rs 启动事实 | 精确目标、无认证门槛、旧状态转换、准备失败保留 current 和数据 |
| 4 | host.rs/dialogs.rs 的标题、原生入口和更新反馈 | 运行/选择/已下载版本分开，未知可见，检查保留 staged，各入口共用动作 |
| 5 | job.rs/process.rs/bridge 与 host.rs 的退出、确认、激活、恢复 | U1 齐备才实现完整正常退出；全树/收尾分离，确认目标稳定，恢复不等死进程 |
| 6 | prepare/build/publish-runtime、release-tag、build-msi 及 GitHub workflow 的旧闭包消费者 | 原有 MSI 链内解除官方更新对重发闭包的依赖，保留自有制品验证；不操作远程发布物 |

源码、类型和定向测试在明确的实现任务中修改；本设计提供切片输入，不把未验证的安装/退出能力当作已满足。目标项目以外的模型源保持引用，本项目采纳与实现责任归 ProductContract 和本设计。

## 编码约束与停止条件

- 先读项目入口、产品契约、相关模型、目标源码及测试。核对官方接口时绑定具体发行，不把浮动主分支说明当固定版本保证。
- 只改相关 Native Host、准备、bridge 适配和直接构建消费者；不改 Harness 页面、业务数据结构或上游公共接口，不增自有页面、业务镜像、服务或全局机器配置。
- 保持隐藏进程、单实例、随机端口、独立 DSH_HOME、卸载留数据与确认边界；密钥、Token、正文、日志和生成物不入 Git。
- 停止受影响切片：官方安装要求编译器/系统依赖而与产品承诺冲突；退出接口无法覆盖目标；无法辨认旧目标；必须改上游公共接口；需要新权限、秘密或外部发布。报告具体缺口，不弱化断言或重设自有认证门槛。
- 无待定产品方向；待查证的是安装可行性、实际接口与迁移样例。数据向后兼容缺口继续以 forward repair 处理，不自动增加回滚。

## Development 完成标准

方案调整以所有者一致、来源明确、实现差异与前置证据清楚及文档门禁通过为完成。下列是对应代码切片的白盒要求，不是本次方案任务已执行结果：

- 精确官方目标可在私有目录准备；无本产品 closure descriptor 不阻断发现；准备失败不改变 current/DSH_HOME。
- 自动暂存后未确认重开仍用 current；旧 runtime release 不误当 Harness 版本；新 current 启动失败显示未运行。
- 标题、关于、更新面板与托盘共用运行事实；检查失败保留 staged；查看不下载/切换；呈现失败不冒充副本损坏。
- bridge 失败按未知，审批等待完成工作不被 running 数遗漏；回执不当收尾；根退出树仍存在/未知不启动新运行；全树结束收尾未知允许按规则同版恢复。
- 确认 C2 后暂存 C3 不替换确认，激活 C2 不清 C3；准备失败、指针失败、提交后启动失败分别断言；失败不删除或回滚业务数据。
- 按边界选测试：Rust 使用 `cargo +stable-x86_64-pc-windows-gnu test --manifest-path src-tauri/Cargo.toml`；bridge 使用 `npm run test:bridge`；构建消费者按需使用 `npm run test:runtime-build`、`npm run test:release-tag`、`npm run test:runtime-publish`；正式文档使用 `npm run check:docs`。旧 doctor 测试不作为新目标成立的证明。
- 安装路径需要私有目录精确安装与启动的定向证据；原生入口/进程树需要 Windows 交互与所属子进程证据。mock 只证明消费者分支，不能补足 U1；独立候选验收及正式发布不列为实施步骤。
