# DSH Desktop 本机安装与恢复

Status: Active
Kind: Runbook
Scope: atlas-dsh-desktop / Windows 当前用户本机操作
Owner: 项目维护者
Updated: 2026-09-16
Depends On:
- ProductContract.md
- CurrentDesign.md

## 适用范围

在已获授权的 Windows x64 本机安装验证中使用。源码根执行命令。安装输入为当前源码 `npm run build:msi` 生成的 0.2.0 MSI；联网用于官方 Node/npm 准备。禁止将本机验证解释为远端发布。业务数据位于 `%APPDATA%\DSH Desktop\dsh-home`，安装升级保持该目录和 `client-settings.json`。

升级旧版 0.1.x 前须另存 `state` 和 `runtimes`：旧版卸载动作可能在升级时清除私有缓存；安装后先恢复原 current 及对应副本，再启动。0.2.0 已将清理限制为非升级卸载。业务数据不受此缓存清理影响。

## 构建与安装

先确认没有正在使用的 DSH Desktop 运行；已有运行通过原生菜单退出，退出证据未知时由用户明确选择强制退出。不得用安装替换掩盖未完成的工作。

```powershell
npm run build:msi
$installMsi = (Resolve-Path 'src-tauri\target\x86_64-pc-windows-gnu\release\bundle\msi\DSH Desktop_0.2.0_x64_en-US.msi').Path
$installLog = Join-Path $env:TEMP 'dsh-desktop-0.2.0-install.log'
$installer = Start-Process -FilePath "$env:WINDIR\System32\msiexec.exe" -ArgumentList "/i `"$installMsi`" /qn /norestart /l*v `"$installLog`"" -WindowStyle Hidden -PassThru -Wait
$installer.ExitCode
```

退出码 0 为安装完成，3010 表示需要重启；其他值停止并检查本机安装日志。不得提交该日志。检查 `%LOCALAPPDATA%\DSH Desktop\dsh-desktop.exe` 的文件版本、HKCU `Software\atlas\DSH Desktop` 与桌面快捷方式；安装不应要求管理员权限。

## 启动与确认

```powershell
Start-Process -FilePath "$env:LOCALAPPDATA\DSH Desktop\dsh-desktop.exe"
```

应出现原生准备提示，再打开官方 Harness 页面。窗口标题显示本轮 Harness 版本，原生“版本与更新 / 查看版本与状态”分列 Launcher、运行、启用、待确认和偏好。已有 current 时继续原版；后台准备不自动激活 staged。

检查更新失败应显示缺口；菜单“下载已选择目标”准备具体目标，“确认切换已准备目标”确认特定副本。当前官方接口没有完整退出证据，运行保留时计划切换会停止；需要用户明确强制停止后，才能确认切换。强制停止等待 Job Object 全树结束，不宣称正常收尾。

## 恢复

- 窗口关闭后运行保留，从托盘“显示主窗口”重开。
- 窗口呈现失败而运行仍在，使用“重新打开或同版恢复”，不重新安装副本。
- 进程树已结束可同版恢复，使用相同 `dsh-home`；仍存活或状态未知时禁止另开运行。
- 私有副本缺失时同版官方 npm 前向重建。旧 OSS 副本不可重建时选择明确官方版本；不回滚用户数据。
- npm 准备失败可重试，本机详情在 `%LOCALAPPDATA%\DSH Desktop\cache\npm\_logs`；不复制其中凭据或业务内容。
- Launcher 替换走 MSI；WebView2 前置由安装器处理，模型或业务认证由 Harness 自己处理。

