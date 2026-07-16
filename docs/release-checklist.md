# v0.2.0 发布检查表

本文把可自动复现的发布候选证据与必须人工完成的平台/服务验证分开。勾选表示当前候选分支已经实际执行；未勾选项不得在发布说明中声称完成。

## 1. 版本与仓库元数据

- [x] `package.json`、`package-lock.json` 根包、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 均为 `0.2.0`。
- [x] `LICENSE`、`SECURITY.md`、`CONTRIBUTING.md` 和最小 Tauri capability 已纳入仓库。
- [x] `.github/workflows/build.yml` 在 PR/master 运行质量门禁，只在 `v*` tag 或显式 workflow dispatch 时打包；没有普通分支自动发布路径。
- [x] README 保留四种提供商配置矩阵、运行/构建/使用流程、批处理并发原理、CMP 校对写回以及真实基准口径。
- [x] README 明确区分接口成功率、99.44% 格式守卫通过率和约 45%–55% 严格可发布准确率。
- [ ] 创建 tag 前再次确认工作树干净、版本与发布说明一致。

## 2. 当前候选自动质量门禁

候选分支：`release/v1-preparation`。2026-07-15 在 macOS 26.5.2 arm64 环境执行以下记录；没有运行 ignored 的真实网页 smoke test。

- [x] `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml`：主 crate 93 passed / 1 ignored；Golden 集成 96 passed / 1 ignored；0 failed。
- [x] `npm ci`
- [x] `npm run build`
- [ ] `npm run tauri -- build`：release binary 与 `.app` 成功，当前 Codex 桌面环境的 Finder/DMG 美化步骤两次退出；沙箱外重跑结果相同，不能记作普通命令无条件通过。
- [ ] 更名后重新执行 `CI=true npm run tauri -- build`，确认生成 `.app` 与最终非 `rw.*` 的 `FTB Translator_0.2.0_aarch64.dmg`。更名前曾用相同命令成功生成旧名称候选产物，但不能作为本次更名后的打包证据。
- [x] `git diff --check`（文档与候选修改后通过；提交前再复核一次）。
- [ ] 重新检查更名后的 `src-tauri/target/release/bundle/`、Mach-O 架构、DMG 大小、SHA-256 与 `hdiutil verify`；更名前记录的旧产物散列不再作为当前候选证据。

更名前的本机 `.app` 使用旧拼写 identifier；当前配置已改为 `com.openres.ftb-translator`，需要通过上面的重新打包项核验。发布候选仍只有 ad-hoc/linker 签名，没有 Developer ID 与公证，因此不能跳过第 6 节。

## 3. 离线场景与证据映射

这些测试使用临时目录和确定性数据，不访问翻译服务或系统钥匙串。

