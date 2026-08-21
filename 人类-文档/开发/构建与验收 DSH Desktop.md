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

成功标志是生成 `src-tauri\target\x86_64-pc-windows-gnu\release\bundle\msi\DSH Desktop_0.1.1_x64_en-US.msi`。脚本会准备 WiX 3.14.1，构建前不需要手动安装 WiX。

## 按版本 Tag 构建

在 GitHub 网页将 `main` 的已验证提交创建为 `v0.1.1` 后，发布工作流会调用 `release:tag`。MSI/Launcher 使用 Tag 版本；runtime manifest 的 `release` 使用该 runtime 发布版本，`minimumLauncher` 默认取源码 `version.json` 中的稳定 Launcher 版本，因此 runtime-only 更新不会因为版本号变化而强制重装 Launcher。只有 bridge、协议或 manifest schema 不兼容时，才显式提高该兼容下限并发布新的 MSI。需要在本机复现该构建时运行：

```powershell
npm run release:tag -- --tag v0.1.1
```

成功标志仍是生成 `DSH Desktop_0.1.1_x64_en-US.msi`；Tag 必须为 `vX.Y.Z` 格式。

修改 Rust 后运行白盒检查：

```powershell
cargo +stable-x86_64-pc-windows-gnu fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo +stable-x86_64-pc-windows-gnu test --lib --manifest-path src-tauri/Cargo.toml
cargo +stable-x86_64-pc-windows-gnu clippy --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run check:docs
npm run test:bridge
```

## 构建 runtime 闭包

runtime 闭包必须已经包含固定 Node、dsh、ripgrep、desktop bridge 和所有目标机器需要的原生模块；用户机器不执行 `npm install`。先将闭包放在当前用户临时目录，再运行 doctor：

```powershell
$runtimeRoot = Join-Path $env:TEMP 'dsh-runtime-0.1.1'
$runtimeWork = Join-Path $env:TEMP 'dsh-runtime-0.1.1-work'
$runtimeOutput = Join-Path $env:TEMP 'dsh-runtime-build'
npm run prepare:runtime -- --runtime-root $runtimeRoot --work-dir $runtimeWork
node scripts/doctor-runtime.mjs --root $runtimeRoot
```

doctor 必须同时通过 `node --version`、`rg --version`、`dsh --version`、随机端口 `dsh web --port 0 --no-open`、Harness bootstrap 身份检查和 bridge drain。通过后生成本地 manifest、bootstrap 和 ZIP：

```powershell
node scripts/build-runtime.mjs --runtime-root $runtimeRoot --release 0.1.1 --minimum-launcher-version 0.1.1 --output-dir $runtimeOutput
```

发布新 runtime 时，发布工作流会先读取并校验现有 OSS Bootstrap 指向的 immutable catalog，再将其路径传给 `--catalog-input`；本地首次 Development 构建可省略该参数，脚本会生成只包含当前 release 的 catalog。输出目录会同时包含 `releases/<release>/windows-x64/` 下的 runtime ZIP/manifest 和 `catalog/<release>/windows-x64/catalog.json`。客户端不会查询 npm，也不会把 npm 上游发布直接视为更新；只有进入已校验 runtime closure 并登记到 catalog 的版本才可能被选择。

构建机无法连接上游 ripgrep 下载地址时，先从固定上游取得 `ripgrep-15.2.0-x86_64-pc-windows-msvc.zip`，再将本地路径传给 `--ripgrep-zip`。脚本仍会校验 `runtime-versions.windows-x64.json` 中的 SHA-256；不匹配的文件不会进入 closure。

命令会报告 ZIP 字节数和 SHA-256；`$runtimeOutput` 只是本地 Development 制品目录，不会上传 OSS，也不会自动改写仓库内的 seed 文件。将生成的 manifest、catalog、bootstrap 和 ZIP 按 [产品契约](../../文档/项目/项目_atlas_dsh_desktop/ProductContract.md) 的 `atlas-dsh-desktop/` 前缀发布，需要另行授权 Deployment。Bootstrap 仍保留顶层 manifest，供旧 Launcher 回退；新 Launcher 从 catalog 选择最高兼容版本，不降级。

## 安装启动

使用刚生成的 MSI 静默安装并记录日志：

```powershell
$msi = (Resolve-Path 'src-tauri\target\x86_64-pc-windows-gnu\release\bundle\msi\DSH Desktop_0.1.1_x64_en-US.msi').Path
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
$product = @(
    Get-ItemProperty -Path (Join-Path 'HKCU:' 'Software\Microsoft\Windows\CurrentVersion\Uninstall\*') -ErrorAction SilentlyContinue
    Get-ItemProperty -Path (Join-Path 'HKLM:' 'Software\Microsoft\Windows\CurrentVersion\Uninstall\*') -ErrorAction SilentlyContinue
) | Where-Object DisplayName -eq 'DSH Desktop' | Select-Object -First 1
msiexec.exe /x "$($product.PSChildName)" /qn /norestart
Test-Path "$env:LOCALAPPDATA\DSH Desktop\dsh-desktop.exe"
Test-Path "$env:APPDATA\DSH Desktop\dsh-home"
```

预期第一项为 `False`，第二项为 `True`。安装日志、MSI 和本地 runtime 不提交 Git；本地构建也不代表 OSS 发布或 Deployment。
