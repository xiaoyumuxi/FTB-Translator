# ADR-003：从 Python 桌面实现迁移到 Rust + Tauri

- 状态：接受并已实施
- 决策日期：2026-07-12
- 历史依据：`708114e`、`477dafb`、`e095d03`

## 背景

项目最初是 Python 应用，包含桌面界面、解析、提供商、备份、缓存、历史和测试。随后仓库在 `708114e` 重写为 Rust 后端、Tauri 2 桌面壳与 React/TypeScript 前端，并删除 Python 包、入口和依赖锁；后续合并保留了 Python 分支的历史，以便追溯 token span 和颜色守卫等设计。

## 原始问题

桌面发布需要同时处理解释器/依赖分发、跨平台系统集成、凭证管理、网络请求、文件写回和前端交互。继续把 Python 作为运行时 sidecar 会增加安装包结构、进程生命周期、IPC、诊断和版本一致性的维护面。

## 考虑过的方案

1. 保留纯 Python GUI 和打包工具。
2. 使用 Tauri 前端，但把现有 Python 作为 sidecar，通过 IPC 调用。
3. 把运行时业务全部迁到 Rust，React 只负责界面和 Tauri command/event。
4. 同时长期维护 Python 与 Rust 两套实现。

## 最终选择

采用方案 3。当前运行时没有 Python、sidecar 或外部解释器；Rust 是解析、翻译编排、凭证、日志、持久化和写回的后端，React/Tauri 是桌面界面与命令桥接。Python 提交只作为历史设计资料，不是可执行兼容层。

## 选择原因

- 单一桌面进程和编译产物减少运行时依赖与 IPC 故障面。
- Tauri 提供跨平台窗口、dialog 和 command/event 集成，React 适合实现设置、进度和 CMP 校对界面。
- Rust 的类型、所有权和明确错误传播适合文件解析与多阶段写回安全边界。
- 业务安全校验集中在后端，不依赖前端是否隐藏或禁用操作。

## 缺点

- 重写不是机械等价迁移；历史 Python 的 token walker 和颜色 AST 等能力可能遗漏，必须逐项审计。
- Rust/Tauri 构建依赖更多系统工具，跨平台安装包仍需分别验证。
- Rust 模块数量和强类型 command 契约增加了边界维护成本；新增工作流仍需同步前后端类型和测试。
- 旧 Python 测试不能直接运行于新实现；关键行为应继续转成 Rust fixture/测试，不能仅依赖历史实现。

## 后续影响

- README 必须明确“纯 Rust、无 Python sidecar”，源码运行要求改为 Node.js、Rust 与 Tauri 系统依赖。
- 新功能不得重新引入 Python 运行时作为隐式依赖；确需外部进程必须另立 ADR。
- 迁移验收应以行为基线和 golden fixtures 为准，而不是仅比较文件名或模块数量。
- 查阅 Git 历史时要区分“曾在 Python 实现”与“当前 Rust 已具备”，避免把历史功能写成当前承诺。

相关文档：[架构概览](../architecture.md)、[当前行为基线](../current-baseline.md)、[测试策略](../testing-strategy.md)。
