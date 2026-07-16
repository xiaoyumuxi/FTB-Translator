# 当前行为基线

本文记录 `5ecd89154273091a5afb24434fd4714cfad105ca`（2026-07-15）的重构前行为。后续重构应保持这里描述的输入识别、CMP 结构、格式保护和写回结果；如果确实需要改变，应先更新测试与格式规范并说明兼容策略。

## 自动验证结果

所有命令均在 macOS、Rust stable 和 Node.js 环境中从干净 worktree 执行。匿名网页翻译 live smoke test 按项目约定保持忽略，没有访问真实翻译服务或钥匙串。

| 命令 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 通过；共发现 53 个 Rust 单元测试，52 个通过、0 个失败、1 个 live smoke test 忽略；另有 0 个二进制测试和 0 个文档测试 |
| `npm ci` | 通过；安装 74 个 package |
| `npm run build` | 通过；TypeScript 检查和 Vite production build 完成，转换 1,582 个模块 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 通过；桌面 Rust/Tauri crate 编译检查完成 |

## `lang` 模式样本

样本任务书位置为 `config/ftbquests/quests/lang/en_us.snbt`，内容为：

```snbt
{ title: "Hello" }
```

基线结果：

- 扫描模式：`lang`（语言文件）。
- 扫描文件数量：1。
- 提取条目数量：1。
- 文件明细：`lang/en_us.snbt`，1 条。
- 条目 ID：`title`；英文原文：`Hello`。
- 模拟译文：`你好`。

对应 CMP 的稳定、人工可编辑部分为：

```text
# FTB Translator CMP v1
# 只修改箭头右侧的中文；保留 @ 行、英文原文、引号与 JSON 转义。
# meta {"version":1,"task_id":"baseline-lang","quests_dir":"<sample>/config/ftbquests/quests","mode":"lang","source_fingerprint":"<sha256>","provider":"google_web","base_url":"https://translate.googleapis.com","model":"google-web","style":"自然中文","glossary_enabled":false,"glossary_fingerprint":"","total_entries":1,"cache_hits":0}

## file "lang/en_us.snbt"

@ {"file":"lang/en_us.snbt","entry_id":"title","path":"$","status":"translated"}
"Hello" -> "你好"
```

`<sample>` 和 `<sha256>` 在真实任务中由所选目录及源内容生成，不属于固定字面值；其余字段顺序和记录结构是当前序列化基线。

模拟应用该 CMP 后，写回文件为 `lang/zh_cn.snbt`，重新解析得到一个键值：

```text
title = Text("你好")
```

应用前会重新校验任务书目录、模式、源指纹、条目位置和左侧英文；把 `Hello` 改成其他文本时会拒绝，并且不会创建 `zh_cn.snbt`。成功应用报告中的 `translated_entries` 为 1。

以上行为由 `core::tests::scans_lang_pack`、`core::tests::cmp_is_validated_before_it_writes_the_language_file` 和 `core::tests::cmp_with_modified_english_never_writes_output` 固定。

## `chapters` 模式样本

样本文件 `chapters/a.snbt` 内容为：

```snbt
{
 // title: "Comment"
 title: "Hello", description: ["Line one", "第二行"]
}
```

基线结果：

- 扫描模式：`chapters`（章节文件）。
- 扫描文件数量：1。
- 提取条目数量：2。
- 注释中的 `title` 不提取。
- 没有 ASCII 英文字母的 `第二行` 不提取。
- 提取顺序为 `Hello`、`Line one`，位置索引分别为 0、1。
- 模拟译文为 `你好`、`第一行`。

对应 CMP 的记录部分为：

```text
## file "chapters/a.snbt"

@ {"file":"chapters/a.snbt","entry_id":"a.snbt:0:title","path":"$","status":"translated"}
"Hello" -> "你好"

@ {"file":"chapters/a.snbt","entry_id":"a.snbt:1:description","path":"$","status":"translated"}
"Line one" -> "第一行"
```

真实翻译任务生成的 `entry_id` 和 `path` 由当前提取流程保存并在应用时重新匹配；上面的记录展示基线样本的顺序和字段形状，不授权人工修改 `@` 行。

模拟写回结果为：

```snbt
{
 // title: "Comment"
 title: "你好", description: ["第一行", "第二行"]
}
```

写回数量为 2；原注释、未翻译中文、括号结构和原有字段顺序保持不变。替换文本中的引号、反斜杠和换行必须重新转义，缺失位置或重复位置会拒绝写回。以上提取和渲染行为由 `chapters::tests::extracts_and_replaces`、`chapters::tests::replacement_text_cannot_escape_the_original_snbt_string` 和 `chapters::tests::rejects_missing_or_duplicate_replacement_positions` 固定。

## CMP 和事务保护基线

- 新 CMP 文件头必须是 `# FTB Translator CMP v1`；旧拼写文件头仅保留读取兼容。
- 元数据使用单行 JSON，记录使用 `@` 位置行和 `"source" -> "target"` JSON 字符串行。
- `->`、换行和 JSON 转义可往返解析；旧 v1 缺少 `task_id` 时仍可读取。
- 未知状态、重复回填位置、损坏的翻译对会被拒绝。
- 多文件输出中后续写入失败时，已经写入的文件恢复原内容。
- 同一秒创建的备份使用不同目录，不能相互覆盖。
- 任务书提交成功但历史保存失败时，不能把结果误报为任务书写回失败。

## 后续重构必须保持的关键行为

1. `lang` 与 `chapters` 的目录发现、扫描计数、文件排序和条目顺序保持确定性。
2. 翻译阶段只生成 CMP、缓存和诊断数据，不直接修改任务书。
3. 应用 CMP 前必须重新验证源任务书和所有受保护字段，人工编辑只能改变右侧译文。
4. 格式码、占位符、选择器、数字、URL、资源 ID、标签和 JSON 富文本结构继续受格式守卫保护。
5. 语言文件通过完整 SNBT 解析与重新生成写回；章节文件按记录位置替换并验证引号、转义和括号结构。
6. 备份在所有验证通过后、正式提交前创建；多文件提交失败必须回滚。
7. API Key 不进入 CMP、日志、历史、报告或项目文件，网页模式不访问钥匙串。
