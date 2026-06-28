# FTB Translater

FTB Translater 是一个用于汉化现代 FTB Quests 任务文本的小型桌面工具，翻译接口使用 DeepSeek。

## 功能范围

- 目前只支持 `en_us -> zh_cn`。
- 支持新版 FTB Quests 语言文件：`config/ftbquests/quests/lang/en_us.snbt`。
- 支持章节式 FTB Quests 文件：`config/ftbquests/quests/chapters/*.snbt`。
- `lang` 模式会写入或覆盖 `lang/zh_cn.snbt`。
- `chapters` 模式会原地改写 `chapters/*.snbt` 中可翻译的文本字段。
- 写入前会自动备份已有的 `lang` 或 `chapters` 目录。
- 翻译缓存、报告和备份会保存在任务目录下的 `.ftb-translater/`。

## 安装

```powershell
python -m pip install -e .
```

## 运行

```powershell
python main.py
```

选择整合包目录即可，也可以选择它下面的 `config`、`config/ftbquests`、
`config/ftbquests/quests`，甚至直接选择 `lang` 或 `chapters` 目录。程序会自动定位
FTB Quests 任务目录。

在界面左侧进入“设置”，填写并保存 DeepSeek API Key。Key 会以明文保存到项目根目录的 `.env`：

```text
DEEPSEEK_API_KEY=your_key_here
```

也可以复制 `.env.example` 为 `.env`，然后手动填写 Key。

设置面板还可以配置：

- DeepSeek API 地址：`DEEPSEEK_BASE_URL`
- 模型名：`DEEPSEEK_MODEL`
- 翻译风格：`FTB_TRANSLATER_STYLE`
- 批大小：`FTB_TRANSLATER_BATCH_SIZE`，填 `auto` 使用自动策略
- 并发数：`FTB_TRANSLATER_CONCURRENCY`，填 `auto` 使用自动策略

## 翻译策略

程序默认会自动切分翻译请求，并根据当前任务规模选择一个保守的 DeepSeek 并发数；也可以在设置面板手动覆盖批大小和并发数。
翻译时，日志区域会显示 DeepSeek 调用、批次进度、备份创建和覆盖写入目标。

如需手动指定并发数，可以直接在设置面板填写，也可以用环境变量：

```powershell
$env:FTB_TRANSLATER_CONCURRENCY=6
```

如需恢复自动调节，可以取消该环境变量，或设置为：

```powershell
$env:FTB_TRANSLATER_CONCURRENCY=auto
```

如果 DeepSeek 返回的译文丢失受保护的格式内容，该条译文会被丢弃，并保留原文。当前保护内容包括：

- FTB / Minecraft 格式码，例如 `&e`、`&r`、`§a`
- 占位符，例如 `%s`、`%1$s`
- 物品或标签 token，例如 `<item:minecraft:stone>`、`#forge:ingots/iron`
- 字面转义序列，例如 `\n`、`\t`
- 实际换行和制表符数量

## 输出文件

翻译完成后会写入：

- `config/ftbquests/quests/lang/zh_cn.snbt`，或改写 `chapters/*.snbt`
- `config/ftbquests/quests/.ftb-translater/cache.json`
- `config/ftbquests/quests/.ftb-translater/report-latest.json`
- `config/ftbquests/quests/.ftb-translater/backups/YYYYMMDD-HHMMSS/`

写入前程序会弹窗确认，并先创建备份。`lang` 模式会覆盖 `lang/zh_cn.snbt`；
`chapters` 模式会改写章节文件里的匹配文本字段。

## 测试

普通测试组：

```powershell
python -m tests.run_groups unit
```

完整流程测试组：

```powershell
python -m tests.run_groups e2e
```

完整流程测试组会自动启用真实 live 测试开关。它会下载真实 CurseForge 整合包 zip，解压到临时目录，
定位 `config/ftbquests/quests`，先用假翻译器跑一遍真实下载处理流程，再抽样调用真实 DeepSeek API
跑一遍付费端到端流程。真实 DeepSeek 测试的最终文件会保存在：

```text
.ftb-translater/e2e-runs/YYYYMMDD-HHMMSS/
```

其中 `summary.txt` 会列出 `zh_cn.snbt`、`report-latest.json`、`cache.json` 和备份目录的完整路径。

如需指定测试用整合包，建议使用 CurseForge / ForgeCDN 的直接 `.zip` 链接：

```powershell
$env:FTB_TRANSLATER_CURSEFORGE_URL="https://edge.forgecdn.net/files/1234/567/your-pack.zip"
$env:FTB_TRANSLATER_LIVE_MAX_MB=500
```

调整真实 DeepSeek 测试的抽样条目数：

```powershell
$env:FTB_TRANSLATER_LIVE_DEEPSEEK_ENTRIES=20
```

安装为可执行脚本后，也可以使用：

```powershell
ftb-test
ftb-test-e2e
```

## 打包

本地打包 Windows 版本：

```powershell
python -m pip install -e .
python -m pip install pyinstaller
python -m PyInstaller --noconfirm --clean --onefile --windowed --name FTB-Translater --collect-data customtkinter main.py
```

输出文件：

```text
dist/FTB-Translater.exe
```

仓库也提供了 GitHub Actions 跨平台打包流水线：`.github/workflows/build.yml`。

- push 到 `main` / `master` 或发起 PR 时，会在 Windows、macOS 上运行单元测试并打包。
- Windows 产物：`FTB-Translater-windows.exe`
- macOS 产物：`FTB-Translater-macos.dmg`
- 推送 `v*` 格式的 tag，例如 `v0.1.0`，会自动把双平台产物作为裸文件上传到 GitHub Release。
- 普通 Actions artifact 下载会被 GitHub 自动包装成 zip，所以流水线不上传 workflow artifact，只发布 Release 附件。
