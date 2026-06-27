# FTB Translater

FTB Translater is a small desktop tool for translating modern FTB Quests language files with DeepSeek.

## v1 Scope

- Supports `en_us -> zh_cn` only.
- Supports modern FTB Quests lang files: `config/ftbquests/quests/lang/en_us.snbt`.
- Supports chapter-style FTB Quests files: `config/ftbquests/quests/chapters/*.snbt`.
- Writes `lang/zh_cn.snbt` in place for lang mode.
- Rewrites translatable text fields in `chapters/*.snbt` in place for chapter mode.
- Backs up the existing `lang` or `chapters` directory before writing.
- Stores translation cache and reports under `.ftb-translater/` inside the selected quests directory.

## Install

```powershell
python -m pip install -e .
```

## Run

```powershell
python main.py
```

Pick a modpack folder, its `config` folder, `config/ftbquests`, `config/ftbquests/quests`,
or even the `lang` / `chapters` folder. The app will locate FTB Quests automatically.

Paste your DeepSeek API key in the GUI and save it. The key is stored in `.env` as plain text:

```text
DEEPSEEK_API_KEY=your_key_here
```

You can copy `.env.example` to `.env` and replace the placeholder if you prefer editing the file directly.

The app automatically chunks translation requests. There is no batch-size setting in the UI.
During translation, the log panel shows DeepSeek calls, batch progress, backup creation, and overwrite targets.

## Output

After translation, the tool writes:

- `config/ftbquests/quests/lang/zh_cn.snbt` in lang mode, or translated `chapters/*.snbt` in chapter mode
- `config/ftbquests/quests/.ftb-translater/cache.json`
- `config/ftbquests/quests/.ftb-translater/report-latest.json`
- `config/ftbquests/quests/.ftb-translater/backups/YYYYMMDD-HHMMSS/`

Before writing, the app asks for confirmation and creates a backup. Lang mode overwrites `lang/zh_cn.snbt`;
chapter mode rewrites matching text fields inside `chapters/*.snbt`.

## Tests

```powershell
python -m unittest discover -s tests -v
```

Optional live CurseForge download test:

```powershell
$env:FTB_TRANSLATER_LIVE_TEST=1
python -m unittest tests.test_live_curseforge -v
```

By default this downloads a CurseForge modpack zip, extracts it into a temporary directory,
locates `config/ftbquests/quests`, and runs translation with a fake translator so DeepSeek is
not called. Override the source URL when needed; a direct CurseForge/ForgeCDN `.zip` URL is the
most reliable form for scripted tests:

```powershell
$env:FTB_TRANSLATER_CURSEFORGE_URL="https://edge.forgecdn.net/files/1234/567/your-pack.zip"
$env:FTB_TRANSLATER_LIVE_MAX_MB=500
```

Optional paid end-to-end test with the real DeepSeek API:

```powershell
$env:FTB_TRANSLATER_LIVE_DEEPSEEK=1
python -m unittest tests.test_live_deepseek_e2e -v
```

This also downloads and extracts a real CurseForge pack, then samples a small number of real
`en_us.snbt` entries before calling DeepSeek and writing outputs. Adjust the sample size with:

```powershell
$env:FTB_TRANSLATER_LIVE_DEEPSEEK_ENTRIES=20
```
