# FTB Translater 维护约束

本文档面向后续维护者和自动化代理。README 面向用户，必须保留项目原理、翻译模式、真实性能与准确度数据、运行方法和使用流程；实现约束集中维护在这里，避免在多份设计文档中复制后失去同步。

## 项目结构

- 当前桌面实现基于 Rust、Tauri 2、React 和 TypeScript，运行时不依赖 Python sidecar。
- 前端入口：`src/main.tsx`。
- 翻译流程与格式保护：`src-tauri/src/core.rs`。
- CMP 格式读写：`src-tauri/src/cmp.rs`；格式规范：`docs/cmp-format.md`。
- 提供商请求：`src-tauri/src/providers.rs`。
- 设置、钥匙串与历史：`src-tauri/src/storage.rs`。
- 本地词表实现：`src-tauri/src/glossary.rs`。
- 诊断日志实现：`src-tauri/src/logging.rs`。
- 内置词表模板：`src-tauri/resources/minecraft_glossary.json`。
- 翻译准确度审计：`docs/translation-accuracy-audit.md`。
- 日志维护规范：`docs/logging.md`。

## 翻译提供商能力

默认提供商是 `google_web`。已有有效用户配置必须继续按保存值加载，不能因为默认值变化而被强制迁移。

| 设置区域 | `google_web` | `deepl_web` | `deepl` | `openai_compatible` |
|---|:---:|:---:|:---:|:---:|
| 提供商选择 | 显示 | 显示 | 显示 | 显示 |
| 服务凭证 | 隐藏 | 隐藏 | DeepL Authentication Key | API Key |
| 接口地址 | 隐藏 | 隐藏 | 显示 | 显示 |
| 模型名称 | 隐藏 | 隐藏 | 隐藏 | 显示 |
| 翻译要求 | 隐藏 | 隐藏 | 隐藏 | 显示 |
| Minecraft/模组词表 | 隐藏 | 隐藏 | 显示，默认关闭 | 显示，默认关闭 |
| 每批条目与并发 | 隐藏 | 隐藏 | 显示 | 显示 |

保存按钮始终可见。不要退回到仅使用 `needsKey` 判断所有卡片的设计；需要 Key 不代表提供商支持相同的配置。

前端 `providerOptions` 中每个提供商必须显式声明：

- `credentialLabel`：是否需要以及如何命名凭证；
- `supportsGlossary`：是否显示词表；
- `supportsTaskParameters`：是否显示批大小和并发数；
- `configuration`：使用 `none`、`deepl` 或 `openai` 专属配置。

### 网页模式

- `google_web` 使用 `https://translate.googleapis.com`，单次尽量装入约 4,500 字符。
- `deepl_web` 使用 `https://oneshot-free.www.deepl.com`，按约 1,500 字符限制装批。
- 两者都不需要 Key，不显示其他配置卡片，有效并发固定上限为 1。
- 切换到网页模式时，前端关闭词表，并把 `batch_size`、`concurrency` 恢复为 `auto`。
- `storage.rs` 保存设置和 `core.rs` 启动任务时必须再次执行相同规范化，不能只依赖界面隐藏。

### DeepL 官方 API

- 默认 Free 地址为 `https://api-free.deepl.com`，必须允许 Pro 用户改为 `https://api.deepl.com`。
- 显示 Authentication Key、接口地址、本地词表、批大小和并发。
- 不显示 OpenAI 模型或翻译提示词。
- 词表是应用本地占位符保护层，不是 DeepL 官方 Glossary API。

### DeepSeek / OpenAI 兼容

- 默认地址为 `https://api.deepseek.com`，默认模型为 `deepseek-chat`。
- 显示 API Key、接口地址、模型、翻译要求、本地词表、批大小和并发。
- 必须继续允许用户填写其他 OpenAI 兼容地址和模型。

API 模式下，`batch_size=auto` 使用 25，`concurrency=auto` 使用 6，并发硬上限为 12。

## 提供商切换与保存

- 切换提供商时应用目标提供商的默认地址和模型。
- 清空表单中的 Key 和本次 Key 编辑状态，避免将一个服务的凭证带到另一个服务。
- 当前 `settings.json` 只保存正在使用的一套非敏感配置，不分别保存四套接口、模型和任务参数。
- API Key 按提供商分别保存在系统凭证管理器中，不能写入 `settings.json`、项目文件、报告或历史数据库。

## 钥匙串访问

以下操作不得访问钥匙串：

- 应用启动、加载普通设置或打开设置页；
- 切换提供商；
- 保存未修改 Key 的普通设置；
- Google/DeepL 网页翻译。

仅在以下情况按需访问钥匙串：

- 用户点击眼睛按钮明确查看已保存的 Key；
- 用户保存新 Key 或删除原 Key；
- API 模式实际开始翻译且当前会话没有对应 Key。

