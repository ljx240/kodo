# Kodo FACET Logo 系列

2026-10-12 选定方案 1「折面 K」。三个产品保留共同的 K 骨架，通过切角、折面和颜色区分。

| Kodo | Kodo Code | Kodo Work |
| --- | --- | --- |
| <img src="kodo-icons-pack/kodo-icon-primary-1024.png" width="120" alt="Kodo 应用图标" /> | <img src="kodo-icons-pack/kodo-code-icon-1024.png" width="120" alt="Kodo Code 应用图标" /> | <img src="kodo-icons-pack/kodo-work-icon-1024.png" width="120" alt="Kodo Work 应用图标" /> |
| 黑底，白色竖笔与下翼，蓝色上翼 | 蓝底，白色硬切角标识 | 紫底，白色柔和折面标识 |

## 资源与使用

- 应用图标：[`kodo-icons-pack/`](kodo-icons-pack/)，三张 1024×1024 PNG，圆角方形外部透明。适合应用入口和深浅色界面。
- 透明底标识：[`kodo-transparent-icons-pack/`](kodo-transparent-icons-pack/)，三张 1254×1254 PNG，仅含标识。竖笔使用深色，适合浅色背景。
- 前端：`apps/desktop/src/assets/kodo{,-code,-work}-mark.png` 从应用图标导出，分别为 144、128、128px。Code / Work 模式下的侧栏和欢迎页显示对应产品图标。
- favicon：`apps/desktop/public/favicon.png`，64px Kodo 主应用图标。
- 原生图标：`apps/desktop/src-tauri/icons/`，由 Kodo 主应用图标生成 ICNS、ICO，以及现有 Windows、Android、iOS PNG 尺寸；Android 和 iOS 使用主图标深色底。

旧资源路径保持兼容。现有 `icon.png.bak` 是历史备份，未替换或删除，不在构建配置中引用。

## 生成与导出

六张图像使用内置 ImageGen 生成，仅以本次生成的 FACET 预览和标识作图像参考。没有发送项目代码、配置或日志。最终图片已保存到上面的两个资源目录。

[生成提示词](generation-prompts.md) 记录了六张图片及两次局部校正。平台尺寸使用项目已安装的 Tauri CLI 导出，没有新增依赖：

```bash
cd apps/desktop
node node_modules/@tauri-apps/cli/tauri.js icon ../../docs/logos/kodo-icons-pack/manifest.json --output src-tauri/icons
```

图像是 PNG 资产，不是矢量源文件。原生图标在重新构建应用后生效；已安装应用的图标缓存可能需要重启应用才会刷新。

### 菜单栏托盘图标

`apps/desktop/src-tauri/icons/tray.png` 是 macOS 顶部状态栏用的**模板图**：单色黑剪影 + alpha（`iconAsTemplate: true`，系统按菜单栏深浅自动反色）。它是从透明底标识派生的，不是手绘的——本地重新推导：

```bash
magick docs/logos/kodo-transparent-icons-pack/kodo-icon-transparent-1254.png \
  -trim +repage -resize 36x36 -background none -gravity center -extent 44x44 \
  -channel RGB -fill black -colorize 100 +channel \
  PNG32:apps/desktop/src-tauri/icons/tray.png
```

- `-channel RGB -colorize` 只压 RGB、保留 alpha，得到纯黑剪影；
- `PNG32:` 前缀强制输出 RGBA —— Tauri 编译期嵌入图标时要求 RGBA；
- 44×44 够 Retina 用：`tray-icon` 会按 18pt 高缩放并保持宽高比。

托盘图标由 `tauri.conf.json` 的 `app.trayIcon` 在编译期嵌入，改图后需要重新构建应用。
