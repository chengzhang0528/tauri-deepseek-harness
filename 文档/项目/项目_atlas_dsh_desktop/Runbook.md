# DSH Desktop 运行与发布 Runbook

Status: Active
Kind: Runbook
Scope: atlas-dsh-desktop / Windows 当前用户本机操作与 GitHub Release
Owner: 项目维护者
Updated: 2026-09-17
Depends On:
- ProductContract.md
- CurrentDesign.md

## 适用范围

在已获授权的 Windows x64 本机安装验证中使用。源码根执行命令。安装输入为当前源码 `npm run build:msi` 生成的 0.2.0 MSI；联网用于官方 Node/npm 准备。禁止将本机验证解释为远端发布。业务数据位于 `%APPDATA%\DSH Desktop\dsh-home`，安装升级保持该目录和 `client-settings.json`。

升级旧版 0.1.x 前须另存 `state` 和 `runtimes`：旧版卸载动作可能在升级时清除私有缓存；安装后先恢复原 current 及对应副本，再启动。0.2.0 已将清理限制为非升级卸载。业务数据不受此缓存清理影响。

## 构建与安装

先确认没有正在使用的 DSH Desktop 运行；已有运行通过“文件 → 退出”，无法确认任务完成时由用户决定是否强制退出。不得用安装替换掩盖未完成的工作。

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

应出现原生准备提示，再打开官方 Harness 页面。窗口标题显示本轮 Harness 版本，原生“帮助 / 故障处理 / 诊断信息”分列 Launcher、运行、启用、待确认和偏好。已有 current 时继续原版；后台准备不自动激活 staged。

选择“帮助 → 检查更新”，应看到当前运行版本、检查范围与结果。有新版本时点“下载更新”，已下载时点“重启并更新”；没有更新时不应出现安装按钮。重启前说明任务可能中断，选择取消保留运行与下载，确认后等待全树结束再安装并打开新版本。来源失败可重新检查；固定版本限制和本机范围分别解释。通过“帮助 → 更新设置”查看来源与版本的当前选中项，可恢复自动选择最新版本；取消不改设置。

## 恢复

- 窗口关闭后运行保留，从托盘“显示主窗口”重开。
- 窗口呈现失败而运行仍在，使用“视图 → 显示主窗口”，不重新安装副本。
- 进程树已结束后，从“帮助 → 故障处理 → 启动或修复当前版本”启动同版，使用相同 `dsh-home`；仍存活或状态未知时禁止另开运行。
- 私有副本缺失时同版官方 npm 前向重建。旧 OSS 副本不可重建时选择明确官方版本；不回滚用户数据。
- npm 准备失败可重试，本机详情在 `%LOCALAPPDATA%\DSH Desktop\cache\npm\_logs`；不复制其中凭据或业务内容。
- Launcher 替换走 MSI；WebView2 前置由安装器处理，模型或业务认证由 Harness 自己处理。

## GitHub Release

只有 Active DeploymentPlan 固定 Artifact、Tag、公开 GitHub Release Target、当前用户授权、准入证据或明确豁免及回滚后，才执行本节。发布前确认主工作区无不明改动、`origin/main` 包含 Artifact、远端没有同名 Tag，且 `version.json`、Tag 与 `scripts/check-version.mjs` 一致。

```powershell
$artifactCommit = '<DeploymentPlan Artifact>'
$releaseVersion = '<DeploymentPlan version>'
$releaseTag = "v$releaseVersion"
node --test scripts/release-tag.test.mjs
node scripts/check-version.mjs --release-version $releaseVersion
git tag -a $releaseTag $artifactCommit -m "DSH Desktop $releaseVersion"
git push origin "refs/tags/$releaseTag"
```

Tag 推送触发 `.github/workflows/release.yml`。等待 `Build Windows MSI` 终态；只接纳 head SHA 等于 Artifact 的成功运行。公开 Release 必须匹配 Tag、不是 Draft 或 prerelease，并且只有预期 Windows x64 MSI。匿名下载该 MSI，记录文件名、字节数和 SHA-256；不得在同一 Tag 下重建或替换附件。

若工作流或公开下载检查失败，先在 GitHub Releases 中删除失败的同名 Release，再删除远端和本地 Tag；上一稳定 Release 保持可下载。删除前再次核对同名 Release 与 Tag 确实属于当前 DeploymentPlan。

```powershell
git push origin --delete $releaseTag
git tag -d $releaseTag
```