成功读取的 Key 必须在当前应用会话中复用，不能按批次重复读取并反复触发系统验证。

## Minecraft/模组词表

- 词表只在 `deepl` 和 `openai_compatible` 模式可见并生效，默认关闭。
- 首次运行把内置模板复制到应用数据目录，已有用户文件不得被后续启动覆盖。
- 用户可以编辑默认 JSON、输入自定义路径、选择其他文件或恢复默认路径。
- 保存设置和开始任务时都要校验 JSON、空条目与重复术语。
- 词表内容的 SHA-256 指纹参与缓存键；修改内容后不能误用旧翻译缓存。
- 不确定的模组专名优先保留英文，避免通用翻译引擎错误直译。

## CMP 校对与写回

- API 翻译阶段只能生成 `.ftb-translater/reviews/*.cmp`、接口诊断 JSONL 和缓存，不得创建备份、覆盖 `lang/zh_cn.snbt` 或修改 `chapters/*.snbt`。
- API 阶段结束后，前端必须提供“是，直接覆盖”和“否，人工校对”。两条路径最终都调用同一套 CMP 解析、格式守卫、备份和提交逻辑，不能为“直接覆盖”建立低校验旁路。
- 扫描结果按源文件列出条目数；CMP 也按 `lang/en_us.snbt` 或 `chapters/<文件名>.snbt` 分组。
- CMP v1 的人工可编辑内容仅为 `"英文" -> "中文"` 右侧 JSON 字符串。文件头、`# meta`、`## file`、`@` 回填位置和左侧英文都属于受保护内容。
- CMP 元数据可以保存非敏感的提供商、模型、接口地址、翻译要求和词表指纹；禁止保存 API Key、Authorization、钥匙串值或完整 HTTP 请求/响应。
- CMP 元数据保存非敏感 `task_id`，用于串联 API、人工校对、写回和历史日志；兼容缺少该字段的早期 v1 文件。
- 应用 CMP 时必须重新扫描当前任务书并校验目录、模式、条目数、源内容 SHA-256、文件归属、条目 ID、JSON Pointer 和左侧英文。任务书变化、条目缺失/重复或元数据被修改时拒绝写入。
- 手工译文仍要经过换行、格式码、占位符、选择器、数字、URL、资源 ID、标签和 JSON 富文本结构校验；人工编辑不能绕过格式守卫。
- 所有验证通过后才创建备份并生成完整输出。语言文件重新解析完整 SNBT；章节文件验证替换位置、引号、转义和括号结构；多文件提交失败时回滚。
- 应用重新启动后，允许用户扫描同一目录并导入已有 CMP，不得强制重新调用翻译接口。
- CMP 的语法、状态和兼容性规则以 `docs/cmp-format.md` 为准；格式变化必须同步更新解析测试、README 和该规范。

## 诊断日志

- React 前端日志写入 `frontend.log`，Rust 后端日志写入 `backend.log`，不得重新合并成单文件。
- 两份日志统一支持 `error`、`warn`、`info`、`debug`、`trace`，默认级别为 `info`；已有有效用户配置继续按保存值加载。
- 日志固定保存在应用程序所在目录的 `logs/`，不得静默回退到系统用户数据目录。目录不可写时必须向界面返回明确错误。
- 两份日志分别按 5 MB 滚动，各保留 5 份旧文件；诊断 ZIP 必须同时包含前后端日志。
- 禁止记录 API Key、Authorization、密码、令牌、完整请求体、完整响应体或完整待翻译文本。结构化上下文必须继续经过后端脱敏和截断。
- 新增关键用户流程时，需要同时考虑前端操作/异常日志与后端命令/持久化日志。详细事件覆盖、格式和扩展规则见 `docs/logging.md`。

## README 与数据口径

README 必须持续包含：

- 四种提供商的用户可见配置矩阵；
- 从源码运行、实际使用流程和构建命令；
- 批处理与并发原理；
- CMP 生成、人工校对、导入导出、确认后写回和安全拒绝条件；
- StoneBlock 4 Google 网页翻译的真实耗时、吞吐、接口成功和格式守卫数据；
- 明确区分接口成功率、格式守卫通过率和语义准确率。

不得把 99.44% 格式守卫通过率描述成翻译准确率。当前独立审计估算严格可发布准确率约为 45%–55%，详细证据保存在 `docs/translation-accuracy-audit.md`。更新基准数据时，需要记录日期、输入规模、缓存状态、提供商、批大小、有效并发、耗时、失败数和准确度口径。

## 修改后的最低验证

Rust 或设置逻辑修改：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
```

React、TypeScript 或设置页修改：

```bash
npm run build
```

文档和所有改动：

```bash
git diff --check
```

匿名网页接口的 live smoke test 默认忽略，因为它依赖网络与第三方端点。只有在明确需要验证真实服务时再单独运行，不能把第三方端点暂时限流误判为本地单元测试失败。
