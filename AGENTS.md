# FTB Translater 维护约束

本文档面向后续维护者和自动化代理。README 面向用户，必须保留项目原理、翻译模式、真实性能与准确度数据、运行方法和使用流程；实现约束集中维护在这里，避免在多份设计文档中复制后失去同步。

## 项目结构

- 当前桌面实现基于 Rust、Tauri 2、React 和 TypeScript，运行时不依赖 Python sidecar。
- 前端入口：`src/main.tsx`。
- 翻译流程与格式保护：`src-tauri/src/core.rs`。
- 提供商请求：`src-tauri/src/providers.rs`。
- 设置、钥匙串与历史：`src-tauri/src/storage.rs`。
- 本地词表实现：`src-tauri/src/glossary.rs`。
- 内置词表模板：`src-tauri/resources/minecraft_glossary.json`。
- 翻译准确度审计：`docs/translation-accuracy-audit.md`。

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

## README 与数据口径

README 必须持续包含：

- 四种提供商的用户可见配置矩阵；
- 从源码运行、实际使用流程和构建命令；
- 批处理与并发原理；
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
