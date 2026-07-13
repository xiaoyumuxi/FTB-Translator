# FTB Translater

FTB Translater 是一个用于汉化现代 FTB Quests 任务文本的桌面工具，支持 OpenAI 兼容接口、DeepL 官方 API，以及无需 API Key 的实验性网页翻译接口。目前固定支持 `en_us -> zh_cn`。

## 功能

- 支持新版语言文件：`config/ftbquests/quests/lang/en_us.snbt`
- 支持章节式任务文件：`config/ftbquests/quests/chapters/*.snbt`
- `lang` 模式写入或覆盖 `lang/zh_cn.snbt`
- `chapters` 模式原地改写 `chapters/*.snbt` 中可翻译文本字段
- 写入前自动备份 `lang` 或 `chapters` 目录
- 生成翻译缓存、报告、备份和可导出的翻译历史

## 安装

需要 Python 3.11 或更高版本。

```powershell
python -m pip install -e .
```

也可以使用 uv：

```powershell
uv sync --dev
```

## 运行

```powershell
python main.py
```

安装为可执行脚本后也可以运行：

```powershell
ftb-translater
```

启动后选择整合包目录即可。也可以直接选择它下面的 `config`、`config/ftbquests`、`config/ftbquests/quests`、`lang` 或 `chapters` 目录，程序会自动定位 FTB Quests 任务目录。

## 配置

右上角“设置”里可以选择翻译提供商、填写对应 API Key 和翻译参数。

API Key 优先保存到系统凭证管理器：

- macOS：钥匙串
- Windows：凭据管理器
- Linux：Secret Service

如果系统凭证后端不可用，程序会回退到本地受限文件，避免把 API Key 明文写入 `.env`。旧版本 `.env` 里的 `DEEPSEEK_API_KEY` 会在启动时尝试迁移到新存储。

设置面板还可以配置：

- 翻译提供商：`FTB_TRANSLATER_PROVIDER`，可选 `openai_compatible`、`deepl`、`google_web` 或 `deepl_web`
- API 地址：`DEEPSEEK_BASE_URL`（沿用旧配置名以保持兼容）
- 模型名：`DEEPSEEK_MODEL`（DeepL 模式保持 `deepl` 即可）
- 翻译风格：`FTB_TRANSLATER_STYLE`
- 批大小：`FTB_TRANSLATER_BATCH_SIZE`，填 `auto` 使用自动策略
- 并发数：`FTB_TRANSLATER_CONCURRENCY`，填 `auto` 使用自动策略

手动指定并发数也可以使用环境变量：

```powershell
$env:FTB_TRANSLATER_CONCURRENCY=6
```

恢复自动调节：

```powershell
$env:FTB_TRANSLATER_CONCURRENCY=auto
```

## 翻译流程

1. 选择整合包或 FTB Quests 目录。
2. 点击扫描，确认程序识别到 `lang` 或 `chapters` 模式。
3. 点击开始汉化。
4. 程序会先弹窗确认覆盖写入，然后创建备份。
5. 翻译完成后可以查看日志、报告和历史记录。

程序会自动切分翻译请求，并根据任务规模选择保守的并发数。翻译时日志区域会显示 API 调用、批次进度、备份创建和覆盖写入目标。

OpenAI 兼容模式默认使用 DeepSeek，也可以填写 OpenAI、OpenRouter、硅基流动或中转服务的 Base URL 与模型名。如果服务不支持 `response_format=json_object`，程序会自动回退到仅通过提示词约束 JSON，并兼容 Markdown 代码块包裹的 JSON 返回值。

DeepL 模式默认使用 Free API 地址 `https://api-free.deepl.com`；Pro 账号可改为 `https://api.deepl.com`。DeepL 不使用翻译风格和模型参数。

Google 和 DeepL 网页翻译模式不需要 API Key。它们调用的是网站或浏览器扩展使用的匿名接口，不属于官方稳定 API，因此程序会强制使用低并发、失败重试和本地缓存。Google 会用不可翻译的批次标记在一次 POST 中装入约 4500 字符，DeepL 会按匿名端点限制在一次请求中装入约 1500 字符。服务端限流、鉴权规则或接口格式随时可能变化；调用失败时程序会保留原文，不应将网页模式视为有可用性保证的服务。

