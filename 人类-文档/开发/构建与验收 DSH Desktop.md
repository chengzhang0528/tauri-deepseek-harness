# 构建与验收 DSH Desktop

本页用于 Windows x64 上的本地开发构建与安装检查。独立 SystemTest 和正式 Deployment 按各自明确授权执行；创建 Release Tag 会发布 MSI，普通构建不会发布。

## 首次构建

1. 在仓库根目录打开 PowerShell，准备 Node.js 22/npm、Rust 与 `stable-x86_64-pc-windows-gnu` 工具链；本地 Node 版本与 Release 工作流一致。首次获取依赖和 WiX 需要联网。
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

依次执行 Runbook 的[启动与确认](../../文档/项目/项目_atlas_dsh_desktop/Runbook.md#启动与确认)，核对官方页面、版本和更新入口；异常时按[恢复](../../文档/项目/项目_atlas_dsh_desktop/Runbook.md#恢复)处理。退出从“文件 → 退出”操作，若提示无法确认任务完成，先完成工作或明确选择强制退出。

## 发布构建的区别

正式发布的前置条件、Tag 操作及结果检查见 Runbook 的 [GitHub Release](../../文档/项目/项目_atlas_dsh_desktop/Runbook.md#github-release)。不要将推送 Tag 当作本地构建步骤。
