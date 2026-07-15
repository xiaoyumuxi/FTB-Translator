# 测试策略

测试的首要目标不是证明译文语义正确，而是固定“解析位置正确、外部返回不破坏格式、CMP 可审阅、写回前重新验证、失败可恢复”这些安全属性。语义质量评估是独立工作，见 [翻译准确度审计](translation-accuracy-audit.md)。

## 当前基线

阶段 0 在 `5ecd891` 上记录的结果为：53 个 Rust 测试中 52 个通过、1 个真实网页接口 smoke test 忽略；`npm run build` 和 `cargo check` 通过。样本与命令细节见 [当前行为基线](current-baseline.md)。

当前测试主要放在各 Rust 模块的 `#[cfg(test)]` 中，使用 `tempfile` 覆盖临时目录读写。前端目前以 TypeScript/Vite production build 作为最低检查，没有组件或浏览器自动化测试。

## 分层策略

### 1. 纯函数与格式单元测试

适合快速固定局部不变量：

- `snbt.rs`：parse/dump 往返、尾随内容拒绝；
- `chapters.rs`：提取位置、转义写回、缺失或重复位置、结构校验；
- `rich_text.rs`：只提取展示字段、非展示字段不变、重复 JSON key 拒绝；
- `cmp.rs`：写入/读取、`->` 与换行转义、缺失 `task_id` 兼容、未知状态、重复位置和破损翻译对拒绝；
- `glossary.rs`：首次复制不覆盖、内容指纹、最长匹配、边界和保留专名；
- `logging.rs`：级别、脱敏、HTTP body 清洗、轮转和 ZIP 内容；
- `storage.rs`：网页模式规范化、设置行为与历史持久化。

格式 parser/serializer 的新增边界应优先写成输入—输出明确的表驱动测试，并同时加入拒绝用例，避免只验证 happy path。

### 2. 核心流程临时目录测试

`core.rs` 当前测试跨越多个模块，覆盖目录扫描、token 保护/恢复、富文本缓存隔离、CMP 源一致性、写回前拒绝、备份唯一性和提交回滚。这一层不访问网络或系统钥匙串。

凡是修改以下代码，至少应补充或更新这一层：源指纹、`entry_id/path/file` 推导、CMP 应用、备份目录、`commit_outputs`、提交后历史/报告错误处理。

### 3. 离线 golden 流水线

目标形态是为 `lang` 与 `chapters` 保存真实结构 fixture，执行：

```text
扫描 → 提取 → Mock Provider → 生成 CMP
     → 修改右侧译文 → 应用 → 比较完整输出与备份
```

基线分支尚未提供贯穿异步 `translate` 的可注入 Mock Provider，因此当前还没有完整的 provider→CMP→writeback golden harness。后续实现时不应调用真实服务或钥匙串；建议至少覆盖多行、嵌套章节、JSON 富文本、颜色码、选择器/宏、重复键、破损 JSON、特殊转义和多文件回滚。

### 4. 前端契约与构建

`npm run build` 同时运行 `tsc` 与 Vite production build，可发现 TypeScript 类型、导入和打包错误，但不能证明页面交互正确。前端模块拆分或 Tauri payload 变化后，至少手工核对：

- 扫描结果和按文件计数；
- 四种提供商切换后可见配置；
- 网页模式清空词表并恢复自动批次/并发；
- CMP 表格保存、打开、导出、选择已有 CMP；
- “直接覆盖”和“人工校对”都进入同一写回命令；
- 限流筛选与重试只更新对应记录。

长期应把 Tauri command payload/response 固定成共享类型或契约测试，并为关键校对交互增加组件测试；本文只记录方向，不表示当前已经具备这些测试。

### 5. 桌面集成与真实服务 smoke test

`cargo check` 验证 Tauri/Rust crate 能编译；发布前的桌面 smoke test还应人工确认窗口资源、IPC、目录选择、钥匙串按需访问、日志目录和安装包启动。

匿名网页接口和付费 API 会受网络、限流、账户和第三方协议变化影响，默认测试必须保持离线。只有明确要验证真实端点时才运行 ignored smoke test，并把第三方 429/故障与本地回归分开报告。

## 改动到验证的映射

| 改动范围 | 最低自动验证 | 额外关注 |
|---|---|---|
| Rust 解析、CMP、写回、设置 | `cargo fmt --check`、`cargo test` | 对应拒绝用例、离线 fixture |
| 提供商协议或重试 | 上述 Rust 检查 | Mock 响应；真实 smoke test 仅按需 |
| React/TypeScript/设置页 | `npm run build` | 关键交互手工核对 |
| 文档 | `git diff --check` | 相对链接与代码事实核对 |
| 发布候选 | Rust 检查 + 前端构建 + Tauri 编译/打包 | 安装包桌面 smoke test |

项目约定的通用命令：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
git diff --check
```

## 不变量测试清单

解析和提取：文件排序与条目顺序确定；注释和非展示 JSON 字段不被翻译；字符串 span 不越界；输出可重新解析。

格式安全：所有 `P/G` 占位符恰好恢复一次；换行和 token 多重集合一致；富文本结构不变；疑似但无效的 JSON 不降级发送。

CMP：只允许右侧译文变化；旧 v1 可选字段兼容；未知状态、重复/缺失位置、修改英文和源变化都拒绝。

事务：所有校验先于备份；所有输出先于提交生成；第 N 个写入失败恢复之前文件；成功提交后的附属持久化失败不回滚。

重试：只重试 `rate_limited`；另一个任务书、修改后的源、变化的位置或英文在发请求前拒绝；非限流记录和人工修改保持不变。

## 当前缺口

- Rust 重写没有移植历史 Python 的轻量颜色/样式 AST；应增加相同 token 数量但作用域不同的失败测试。
- `chapters` 当前仍为受限正则提取，应为嵌套 compound/list、注释边界和相似非目标字段增加 golden fixtures，并评估迁移到 token walker。
- 没有完整离线 Mock Provider 端到端测试。
- 没有前端组件/IPC 契约自动测试。
- 回滚测试覆盖可控写入失败，但没有进程崩溃恢复或原子 rename 方案。
