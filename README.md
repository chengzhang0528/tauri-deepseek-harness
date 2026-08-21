# DSH Desktop

DSH Desktop 是 DeepSeek Harness 的非官方 Windows 桌面宿主。它负责安装和管理应用私有 runtime，启动 `dsh web`，并在桌面 WebView 中加载 Harness 自己提供的完整页面；它不新增或替代 Harness 的 Web UI。

## 下载

面向用户的 Windows x64 MSI 会在推送匹配版本 Tag 后由 GitHub Actions 自动构建，并上传到对应的 GitHub Release：

[下载最新 DSH Desktop 安装包](https://github.com/chengzhang0528/tauri-deepseek-harness/releases/latest)

打开最新 Release，下载其中的 `.msi` 文件并运行安装。支持 Windows 10/11 x64；首次启动需要联网准备私有 runtime，并需要可用的 WebView2。DSH Desktop 不内置模型或 API Key，相关配置仍由 Harness 管理。

## 开发

构建、runtime 准备、安装验收和卸载边界见[构建与验收 DSH Desktop](人类-文档/开发/构建与验收%20DSH%20Desktop.md)。产品边界和技术设计见[产品契约](文档/项目/项目_atlas_dsh_desktop/ProductContract.md)与[当前技术设计](文档/项目/项目_atlas_dsh_desktop/CurrentDesign.md)。

[全部人类文档](人类-文档/README.md)
