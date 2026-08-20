# 构建与验收 DSH Desktop

本页用于在 Windows x64 上构建当前用户 MSI，并验证安装、启动和卸载边界。首次准备 WiX 工具链需要联网；构建不会发布 OSS，也不会生成严格离线包。

## 首次准备

1. 在仓库根目录打开 PowerShell。
2. 确认已安装 Node.js、npm、Rust stable 和 `x86_64-pc-windows-gnu` 工具链。
3. 安装锁定的 Node 依赖：

   ```powershell
   npm ci
   ```

## 构建并检查

运行 MSI 构建：

```powershell
npm run build:msi
```

成功标志是生成 `src-tauri\target\x86_64-pc-windows-gnu\release\bundle\msi\DSH Desktop_0.1.0_x64_en-US.msi`。脚本会准备 WiX 3.14.1，构建前不需要手动安装 WiX。

修改 Rust 后运行白盒检查：

```powershell
cargo +stable-x86_64-pc-windows-gnu fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo +stable-x86_64-pc-windows-gnu test --lib --manifest-path src-tauri/Cargo.toml
cargo +stable-x86_64-pc-windows-gnu clippy --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run check:docs
```

## 安装启动

使用刚生成的 MSI 静默安装并记录日志：

```powershell
$msi = (Resolve-Path 'src-tauri\target\x86_64-pc-windows-gnu\release\bundle\msi\DSH Desktop_0.1.0_x64_en-US.msi').Path
$log = Join-Path $env:TEMP 'dsh-desktop-install.log'
Start-Process -FilePath "$env:WINDIR\System32\msiexec.exe" -ArgumentList "/i `"$msi`" /qn /norestart /l*v `"$log`"" -Wait
```

检查安装文件、HKCU 状态和快捷方式：

```powershell
Get-ChildItem "$env:LOCALAPPDATA\DSH Desktop" -Force
Get-ItemProperty -Path (Join-Path 'HKCU:' 'Software\atlas\DSH Desktop')
Get-Item "$env:USERPROFILE\Desktop\DSH Desktop.lnk"
```

启动必须使用已安装的 `dsh-desktop.exe`。没有可用的公共 runtime 制品时，预期看到原生 `DSH Desktop` 错误对话框，不会打开自有 Web 页面；有 runtime 时，唯一 WebView 内容应是本次 `dsh web` 输出的完整页面。

## 卸载边界

从当前用户卸载并保留 Harness 用户数据：

```powershell
$product = Get-ItemProperty -Path (Join-Path 'HKLM:' 'Software\Microsoft\Windows\CurrentVersion\Uninstall\*') | Where-Object DisplayName -eq 'DSH Desktop'
Start-Process -FilePath "$env:WINDIR\System32\msiexec.exe" -ArgumentList "/x $($product.PSChildName) /qn /norestart" -Wait
Test-Path "$env:LOCALAPPDATA\DSH Desktop\dsh-desktop.exe"
Test-Path "$env:APPDATA\DSH Desktop\dsh-home"
```

预期第一项为 `False`，第二项为 `True`。安装日志、MSI 和本地 runtime 不提交 Git；本地构建也不代表 OSS 发布或 Deployment。
