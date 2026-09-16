# 构建与验收 DSH Desktop

本页用于 Windows x64 上的本地开发构建与安装检查。独立 SystemTest 和正式 Deployment 按各自明确授权执行；创建 Release Tag 会发布 MSI，普通构建不会发布。

## 首次构建

1. 在仓库根目录打开 PowerShell，准备 Node.js/npm、Rust stable 和 `stable-x86_64-pc-windows-gnu` 工具链。首次获取 WiX 需要联网。
2. 安装构建依赖：

   ```powershell
   npm ci
   ```

3. 构建当前源码：

   ```powershell
   npm run build:msi
   ```

成功后生成 `src-tauri\target\x86_64-pc-windows-gnu\release\bundle\msi\DSH Desktop_0.2.0_x64_en-US.msi`。脚本准备 WiX 3.14.1，不需要手工安装 WiX。构建失败先查看命令输出，不使用旧 MSI 代替本次结果。

## 修改后验证

```powershell
cargo +stable-x86_64-pc-windows-gnu fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo +stable-x86_64-pc-windows-gnu test --lib --manifest-path src-tauri/Cargo.toml
cargo +stable-x86_64-pc-windows-gnu clippy --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run test:bridge
npm run test:release-tag
npm run check:docs
```

这些检查覆盖本机状态、进程树、bridge 和构建入口，不证明 Harness 业务任务或完整收尾。源码修改后需重新构建并安装才能检查新行为。

## 安装与使用

按[本机安装与恢复](../../文档/项目/项目_atlas_dsh_desktop/Runbook.md)安装刚构建的 MSI，并检查已安装文件版本、HKCU 注册和窗口。该操作修改当前用户的应用安装，保留 Harness 用户数据。

首次无运行时会直接通过官方发行方式准备私有 Node 与精确 Harness npm 版本；系统 Node 不作为产品运行时。已有 current 时保留原版启动，后台只准备 staged。看到可更新时选择“帮助 → 立即更新”，按提示完成下载和重启确认；也可从“帮助 → 检查更新”直接开始。取消重启会保留当前运行和已下载更新。

关窗保留运行，托盘可重开。当前上游没有完整退出证据，退出时会提示未完成任务的风险，确认后强制退出。进程树结束后才能恢复或切换。详细边界见[产品契约](../../文档/项目/项目_atlas_dsh_desktop/ProductContract.md)。

## 发布构建的区别

`release:tag` 将指定 Tag 版本传入同一 MSI 构建器；GitHub Release 工作流只发布自有 MSI，不再生成或上传官方依赖闭包。旧 runtime 脚本仅保留为历史工具，不在 Launcher 或 MSI 发布主路径中。远端 Tag、Release 和 OSS 写操作需要明确发布授权。