| 场景 | 自动化证据 |
|---|---|
| `lang` 扫描、Mock 响应、CMP、dry-run、备份与完整输出 | `application::golden_tests::lang_pipeline_matches_golden_files_offline` |
| 多 `chapters` 扫描、嵌套/富文本/转义、CMP 与完整输出 | `application::golden_tests::chapters_pipeline_matches_golden_files_offline` |
| Token 占位符恢复、完整着色片段移动、CMP、备份与写回 | `application::golden_tests::token_protection_provider_round_trip_applies_safe_colour_segment_move` |
| 相同颜色 token 但作用域错误时在备份前拒绝 | `application::golden_tests::token_protection_invalid_colour_scope_is_rejected_before_backup` |
| 应用内编辑只接受 `index + target` | `commands::tests::cmp_target_edit_accepts_only_index_and_target`、`core::tests::typed_cmp_target_edits_preserve_identity_and_cannot_bypass_validation` |
| 外部编辑箭头右侧后重新导入 | `cmp::tests::editing_only_the_right_hand_json_string_is_readable` |
| 修改左侧英文时拒绝且零写入 | `core::tests::cmp_with_modified_english_never_writes_output` |
| 修改 file/path 或受保护请求字段时拒绝 | `cmp::tests::rejects_location_that_does_not_match_file_group`、`core::tests::retry_validation_rejects_another_pack_or_modified_source`、`commands::tests::cmp_requests_reject_unknown_top_level_fields` |
| 限流项重试前重新核对目录、源和位置 | `core::tests::retry_validation_rejects_another_pack_or_modified_source` 与 `ADR-005` 的一致性约束 |
| 备份唯一且备份故障发生在写回前 | `core::tests::backups_created_in_the_same_second_never_overwrite_each_other`、`backup_failures_are_structured_before_writeback`、`backup_failure_after_validation_never_touches_the_task_book_output` |
| staging、第二文件失败、逆序补偿与回滚失败标记 | `temporary_file_create_failure_touches_no_outputs`、`second_output_replace_failure_restores_all_files`、`commit_failure_restores_earlier_file_from_fixture`、`rollback_failure_marks_the_task_book_as_modified` |
| 历史/报告可持久化，提交后失败只产生告警 | `storage::tests::history_roundtrip_and_export`、`history_failure_after_commit_returns_success_with_an_unavailable_run_id`、`report_failure_after_commit_returns_success_and_preserves_history` |
| 重复翻译、重复应用、UI 丢失结果、重启导入已应用 CMP | `task_state::tests::concurrent_translation_reservations_allow_only_one_for_a_task_book`、`successful_apply_is_remembered_after_ui_loss_and_restart`、`commands::tests::real_load_and_apply_commands_remember_applied_cmp_across_restart` |

自动测试能证明文件与命令边界，不等同于真实桌面点击或第三方服务可用性。

## 4. 手工桌面与安装包验证

- [ ] macOS：从 `.dmg` 安装到新的目录，首次启动、窗口资源、目录选择和退出/重启正常。
- [ ] Windows：分别安装 `.msi`/NSIS `.exe`，验证安装、启动、升级/卸载和含中文/空格路径。
- [ ] 两个平台核对 Google Web、DeepL Web、DeepL API、OpenAI 兼容四种设置卡片；网页模式不访问钥匙串。
- [ ] 用离线样本在 GUI 中完成 `lang` 和多文件 `chapters` 扫描、校对、dry-run、直接覆盖与人工校对路径。
- [ ] 在应用表格中只改译文；外部编辑器只改右侧后重新导入；确认修改左侧英文、file/path、源任务书后均安全拒绝。
- [ ] 人工确认备份可恢复、历史列表/报告可读、重复应用和重启后重新导入已应用 CMP 被拒绝。
- [ ] 确认日志位于应用旁 `logs/`、诊断 ZIP 同时含前后端日志且不含凭证/完整正文。

## 5. 真实服务验证（可选、单独记录）

- [ ] 经明确授权后分别验证计划支持的真实端点；记录日期、提供商、地址、模型、输入规模、缓存、批大小、有效并发、耗时和失败数。
- [ ] 对 429/第三方故障与本地回归分别归因，验证仅重试 `rate_limited` 项且人工译文保持不变。
- [ ] 若更新准确度结论，另做同口径人工抽检；不得把接口成功率或格式守卫通过率当作语义准确率。

## 6. 签名、公证与 GitHub 发布

- [ ] 配置并验证 Windows 代码签名证书；检查安装包签名和 SmartScreen 表现。
- [ ] 配置并验证 macOS Developer ID 签名、公证与 stapling；在干净机器检查 Gatekeeper。
- [ ] 从待发布 commit 创建签名或受保护的 `v0.2.0` tag。
- [ ] 观察 GitHub Actions 两个平台质量门禁与打包全部成功，下载并复核产物名称、版本和校验和。
- [ ] 创建 GitHub Release，附变更摘要、已知边界、安装包与校验和；确认 release 对应同一 tag/commit。
- [ ] 发布后从 GitHub Release 再次下载并做最小启动检查。
