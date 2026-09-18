# WorkBuddy 主题工作台 1.6.4（兼容修复候选版）

纯离线图片主题编辑器，沿用 Tauri、React 和 CodeDrobe。流程：**上传图片 → 本地推荐风格 → 开始真实预览 → 微调 → 保存或保留应用**。不生成或美化原图，不接入 AI，不改变 WorkBuddy 的功能布局。

1.6.4 增加新版首页与会话的运行时兼容层，修复正文白底遮挡背景、组件漏配色和“没有检查也判通过”的问题；同时修复后台图片解码及回退命令参数。成品主题与图片主题分别读取自己的颜色，原主题包不改写。安装包自带 Node 和 CodeDrobe，无需命令行配置。修复依据、回归与验收边界见 [1.6.4 验证说明](docs/1.6.4-validation.md)。

原有图片融合 v2、默认零模糊、草稿/不可变修订、试用与恢复流程保持不变。当前为预发布候选版，不自动覆盖安装或重启 WorkBuddy。本机 5.5.6 的试用与回退生命周期已测，但完整视觉场景验收仍待完成，不等同于全部适配通过。历史验证记录：[1.6.3](docs/1.6.3-validation.md)、[1.6.2](docs/1.6.2-validation.md)、[1.6.1](docs/1.6.1-validation.md)、[1.6.0](docs/1.6.0-validation.md)。

## 下载后使用

