# FTB Translater

用于汉化现代 FTB Quests 任务文本的纯 Rust + Tauri 桌面工具。目前固定支持 `en_us → zh_cn`，翻译接口兼容 DeepSeek Chat Completions API。

## 功能

- 支持 `config/ftbquests/quests/lang/en_us.snbt`
- 支持 `config/ftbquests/quests/chapters/*.snbt`
- 自动识别整合包、config、quests、lang 或 chapters 目录
- 写入前自动备份原始 `lang` 或 `chapters` 目录
- 自动批处理与并发 DeepSeek 请求
- 保护颜色码、占位符、资源 ID、宏、URL、转义序列和换行
- 格式检查失败时保留原文，支持在结果页人工修正
- 翻译缓存、JSON 报告、SQLite 历史与 ZIP 导出
- API Key 存入 macOS 钥匙串、Windows 凭据管理器或 Linux Secret Service
- 浅色/深色主题和响应式桌面布局

项目不依赖 Python，也不会在运行时启动 sidecar 或外部解释器。

## 开发环境

- Node.js 20+
- Rust stable
- Tauri 2 对应的系统依赖

安装依赖：

```bash
npm install
```

启动开发版：

```bash
npm run tauri dev
```

## 测试与构建

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri -- build
```

构建产物位于 `src-tauri/target/release/bundle/`。

## 数据位置

应用设置和 `history.sqlite3` 保存到系统应用数据目录。每个任务书自己的运行数据保存在：

```text
config/ftbquests/quests/.ftb-translater/
├── cache.json
├── report-latest.json
└── backups/YYYYMMDD-HHMMSS/
```

## 翻译安全

程序会先将 Minecraft 格式码和资源标识替换成不可翻译占位符，再提交模型。译文返回后恢复原始 token，并比较控制字符与 token 集合。发现缺失、增加或变化时，该条译文不会写入，源文本会被保留并进入人工检查列表。
