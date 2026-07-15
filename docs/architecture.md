# 架构概览

本文面向维护者，描述从 `d313958` 行为基线演进而来的当前实现。产品用法见 [README](../README.md)，CMP 文件语法见 [CMP v1 规范](cmp-format.md)，本文不重复用户操作说明。

## 系统边界

FTB Translater 是 Rust + Tauri 2 后端与 React + TypeScript 前端组成的桌面应用，运行时没有 Python sidecar。

```text
React 界面（src/{App,components,services,types}.tsx/ts）
        │ Tauri command / event
        ▼
强类型工作流命令（commands.rs / protocol.rs）
        │
        ├── 核心 façade（core.rs）
        │     ├── 扫描、保护、翻译、校对、写回（core/*.rs）
        │     ├── lang SNBT（snbt.rs）
        │     ├── chapters SNBT（chapters.rs）
        │     ├── JSON 富文本（rich_text.rs）
        │     └── CMP（cmp.rs）
        ├── 翻译提供商（providers.rs）
        ├── 本地词表（glossary.rs）
        ├── 设置、钥匙串与历史（storage.rs）
        ├── 持久任务状态（task_state.rs）
        └── 诊断日志（logging.rs）
```

前端负责工作台、设置、校对表格、历史和用户确认。后端是任务书解析、接口调用、格式保护、CMP 校验、备份与提交的唯一可信边界。前端隐藏字段或按钮不是安全校验；提供商规范化、词表校验和 CMP 写回约束都由后端再次执行。

## 核心数据模型

扫描先把所选目录解析为任务书根目录，并选择一种模式。若同一目录同时存在 `lang/en_us.snbt` 和 `chapters/*.snbt`，当前实现优先使用 `lang`。

- `lang`：`snbt.rs` 解析根 compound，值只接受字符串或字符串数组，使用有序 `Vec` 保持键顺序；写回生成完整 `lang/zh_cn.snbt` 并重新解析。
- `chapters`：`chapters.rs` 当前用受限正则寻找 `title`、`subtitle`、`description`、`text`、`name` 字段，记录原字符串字面量的字节区间、引号和序号。替换按倒序 span 完成，并在替换前后检查引号、转义和括号结构。它不是通用 SNBT 解析器。
- 普通条目进入一个 `$` 翻译单元；可安全解析的 Minecraft JSON 富文本按 JSON Pointer 拆为多个玩家可见单元；疑似富文本但无法安全解析或含重复键时整条保留英文。

翻译单元携带 `entry_id`、`path`、英文原文、保护后的文本和占位符映射。CMP 再加入源文件归属、状态与译文，使每个译文都能回到唯一位置。

## 两阶段生命周期

系统刻意把外部服务调用与本地任务书修改分开：

1. API 阶段扫描并提取内容，保护格式 token，查询缓存，调用提供商，恢复并校验结果，然后生成 CMP。该阶段可以更新任务目录内的缓存和诊断 JSONL，但不能备份或修改任务书。
2. 应用阶段重新读取 CMP 和当前任务书，验证目录、模式、源指纹、条目数、文件归属、位置、英文原文和译文格式。所有输出先在内存中生成并验证；之后才备份并提交。多文件提交失败时尝试恢复本次提交前的快照。

“直接覆盖”和“人工校对”只是在 UI 中到达应用阶段的方式不同，后端最终都调用相同的 CMP 应用路径。详细流程分别见 [翻译流水线](translation-pipeline.md) 与 [写回事务](writeback-transaction.md)。

后端另以 `created → translating → review_ready → applying → applied` 表示成功路径，操作失败进入 `failed`；未修改任务书的 apply 失败恢复为 `review_ready`。命令在启动异步任务或写回前先原子转换状态，不能只依赖前端按钮防止重复操作。

## 持久化边界