如果翻译 API 返回的译文丢失受保护格式，该条译文会被丢弃并保留原文。当前保护内容包括：

- FTB / Minecraft 格式码，例如 `&e`、`&r`、`§a`
- 占位符，例如 `%s`、`%1$s`
- 物品或标签 token，例如 `<item:minecraft:stone>`、`#forge:ingots/iron`
- 字面转义序列，例如 `\n`、`\t`
- 实际换行和制表符数量

## 输出

翻译会写入或更新：

- `config/ftbquests/quests/lang/zh_cn.snbt`，或 `config/ftbquests/quests/chapters/*.snbt`
- `config/ftbquests/quests/.ftb-translater/cache.json`
- `config/ftbquests/quests/.ftb-translater/report-latest.json`
- `config/ftbquests/quests/.ftb-translater/backups/YYYYMMDD-HHMMSS/`
- 当前运行目录下的 `history.sqlite3`

`report-latest.json` 包含本次翻译摘要、失败项、格式告警和中英映射。完整输出文件内容保存在历史数据库中，用于后续导出。

## 翻译历史

右上角“历史”入口会列出已保存的翻译记录。每条记录包含整合包路径、模式、模型、条目数、失败数、告警数和缓存命中数。

历史数据库保存在当前运行目录的 `history.sqlite3`。从源码目录执行 `python main.py` 时，数据库会出现在项目根目录；从其他目录启动时，数据库会出现在对应的当前工作目录。

历史页面支持导出 ZIP：

- `lang` 模式导出 `lang/zh_cn.snbt`
- `chapters` 模式导出 `chapters/*.snbt`
- ZIP 内附带 `manifest.json`

## 测试

普通测试组：

```powershell
python -m tests.run_groups unit
```

或直接运行完整普通测试发现：

```powershell
uv run python -m unittest discover -s tests -p "test_*.py"
```

完整流程测试组：

```powershell
python -m tests.run_groups e2e
```

完整流程测试会启用真实 live 测试开关。它会下载真实 CurseForge 整合包 zip，先用假翻译器跑一遍真实下载处理流程，再抽样调用真实 DeepSeek API 跑一遍付费端到端流程。

真实 DeepSeek 测试的输出目录：

```text
.ftb-translater/e2e-runs/YYYYMMDD-HHMMSS/
```

其中 `summary.txt` 会列出 `zh_cn.snbt`、`report-latest.json`、`cache.json` 和备份目录的完整路径。

指定测试用整合包：

```powershell
$env:FTB_TRANSLATER_CURSEFORGE_URL="https://edge.forgecdn.net/files/1234/567/your-pack.zip"
$env:FTB_TRANSLATER_LIVE_MAX_MB=500
```

调整真实 DeepSeek 测试抽样条目数：

```powershell
$env:FTB_TRANSLATER_LIVE_DEEPSEEK_ENTRIES=20
```

安装为可执行脚本后也可以使用：

```powershell
ftb-test
ftb-test-e2e
```

## CI/CD

GitHub Actions 工作流在 `.github/workflows/build.yml`：

- PR：在 Ubuntu、Windows、macOS 上运行本地单元测试。
- 推送到 `main` / `master`：先运行测试，再构建 Windows exe 和 macOS dmg，并上传为 workflow artifacts。
- 推送 `v*` tag：构建成功后自动把安装包发布到 GitHub Release。
- 手动触发：可以勾选 `run_live_e2e` 运行真实 CurseForge 下载和 DeepSeek 端到端测试。

发布版本示例：

```powershell
git tag v0.1.1
git push origin v0.1.1
```

手动 live e2e 需要在仓库 Secrets 中配置：

- `DEEPSEEK_API_KEY`：真实 DeepSeek API Key。未配置时 DeepSeek 付费测试会跳过。
- `FTB_TRANSLATER_CURSEFORGE_URL`：可选，直接指向 CurseForge / ForgeCDN `.zip` 文件；未配置时使用测试默认整合包。

本地 CI 同款检查：

```powershell
uv sync --dev --locked
uv run python -m tests.run_groups unit
```
