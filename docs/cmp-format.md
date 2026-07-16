# FTB Translator CMP v1 格式规范

CMP 是 FTB Translator 面向玩家和校对者的翻译工程文件。它把“调用翻译接口”和“覆盖任务书”拆成两个独立阶段，使机器译文可以先查看、导出、手工修改，再由应用验证并写回。

## 用户流程

1. 应用扫描 `lang/en_us.snbt` 或 `chapters/*.snbt`，按源文件列出待翻译条目。
2. 应用调用所选提供商，完成 token 恢复、JSON 富文本重建和格式守卫检查。
3. 应用把所有可翻译单元写入 `.ftb-translator/reviews/translation-<时间戳>.cmp`。此时不创建备份，也不修改任务书。
4. 用户可以选择直接应用，也可以打开、另存或在其他编辑器中修改 CMP。
5. 应用 CMP 时重新读取当前任务书，验证整包指纹、文件归属、条目 ID、JSON Pointer、英文原文和中文格式。
6. 全部验证通过后才创建备份、生成完整输出并提交；任一输出失败时回滚本次已写文件。

应用重新启动后，可以先扫描同一个任务书目录，再通过“选择已有 CMP”继续校对，无需重新调用翻译接口。

## 文件示例

```text
# FTB Translator CMP v1
# 只修改箭头右侧的中文；保留 @ 行、英文原文、引号与 JSON 转义。
# meta {"version":1,"task_id":"20260714T120000.000Z-0001","quests_dir":"/pack/config/ftbquests/quests","mode":"chapters","source_fingerprint":"...","provider":"google_web","base_url":"https://translate.googleapis.com","model":"google-web","style":"自然玩家向简体中文汉化","glossary_enabled":false,"glossary_fingerprint":"","total_entries":2,"cache_hits":0}

## file "chapters/example.snbt"

@ {"file":"chapters/example.snbt","entry_id":"example.snbt:0:description","path":"$","status":"translated"}
"Open guide" -> "打开指南"

@ {"file":"chapters/example.snbt","entry_id":"example.snbt:1:description","path":"/extra/0/text","status":"review"}
"Press E to continue\nDo not close the menu" -> "按 E 继续\n不要关闭菜单"
```

## 可编辑范围

用户只能修改 `->` 右侧的 JSON 字符串内容：

```text
"English source" -> "可修改的中文译文"
```

以下内容必须保持不变：

- 第一行版本文件头；
- `# meta` 元数据；
- `## file` 文件分组；
- `@` 行中的 `file`、`entry_id`、`path` 和 `status`；
- 箭头左侧的英文原文；
- 两侧 JSON 字符串的双引号和合法转义。

换行必须写成 `\n`，引号写成 `\"`，反斜杠写成 `\\`。因为左右两侧按 JSON 字符串解析，所以原文或译文本身包含 ` -> ` 不会与中间分隔符冲突。

CMP 语法层允许 JSON 空字符串 `""`，以便兼容旧 v1 文件并完整覆盖字符串边界情况；但应用任务书时，空译文仍会被格式守卫拒绝。普通 Unicode 字符直接使用 UTF-8 保存。

## 文档结构与字段规则

除空行和普通 `#` 注释外，文档按以下结构解析：

1. 第一行必须是 `# FTB Translator CMP v1`（允许文件开头有 UTF-8 BOM）；更名前的 `# FTB Translater CMP v1` 仅作为兼容输入接受，重新保存时统一写入新文件头；
2. 全文必须且只能有一个 `# meta ` JSON 对象；
3. 每个翻译单元必须先有一个 `@ ` JSON 对象，再有一行 `"source" -> "target"`；
4. 文档至少包含一个翻译单元；同一 `entry_id` 与 `path` 组合不得重复；
5. `## file "..."` 是文件分组标题。解析器兼容没有分组标题的早期 v1 文本；标题存在时必须是 JSON 字符串，且其后的 `@.file` 必须与该标题一致。

`# meta` 与 `@` 是受保护结构。表中标为必需的字段缺失、类型错误或出现未定义字段时，解析器会拒绝文件。这样可以避免拼写错误被静默忽略，也保证读取后重新写出不会丢失未知的受保护数据。未来如需在 v1 增加字段，必须先将其定义为明确的可选字段并同步更新解析器；不兼容结构变化必须提升版本号。普通 `#` 注释不承载受保护数据，可以添加且读取时会忽略。

## 元数据

`# meta` 后是单行 JSON 对象：

