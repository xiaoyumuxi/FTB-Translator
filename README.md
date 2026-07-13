# FTB Translater

用于汉化现代 FTB Quests 任务文本的桌面工具，基于 Rust + Tauri 构建，支持 OpenAI 兼容接口、DeepL 官方 API，以及无需 API Key 的 Google/DeepL 实验性网页翻译。固定翻译方向：`en_us → zh_cn`。

## 功能

- 支持两种模式：语言文件（`lang/en_us.snbt`）和章节文件（`chapters/*.snbt`）
- 自动识别整合包目录结构，无需手动定位文件
- 翻译前自动备份原始文件
- 官方 API 支持批量并发，网页翻译以大批次、低并发方式减少匿名请求
- 翻译缓存、JSON 报告、SQLite 历史与 ZIP 导出
- 格式安全保护 + 人工修正页（见下方原理）
- API Key 存入系统密钥管理器（macOS 钥匙串 / Windows 凭据管理器 / Linux Secret Service）
- 浅色/深色主题，响应式桌面布局
- 纯 Rust，运行时不依赖 Python 或任何 sidecar

## 工作原理

### 1. 文件解析

工具支持两种文件格式，分别对应 FTB Quests 的两种任务书结构：

- **lang 模式**：解析 `lang/en_us.snbt`，这是一个类 JSON 的 SNBT 格式文件，值可以是字符串或字符串数组（多行描述）。工具自己实现了 SNBT 解析器（`snbt.rs`），保留键的原始顺序，写出时也生成合法 SNBT。
- **chapters 模式**：遍历 `chapters/*.snbt`，从每个章节文件中提取可翻译的字符串字段（任务标题、描述等）。

### 2. Token 保护

翻译前，每条原文会经过一道**占位符替换**流程（`core.rs::protect`）：

用正则匹配以下模式，将它们替换为 `⟨P_0⟩`、`⟨P_1⟩`…… 形式的不透明占位符：

| 类型 | 示例 |
|------|------|
| Minecraft 颜色/格式码 | `&e`、`§6`、`§k` |
| printf 格式占位符 | `%s`、`%1$d` |
| 尖括号标签 | `<item:minecraft:stone>` |
| 花括号宏 | `{@player}`、`{amount}` |
| 资源/路径标识符 | `assets/mod/textures/a.png`、`kubejs:items/foo` |
| 转义序列 | `\n`、`\t`、`\\` |
| URL | `https://...` |
| 十六进制颜色 | `#FF5733` |

保护后的文本只包含自然语言和占位符，例如：

```
Use &eGold Ingot&r on <item:minecraft:gold_ingot>
→ Use ⟨P_0⟩Gold Ingot⟨P_1⟩ on ⟨P_2⟩
```

被保护的 token 列表与原文一起保存，翻译后用于恢复。

### 3. 批量并发翻译

待翻译条目按 `batch_size`（默认 25）分批。OpenAI 兼容接口和 DeepL 官方 API 可使用 `concurrency` 并发请求（默认 6，上限 12）；匿名网页接口固定低并发，通过增大单次请求减少 HTTP 调用：

- Google 网页翻译：使用不可翻译批次标记，一次 POST 尽量装入约 4500 字符，返回后按标记拆回原条目。
- DeepL 网页翻译：一次请求使用文本数组装入约 1500 字符，符合匿名端点限制。
- 超长单条文本会在标点或空白附近拆分，并避免切断 `⟨P_N⟩` 占位符。

OpenAI 兼容模式下，每批以 JSON 对象形式发送，键是条目 ID，值是保护后的文本。模型被要求：
- 保持键集合不变
- 不修改任何 `⟨P_N⟩` 占位符
- 返回同结构的 JSON 对象

所有提供商请求失败时最多重试 3 次，间隔递增。整批失败时，该批所有条目回退为原文。网页接口不是官方稳定 API，服务端限流或接口变化都可能导致暂时不可用。

### 4. Token 恢复与校验

API 返回后，每条译文经过两步处理：

**恢复**：将 `⟨P_N⟩` 替换回对应的原始 token。

**校验**（`core.rs::warnings`）：对恢复后的译文与原文做以下比较：
- 换行、回车、制表符数量是否一致
- 保护 token 集合（排序后）是否完全一致——即没有缺失、没有多余
- 如果原文是 JSON 文本组件，校验译文的 JSON 结构（除 `"text"` 字段外的键和类型）是否不变

**校验失败**：该条译文**不写入**，原文被保留，条目进入「人工修正」列表。校验通过的译文才写入文件并存入缓存。

### 5. 翻译缓存

每条成功通过校验的翻译以 SHA-256 散列为键存入 `cache.json`。散列输入包含原文、提供商标识、模型/接口和风格提示，保证不同服务之间缓存不复用；原有 OpenAI/DeepSeek 缓存保持兼容。下次翻译同一整合包时，命中缓存的条目直接跳过请求。

### 6. 写回与备份

写入前先将原始 `lang` 或 `chapters` 目录整体备份到 `.ftb-translater/backups/<时间戳>/`。

- lang 模式：写出 `lang/zh_cn.snbt`，写前再次解析验证格式合法。
- chapters 模式：按原文件路径就地修改各章节文件中的对应字段。

---

## 数据位置

应用设置和历史数据库保存到系统应用数据目录（`AppData`/`Application Support`/`~/.local/share`）。

每个任务书的运行数据保存在整合包目录内：

```
config/ftbquests/quests/.ftb-translater/
├── cache.json               # 翻译缓存（以 SHA-256 为键）
├── report-latest.json       # 最近一次运行的完整报告
└── backups/YYYYMMDD-HHMMSS/ # 翻译前的自动备份
```

---

## 开发环境

依赖：

- Node.js 20+
- Rust stable
- Tauri 2 对应的系统依赖（[参考官方文档](https://tauri.app/start/prerequisites/)）

安装与启动：

```bash
npm install
npm run tauri dev
```

## 测试与构建

```bash
# 单元测试
cargo test --manifest-path src-tauri/Cargo.toml

# 生产构建
npm run tauri -- build
```

构建产物位于 `src-tauri/target/release/bundle/`。
