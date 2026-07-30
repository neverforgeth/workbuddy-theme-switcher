# WorkBuddy 主题切换器

一个只负责选择、应用和恢复固定 CodeDrobe 主题的本地 Tauri 桌面应用。它不修改 `WorkBuddy.exe`、`resources/app.asar`、模型配置或用户数据。

## 内置主题

- `Pink · 李佳佳版`
- `Ice Blue · 冰蓝风景版`
- `山高水长 · 东方黛青版`
- `晴空海风 · 夏日天空版`

主题清单在 `themes.json`。新增主题时，复制一个 `themes/<id>/` 目录并在该清单增加一条记录即可；前端与 Rust 切换逻辑不需要修改。

每个主题目录只包含可分发资源：

```text
themes/<id>/
├─ theme.codedrobe-theme
├─ preview-safe.png
└─ preset.json
```

预览图仅使用固定背景资源，不包含聊天、项目或会话列表内容。

## 本机运行

开发构建需要 Windows、Node.js、Rust/Tauri 构建环境。发布安装包本身不要求
收件人安装 Node.js、Rust、Tauri 或 CodeDrobe Core。

已审计的本地 CodeDrobe Core 快照和专用 Node.js 运行时位于 `vendor/`，会分别
打入应用资源目录的 `codedrobe/` 与 `node/`。运行时只使用这两份内置资源，不会
依赖开发机的 `D:\view\core` 或系统 PATH。

```powershell
cd D:\view\workbuddy-theme-switcher
npm install
npm run dev
```

开发版本会检测以下 WorkBuddy 路径：已保存路径、`D:\workbuddy\WorkBuddy.exe`、常见 LocalAppData 与 Program Files 目录。找不到时可在界面中选择 `WorkBuddy.exe`；应用会验证同级 `resources\app.asar`。

## 应用流程

1. 检测 WorkBuddy 进程、版本、CDP 和 renderer。
2. 已连接 CDP 时，将主题交给本地 CodeDrobe 直接应用并 verify。
3. 未运行时，CodeDrobe 使用 `127.0.0.1:9336` 启动 WorkBuddy，并清理子进程中的 `ELECTRON_RUN_AS_NODE`、设置 `WORKBUDDY_REMOTE_DEBUGGING_PORT`。
4. 正在运行但没有 CDP 时，界面必须由用户确认后才会重启。
5. 应用后再验证主题 ID、布局验证结果和 `#codedrobe-theme-style-workbuddy` 节点数量（必须为 1）。失败时自动 restore。

切换主题时直接 apply 新主题。CodeDrobe 会先清理同一 host 的旧状态，再复用固定 style ID，因此不会叠加两个主题。

## 恢复原版

“恢复 WorkBuddy 原版”调用本地 CodeDrobe restore，并通过 CDP 检查主题节点数量为 0。重复恢复是安全的，不会删除主题包、图片或 WorkBuddy 用户数据。

## 日志与隐私

开发日志位于 `logs/`；正式构建位于 `%LOCALAPPDATA%\WorkBuddyThemeSwitcher\logs`。日志仅记录时间、WorkBuddy 路径和版本、主题 ID、apply/verify/restore 结果及错误代码。

日志不会记录聊天正文、DOM 正文、Token、Cookie、API Key、完整 CSS、背景图片二进制或 CDP 报文。

## 检查与构建

```powershell
npm run test:rust
npm run typecheck
npm run lint
npm run vite:build
npm run build
```

Debug EXE 输出为：

```text
src-tauri\target\debug\workbuddy-theme-switcher.exe
```

## 发给同事的一键安装包

```powershell
npm run package:installer
```

产物位于：

```text
src-tauri\target\release\bundle\nsis\WorkBuddy 主题切换器_1.0.1_x64-setup.exe
```

将该单个安装程序发给同事即可。安装包内置四套主题、CodeDrobe Core 与 Node.js
运行时；收件人的电脑无需预先安装开发工具。安装包会优先复用系统已有的 WebView2。
如果电脑缺少 WebView2，首次安装时需要联网下载微软运行时；因此 v1.0.1 是轻量联网版。
完全离线版仍保留在 GitHub Release 的 v1.0.0。安装完成后，
从开始菜单运行“WorkBuddy 主题切换器”，首次使用时选择本机的 `WorkBuddy.exe`。

安装程序尚未进行代码签名。Windows 可能显示“未知发布者”提示；应只通过受信任的
内部渠道分发，并在正式对外发布前配置企业代码签名证书。