| 固定序号 | 字段 | 要求 | 含义 |
|---:|---|---|---|
| 1 | `version` | 必需 | CMP 格式版本；当前固定为 `1` |
| 2 | `task_id` | 可选 | 串联 API 请求、CMP 操作、写回与历史日志；早期 v1 文件缺少时按空字符串读取，应用时为本次操作生成新编号 |
| 3 | `quests_dir` | 必需 | 生成 CMP 时的任务书目录 |
| 4 | `mode` | 必需 | `lang` 或 `chapters` |
| 5 | `source_fingerprint` | 必需 | 按条目 ID 和完整英文原文计算的 SHA-256 指纹 |
| 6 | `provider` | 必需 | 生成机器译文的提供商 |
| 7 | `base_url` | 必需 | 非敏感接口地址，用于缓存与历史归属 |
| 8 | `model` | 必需 | 模型或网页翻译标识 |
| 9 | `style` | 必需 | 当次翻译要求 |
| 10 | `glossary_enabled` | 必需 | 当次是否启用本地词表 |
| 11 | `glossary_fingerprint` | 必需 | 当次词表内容指纹；不包含词表正文 |
| 12 | `total_entries` | 必需 | 原始任务书条目数，不是富文本拆分后的单元数 |
| 13 | `cache_hits` | 必需 | 当次 API 阶段的完整条目缓存命中数 |

应用写出时，元数据始终使用表中的字段顺序。除 `task_id` 外，CMP v1 当前没有其他可选元数据字段。

CMP 禁止包含 API Key、Authorization、钥匙串内容或完整 HTTP 请求/响应。

## 回填位置

每个翻译单元前必须有一行 `@` JSON：

| 固定序号 | 字段 | 要求 | 含义 |
|---:|---|---|---|
| 1 | `file` | 必需 | 相对于任务书目录的源文件，例如 `lang/en_us.snbt` |
| 2 | `entry_id` | 必需 | 稳定条目标识；章节模式包含文件名、字段序号和字段名 |
| 3 | `path` | 必需 | `$` 表示普通字符串；JSON Pointer 表示富文本中的展示字段 |
| 4 | `status` | 必需 | 机器翻译阶段的状态 |

`@` 对象没有可选字段。应用写出时固定使用表中顺序。

状态值：

- `translated`：机器译文通过格式守卫；
- `unchanged`：接口返回内容与英文原文相同；
- `rate_limited`：接口在重试后仍返回限流，可从应用内单独重试这一批；
- `request_failed`：非限流类接口请求失败；
- `format_guard`：占位符恢复或格式守卫失败；
- `review`：其他需要人工确认的内容，右侧通常保留英文；
- `fallback`：早期 v1 文件的兼容状态，按需要人工确认处理。

`status` 只用于提示校对优先级和选择限流重试范围，不能绕过应用阶段的重新验证。除 `translated` 外的条目若仍保持英文，应用完成后的报告会继续标记人工检查。未知状态会被拒绝，已有 v1 文件中的 `fallback` 仍可读取。

重试 `rate_limited` 条目前，应用必须重新核对 CMP 的任务书目录、模式、源内容指纹、条目数量、回填位置、文件归属和英文原文。重试只更新原 CMP 中带该状态的位置，并保留其他译文和人工修改。

## 应用时的拒绝条件

出现以下任一情况时，不创建备份、不写入任何任务书文件：

- CMP 版本、文件头或 JSON 语法无效；
- CMP 不属于当前扫描的任务书目录或模式；
- 当前任务书的条目数量或 SHA-256 指纹已经变化；
- 翻译单元缺失、重复或包含未知回填位置；
- `file`、`entry_id`、`path` 或英文原文被修改；
- 中文译文为空；
- 换行、格式码、占位符、选择器、数字、URL、资源 ID 或标签发生不允许的变化；
- 富文本重建后的 JSON 键、类型或非展示字段发生变化；
- 最终 SNBT 无法解析，或章节文件的引号、转义和括号结构异常。

验证通过后，应用先在 `.ftb-translator/backups/<时间戳>/` 创建原文件备份，再提交全部输出。多文件提交失败时恢复本次已经尝试写入的文件。

## 兼容性规则

- 写入器只生成 `# FTB Translator CMP v1`；解析器同时接受更名前的旧拼写文件头。未来不兼容变化必须提升版本号。
- 旧 v1 缺少 `task_id` 时仍可读取；其余必需字段缺失会被拒绝。
- v1 允许增加普通 `#` 注释，但 `# meta` 和 `@` 的未知字段会被拒绝，不能改变翻译对照行的语义。
- 写出时按 `file` 的字典序生成 `## file` 分组，同一文件内保持原翻译单元顺序；元数据与 `@` 字段按上表顺序生成。规范写出的 CMP 再次读取和写出时，数据与字节内容都保持稳定。
- 不要把内部的 `translation-units-latest.jsonl` 当作 CMP。JSONL 仅用于诊断实际接口调用，CMP 才是人工校对和导入导出格式。
- 修改格式时必须同步更新 `src-tauri/src/cmp.rs` 的 parse-write-parse 往返、特殊字符、重复位置和破损对照测试，以及 README 和本文档。
