# 参与贡献

感谢你改进 FTB Translater。提交改动前，请先阅读 `README.md`、`AGENTS.md`，以及与改动相关的 `docs/` 规范；实现行为和文档必须保持一致。

## 开发环境

需要 Node.js 22、npm、稳定版 Rust，以及 Tauri 2 在当前平台所需的系统依赖。安装依赖并运行桌面开发环境：

```bash
npm ci
npm run tauri -- dev
```

请从最新的 `master` 创建目标单一、命名清晰的分支。避免在功能或修复提交中混入无关格式化、依赖升级或大规模整理。

## 改动原则

- 保持现有翻译、CMP 校对、格式守卫、备份和多文件回滚语义；安全检查不得建立旁路。
- 新增提供商或配置项时，必须保持网页、DeepL API 和 OpenAI 兼容模式各自的能力边界。
- Tauri capability 使用最小权限。不要增加任意 Shell 执行、任意文件系统访问或无范围限制的插件权限；确需新增权限时，在 Pull Request 中说明调用点、范围和风险。
- WebView 的 CSP 应保持默认拒绝和最少来源。前端不直接连接翻译服务；提供商请求由 Rust 后端发出。
- 不提交真实整合包的私人内容、生成的日志、诊断 ZIP、缓存、构建产物或本机配置。

## API Key 与敏感信息

API Key 按提供商保存在操作系统凭证管理器中，不得写入 `settings.json`、源码、测试夹具、CMP、历史数据库、项目文档或提交信息。代码只能在用户明确查看/修改凭证，或 API 翻译实际需要凭证时按需访问钥匙串，并应在当前应用会话中复用已读取的值。

前端和后端日志都不得记录 API Key、Authorization、密码、令牌、完整请求/响应正文或完整待翻译文本。新增结构化日志字段时，必须验证后端脱敏和截断仍然生效。测试请使用明显无效的占位值，例如 `test-key-not-secret`；若凭证意外进入提交历史，请立即撤销并轮换。

## 本地验证

提交前至少运行：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run build
git diff --check
```

Rust 改动还应运行 `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`。匿名网页接口的 live smoke test 依赖第三方网络，默认不作为本地测试；只有在明确需要时才运行，并区分第三方限流与本地失败。

涉及 Tauri CSP 或 capability 时，还要用 `npm run tauri -- dev` 手工确认 React 页面和本地资源正常加载、目录/文件对话框可用、翻译事件可接收、IPC 命令可调用。真实提供商请求由 Rust 后端发出，不应通过放宽 WebView `connect-src` 来解决。

## Pull Request

Pull Request 请保持范围小且可审查，并说明：

- 用户可见行为是否变化；
- 修改的模块和关键设计取舍；
- 已执行的测试及结果；
- 安全、兼容性和回滚风险；
- 尚未解决的问题。

格式或用户流程变化必须同步更新相关测试、README 和规范文档。不要把格式守卫通过率表述为语义翻译准确率。
