# 项目约束

- 优先使用可用的代码图工具定位符号；字符串、配置或图工具无结果时再用 `rg`。
- 成品主题与图片主题的配色、存储保持独立；只共享宿主识别、应用与验证。
- `tests/builtin-1.6.3-sha256.json` 冻结原主题资源；新版宿主补丁放在运行时兼容层，不写回原包。
- 不改 WorkBuddy 的布局、滚动、输入和点击行为；不接入 AI 或业务外网请求。
- 应用、试用、同步、截图及恢复使用相同最终 CSS 身份；内部 `WORKBUDDY_CODEDROBE_TARGET_ID` 固定目标，不改连其他窗口。
- 未知结构、缺失必要检查、未打开的动态场景不能宣称通过；恢复成功必须实际验证。
- 实机换肤、截图、重启、安装须获得对应授权；不自动打开或读取私人会话。
- `.qa/`、日志、截图、用户数据及官方 ASAR 不提交或分发；安装包仅作为 Release 附件。
- `STUDIO_QA_ROOT` / `STUDIO_RUN_QA` 仅用于获准的隔离 debug 验收，不属于生产入口。
- 完整实机与安装验收前只发布候选预发布，不标记正式 latest。

## 导航与验证

- 用户流程、模块边界、开发命令：[README.md](README.md)。
- 当前发布门槛、复现依赖与实测边界：[docs/1.6.4-validation.md](docs/1.6.4-validation.md)。历史版本说明是当时快照，不代表当前功能。
- 常规检查：`npm run typecheck`、`npm run lint`、`npm test`、`npm run test:rust`、`npm run check:builtin-compat`、`npm run test:compat`。
- 视觉回归需本机官方 ASAR（`WORKBUDDY_556_ASAR`）和引擎导出样本（`STUDIO_QA_FIXTURES`）；运行 `npm run test:compat:visual` 与 `npm run test:e2e`，缺少依赖不当作通过。
- Windows 安装包：`npm run package:installer`；发布前核对包内运行时、主题哈希和最终安装包 SHA256。