从 [1.6.4 预发布](https://github.com/neverforgeth/workbuddy-theme-switcher/releases/tag/v1.6.4) 下载 **Windows x64 候选安装包**，保存未提交的编辑、退出旧切换器后安装。升级保留主题与设置，建议先备份 `%LOCALAPPDATA%\WorkBuddyThemeSwitcher`。打开切换器，自动识别 WorkBuddy 路径，选择内置主题并应用；已经连上 CDP 时不需要重启。未开放 CDP 时，保存工作后在界面单独确认启动/重启即可，不需要运行命令。若电脑缺少 WebView2，安装器会调用内置引导程序安装运行环境，该首次安装步骤可能需要联网。

遇到必要组件或关键样式不匹配时会明确报错并暂停重试；“设置与诊断”提供当前场景覆盖、未检查区域、错误码和“导出脱敏诊断”。导出仅含版本、结构、静态检查项、错误码和耗时，不含聊天、账户、截图或 CSS。不要关闭验证或编辑应用安装文件来强行注入。

## 使用

1. 上传 JPG、PNG 或 WebP：不超过 10MB，至少 64×64，最多 4000 万像素。真实解码、格式匹配和尺寸校验由后端执行。
2. 导入时推荐“清透浅色 / 暖纸柔和 / 沉静深色”之一。推荐只运行一次；普通滑块调整不切换风格。
3. 左侧调整亮度、背景模糊、面板不透明度、强调色。“高级调整”默认折叠，提供背景定位、九区精调及恢复风格默认值。
4. 切换风格保留图片、名称和背景位置，其他参数恢复该风格默认值。有手动调整或从旧文档升级时先确认。
5. 点击底部“开始真实预览”并确认，临时应用到 WorkBuddy，进入 10 分钟可回退试用。上传图片本身不会改变 WorkBuddy。
6. 试用期间微调同步实机；可手动续期。“保存主题”只保存，“保存并保留”提交正式选择，“恢复之前效果”结束试用。
7. 超时、离开编辑器或关闭窗口会恢复；断连显示待恢复，崩溃后下次启动优先处理恢复记录。

真实预览读取当前 WorkBuddy 内容区，可能包含私人内容，仅留本机。不会替用户切换聊天、输入或打开菜单。需要重启时另行确认，不自动强制关闭。

画面明确区分“参数同步中 / 画面待刷新 / 当前真实效果”。旧图不能标为最新，也不能作为当前图导出。可设置对比基准、适应宽度、100% 查看及主动导出 PNG。保持 WorkBuddy 不最小化；遮挡时仅在取图期间临时允许 renderer 绘制，不抢原生窗口焦点。失败后停止显示“正在获取”，可手动刷新。

折叠的辅助样板和主题库预览是**仿真，不是实机截图**。样板保留固定结构，与应用使用同一份生成 CSS；禁止脚本和外部网络。

## 完全离线与隐私

- 已删除 AI 表单、审核弹窗、请求实现和对应 Tauri 命令；不再配置模型、上传图片或读取 Windows 中的旧 API Key。
- “设置与诊断 → 清除旧 AI 凭据”必须单独确认，只删除精确凭据项 `WorkBuddyThemeSwitcher:CustomThemeAiKey`；不枚举、读取其他凭据。该删除不可恢复。
- 旧状态文件中的普通 AI 设置仅作为兼容字段保留，不再使用。
- 截图不写入日志或主题包，只有用户主动选择导出位置才落盘。发布版不包含实机自动验收入口。
- 移除了全 DOM 上传遮挡扫描；截图只采集受限区域几何、视口及不含内容的变化序号，用于一致性检查。
- 主窗口 CSP 仅允许本地资源、必要的 Tauri 通信及隔离样板；业务不请求外网。本机 CDP 使用环回地址，禁用代理与 HTTP 重定向。
- 图片后台任务保留超时、取消与提交前校验。当前解码步骤安全结束后才响应取消。

## 离线风格与编译

`shared/offline-styles.json` 是前后端共用的风格 v2 默认参数，三套风格默认模糊均为 0。`shared/offline-styles-v1.json` 冻结旧参数。文档仍为 schema 3；新文档保存风格/分析版本 2、候选色、16×16 固定采样、明暗变化统计和手动覆盖。普通微调不重新分析图片或重新推荐风格。

`theme_compiler.rs` 分派固定版本的编译与编辑意图；`theme_fusion.rs` 负责 v2 的配色、分层校验和受限样式生成。`theme-fusion.css` / `theme-fusion-inner.css` 明确外层底色、内层透明的覆盖顺序。前端一次提交完整编辑意图，使用一个草稿序号和一个原子文件替换提交；失败保留输入并允许重试。

背景是单张清晰图片及连续同色系遮罩，不自动模糊，也不为消息或按钮使用模糊。各区域保留独立不透明度，滑块显示相对当前风格的调整百分点。按背景→画布→消息/输入框计算叠色；正文及辅助文字目标 4.5:1，聚焦指示目标 3:1，装饰边框不强制加深。实际值与修正原因可查看；裁切、未采样纹理和动态状态标为“未验证”。

缓存后的微调不重新解码图片。参数提交、实机样式同步和截图是独立阶段；截图最多每秒一次，无操作时不持续截图。

## 数据兼容与恢复

数据位于 `%LOCALAPPDATA%\WorkBuddyThemeSwitcher`：

- `studio/assets/<id>/`：应用自有原图、受控 JPEG 背景与缩略图；移动或删除导入源不影响主题。
- `studio/drafts/`、`studio/last-draft.json`：草稿与最近编辑入口。
- `studio/library/<id>/rN/`：不可变文档与主题包，`head.json` 为发布提交点。
- `studio/save-transactions/`：中断保存的恢复记录。
- `studio/trial.json`：未完成试用的原主题和自动保持恢复记录。
- `studio/trash/`、`studio/recovery-archive/`：回收的主题和人工处理的异常记录。
- `custom-themes/`：更早版本主题原包。
- `effective-themes/`：原 CSS 与匹配的兼容补丁组合后的运行时缓存，不是新主题修订。
- `apply-recovery.json`：普通应用失败／中断时恢复已知原效果的记录；恢复未验证前不清除。
- `state.json`：路径、固定主题 ID＋修订和自动保持状态；`logs/` 只记录操作元数据。

旧 schema 1/2 的 advice、九区设计及已编译 CSS 保留。读取、打开或直接应用旧修订不重新计算配色或覆盖原 CSS；适配新组件时仅在运行时附加兼容补丁。无法识别颜色契约的早期主题不猜测配色，会明确报兼容限制。普通编辑仍走原版本编译路径，只有主动采用新风格才转为 v3 草稿并保存新修订。内置主题只读。无编辑参数的早期包通过图片创建副本，原包不变。

风格 v1 不会原地转换为 v2。“创建新版融合副本”使用新草稿 ID、保留图片/名称/背景位置/手动强调色，重新生成分区参数、模糊归零；不创建正式主题也不应用。原稿与旧修订字节不变；发布失败或取消保留原最近草稿入口。新副本沿用已有保存、试用与回退流程。

1.6.0 的 v3.1 草稿在读取时一次性升级为 v3.2 背景兼容样式，保留图片、颜色和调整参数，序号递增并标记待保存。旧的正式修订和主题包不变；重新打开旧修订得到修复后的可编辑草稿，需重新预览并保存新修订。

已应用的修订不会因另存新版被自动保持替换。应用与恢复互斥，试用期间暂停普通自动保持。当前使用或回退所依赖的主题不能删除，未知外部换肤不能被悄悄覆盖。

## 模块边界

| 模块 | 职责 |
| --- | --- |
| `theme_compiler.rs`、`shared/offline-styles.json` | 离线分析、固定风格、完整编辑意图、v3 配色与编译 |
| `theme_fusion.rs`、`theme-fusion*.css` | 风格 v2 的分层配色、对比度修正及真实组件样式所有权 |
| `theme_engine.rs`、`theme-template.css`、`region_theme.rs` | 图片解码、安全基础样式、兼容旧编译路径与九区模型 |
| `theme_model.rs` | 旧配色数据类型，不含网络或凭据逻辑 |
| `theme_library.rs` | 资源、草稿、不可变修订、原子发布与回收 |
| `workbuddy_session.rs`、`live_preview.rs` | 运行快照、试用、回退、环回 CDP 与截图一致性 |
| `workbuddy_compat.rs`、`vendor/codedrobe/src/adapters/workbuddy-compat/` | 共享结构／绘制契约、两条独立颜色映射、生效 CSS 身份与普通应用恢复 |
| `lib.rs` | 既有 Windows/CodeDrobe 适配、路径发现、精确凭据清理、自动保持 |
| `use-theme-editor.ts`、`studio-api.ts` | 单队列原子提交、迟到响应隔离与输入重基 |
| `RealPreviewPanel.tsx`、`WorkBuddyThemePreview.tsx` | 主真实预览与辅助隔离样板 |

运行态集中探测约 2.5 秒一次，前端事件为主、10 秒轮询补充。保留原有单实例与应用锁；未重写连接系统。

`EffectiveTheme` 的最终 CSS／哈希用于应用、试用、同步、截图、确认与回退；不增加第二个补丁样式节点。兼容选择依据实际结构而不是仅比较版本号，旧结构不附加新版补丁。Rust 向内置 CLI 传入内部 `WORKBUDDY_CODEDROBE_TARGET_ID` 固定同一窗口，窗口消失不静默改连。该变量由程序管理，使用者无需配置。

参考其他项目的组件约束、背景/面板分层及验证理念；本轮没有复制其他项目的品牌、人物或装饰素材，也没有增加市场、同步和主题包导入。

## 开发与验证

Windows、Node.js、Rust/MSVC 和 Tauri 构建环境。安装包携带既有 Node/CodeDrobe 运行资源。

```powershell
npm ci
npm run typecheck
npm run lint
npm test
npm run test:rust
npm run check:builtin-compat
npm run test:compat
$env:STUDIO_QA_FIXTURES = Join-Path (Get-Location) '.qa/engine-fixtures.json'
npm run test:rust -- --lib export_real_engine_visual_fixtures
$env:NO_PROXY = '127.0.0.1,localhost'
# 官方 5.5.6 ASAR 仅在本机只读使用，不随仓库分发。
$env:WORKBUDDY_556_ASAR = 'C:\path\to\WorkBuddy\resources\app.asar'
npm run test:compat:visual
npm run test:e2e
npm run package:installer
```

浏览器验收仅操作合成页面和脱敏样板，不连接 WorkBuddy。实机调试工具与 `STUDIO_RUN_QA` 流程必须另获授权，不应直接运行在未保存工作的实例上。

默认构建输出 `src-tauri/target/release/bundle/nsis/WorkBuddy 主题切换器_1.6.4_x64-setup.exe`；设置了 `CARGO_TARGET_DIR` 时以该目录为准。交付包另外标记为候选版。

保留 5.2.6 旧结构路径，新增 5.5.6 新结构映射；不外推承诺其他版本。本机已验证 5.5.6 保存保留、关闭、断连、600 秒超时与崩溃恢复，并完成 1.6.1 → 1.6.4 覆盖安装的数据保留核对。完整视觉验收、全新安装与 1.6.3 升级等剩余门槛见验证说明。