系统应用数据目录保存普通设置、可编辑默认词表、`history.sqlite3` 和 `task-state.sqlite3`。任务状态库只保存任务身份、规范化任务书路径和状态；SQLite 事务与进程内互斥共同保护转换，重启后仍能拒绝再次应用已完成 CMP。API Key 按提供商保存在系统凭证管理器，只在用户明确查看/修改或 API 模式实际翻译时读取，并在会话中缓存。

每个任务书自己的工作数据位于 `<quests>/.ftb-translater/`：

- `cache.json`：通过格式守卫的条目缓存；
- `translation-units-latest.jsonl`：最近一次实际接口调用的诊断记录；
- `reviews/*.cmp`：可见、可编辑的校对文件；
- `backups/*`：确认应用 CMP 后创建的源文件备份；
- `report-latest.json`：最近一次已应用结果报告。

前端和后端日志固定写在应用程序旁的 `logs/`，具体安全边界见 [日志规范](logging.md)。CMP、报告、日志、缓存和历史都不得保存 API Key 或 Authorization。

## 外部服务与并发

`providers.rs` 适配 Google 网页、DeepL 网页、DeepL 官方 API 和 OpenAI 兼容接口。API 模式 `auto` 批大小为 25、并发为 6，并发硬上限 12；两个网页提供商的有效并发固定收敛为 1。提供商层负责各自协议、拆分和最多三次递增等待的 HTTP 尝试，核心层负责批次并发、失败状态、恢复、格式守卫和 CMP 汇总。

缓存键包含原文和提供商身份；OpenAI 兼容模式包含模型，其他模式还包含接口地址。开启词表时，词表内容指纹参与缓存键；富文本另带处理管线版本，避免复用旧的整段 JSON 结果。

## 必须保持的不变量

1. 外部服务返回值永远不是写回授权；占位符、格式 token 和富文本结构必须在本地验证。
2. API 阶段不创建备份、不覆盖 `zh_cn.snbt`、不改章节文件。
3. 人工编辑只能改变 CMP 箭头右侧译文，不能改变源身份或回填位置。
4. 校验失败发生在备份和提交之前；多文件提交失败必须尝试回滚。
5. 写回成功后的缓存、历史或报告保存失败是非破坏性告警，不能误报为任务书提交失败。
6. 相同输入的文件排序、条目顺序、CMP 分组和指纹计算必须确定。
7. `translating`、`applying` 和 `applied` 的重复命令必须由后端拒绝；导入 CMP 不得覆盖已有 `applied` 状态。

## 已知边界与演进方向

- 当前 `chapters` 提取仍依赖受限正则，不等于完整 SNBT token walker。历史 Python 实现曾使用 token span walker；为何不应扩大为“用正则解析所有 SNBT”见 [ADR-001](decisions/001-token-span-over-regex.md)。
- 历史 Python 格式守卫有轻量颜色/样式 AST；当前 Rust 版只把颜色码当不透明 token 并比较排序后的多重集合，尚不能证明样式作用域等价。见 [ADR-002](decisions/002-colour-ast.md)。
- 文件提交是应用级补偿事务，不是文件系统原子事务；进程崩溃或回滚自身失败仍需使用已创建的备份人工恢复。
- 当前已有 Rust 单元测试、临时目录流程测试，以及使用确定性 Mock 响应的 `lang`/`chapters` 扫描→CMP→备份→写回 Golden fixture；尚无前端组件/浏览器测试，异步 HTTP provider 也未通过可注入客户端贯穿 Golden。见 [测试策略](testing-strategy.md)。

## 决策索引

- [ADR-001：用 token/span 解析边界，而不是正则解析全部 SNBT](decisions/001-token-span-over-regex.md)
- [ADR-002：颜色码需要轻量样式 AST，而不只是数量比较](decisions/002-colour-ast.md)
- [ADR-003：从 Python 桌面实现迁移到 Rust + Tauri](decisions/003-python-to-tauri.md)
- [ADR-004：以 CMP 作为翻译与写回之间的中间格式](decisions/004-cmp-intermediate-format.md)
- [ADR-005：限流项重试前重新验证任务书一致性](decisions/005-rate-limit-retry-consistency.md)
