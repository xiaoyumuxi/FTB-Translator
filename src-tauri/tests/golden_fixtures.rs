#![allow(dead_code)]

#[path = "../src/chapters.rs"]
mod chapters;
#[path = "../src/cmp.rs"]
mod cmp;
#[path = "../src/error.rs"]
mod error;
#[path = "../src/glossary.rs"]
mod glossary;
#[path = "../src/logging.rs"]
mod logging;
#[path = "../src/protocol.rs"]
mod protocol;
#[path = "../src/providers.rs"]
mod providers;
#[path = "../src/rich_text.rs"]
mod rich_text;
#[path = "../src/snbt.rs"]
mod snbt;
#[path = "../src/storage.rs"]
mod storage;

// Compile the production façade and its path-based child modules so the offline
// golden pipeline always exercises the real scan/protection/review/writeback code.
mod application {
    include!("../src/core.rs");

    #[cfg(test)]
    mod golden_tests {
        use super::*;
        use serde::{Deserialize, Serialize};
        use std::{
            collections::{BTreeMap, HashMap},
            fs,
            path::Path,
        };
        use tempfile::tempdir;

        const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

        #[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
        struct UnitFixture {
            path: String,
            source: String,
        }

        #[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
        struct EntryFixture {
            entry_id: String,
            file: String,
            source: String,
            untouched: bool,
            units: Vec<UnitFixture>,
        }

        #[derive(Debug, Deserialize)]
        struct MockTranslation {
            entry_id: String,
            path: String,
            target: String,
        }

        fn copy_tree(source: &Path, target: &Path) {
            fs::create_dir_all(target).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let source_path = entry.path();
                let target_path = target.join(entry.file_name());
                if source_path.is_dir() {
                    copy_tree(&source_path, &target_path);
                } else {
                    fs::copy(source_path, target_path).unwrap();
                }
            }
        }

        fn fixture_json<T: serde::de::DeserializeOwned>(case: &str, name: &str) -> T {
            let path = Path::new(FIXTURES).join(case).join(name);
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
        }

        fn prepare(quests: &Path, mode: &str) -> (Vec<Entry>, Vec<Item>, Vec<EntryFixture>) {
            let mut entries = Vec::new();
            let mut items = Vec::new();
            if mode == "lang" {
                for (entry_index, (key, value)) in snbt::load(&quests.join("lang/en_us.snbt"))
                    .unwrap()
                    .into_iter()
                    .enumerate()
                {
                    let source = match value {
                        LangValue::Text(value) => value,
                        LangValue::Lines(values) => values.join("\n"),
                    };
                    let (entry, entry_items) = prepare_entry(key, source, entry_index, None);
                    entries.push(entry);
                    items.extend(entry_items);
                }
            } else {
                let mut entry_index = 0;
                for file in chapters::files(quests) {
                    for segment in chapters::extract(&file).unwrap() {
                        let (entry, entry_items) =
                            prepare_entry(segment.cache_id, segment.source, entry_index, None);
                        entries.push(entry);
                        items.extend(entry_items);
                        entry_index += 1;
                    }
                }
            }

            let fixtures = entries
                .iter()
                .map(|entry| EntryFixture {
                    entry_id: entry.id.clone(),
                    file: entry_source_file(mode, &entry.id),
                    source: entry.source.clone(),
                    untouched: matches!(entry.kind, EntryKind::Untouched(_)),
                    units: items
                        .iter()
                        .filter(|item| item.entry_id == entry.id)
                        .map(|item| UnitFixture {
                            path: item.path.clone(),
                            source: item.source.clone(),
                        })
                        .collect(),
                })
                .collect();
            (entries, items, fixtures)
        }

        fn mock_document(
            case: &str,
            quests_dir: &str,
            mode: &str,
            entries: &[Entry],
            items: &[Item],
        ) -> cmp::Document {
            let translations: Vec<MockTranslation> = fixture_json(case, "mock-translations.json");
            let targets = translations
                .into_iter()
                .map(|translation| ((translation.entry_id, translation.path), translation.target))
                .collect::<HashMap<_, _>>();
            assert_eq!(targets.len(), items.len(), "Mock 必须覆盖所有翻译单元");
            let records = items
                .iter()
                .map(|item| {
                    let target = targets
                        .get(&(item.entry_id.clone(), item.path.clone()))
                        .unwrap_or_else(|| panic!("缺少 Mock：{} {}", item.entry_id, item.path));
                    cmp::Record {
                        file: entry_source_file(mode, &item.entry_id),
                        entry_id: item.entry_id.clone(),
                        path: item.path.clone(),
                        source: item.source.clone(),
                        target: target.clone(),
                        status: if target == &item.source {
                            "unchanged".into()
                        } else {
                            "translated".into()
                        },
                    }
                })
                .collect();
            cmp::Document {
                meta: cmp::Meta {
                    version: 1,
                    task_id: format!("fixture-{case}"),
                    quests_dir: quests_dir.into(),
                    mode: mode.into(),
                    source_fingerprint: source_fingerprint(entries),
                    provider: "fixture_mock".into(),
                    base_url: "mock://offline".into(),
                    model: "deterministic-v1".into(),
                    style: "fixture".into(),
                    glossary_enabled: false,
                    glossary_fingerprint: String::new(),
                    total_entries: entries.len(),
                    cache_hits: 0,
                },
                records,
            }
        }

        fn assert_golden_case(case: &str, mode: &str) {
            let fixture = Path::new(FIXTURES).join(case);
            let directory = tempdir().unwrap();
            copy_tree(&fixture.join("input"), directory.path());
            let quests = directory
                .path()
                .join("config/ftbquests/quests")
                .canonicalize()
                .unwrap();

            let scan = scan(&json!({"path":directory.path(),"batch_size":"auto"})).unwrap();
            let (entries, items, extracted) = prepare(&quests, mode);
            let expected: Vec<EntryFixture> = fixture_json(case, "expected-extraction.json");
            assert_eq!(scan["mode"], mode);
            assert_eq!(scan["entry_count"], entries.len());
            assert_eq!(extracted, expected);
            let expected_files = expected.iter().fold(BTreeMap::new(), |mut files, entry| {
                *files.entry(entry.file.clone()).or_insert(0_usize) += 1;
                files
            });
            let scanned_files = scan["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| {
                    (
                        file["path"].as_str().unwrap().to_string(),
                        file["entry_count"].as_u64().unwrap() as usize,
                    )
                })
                .collect::<BTreeMap<_, _>>();
            assert_eq!(scan["file_count"], expected_files.len());
            assert_eq!(scanned_files, expected_files);

            let golden_document = mock_document(case, "{{QUESTS_DIR}}", mode, &entries, &items);
            let golden_path = directory.path().join("golden.cmp");
            cmp::write(&golden_path, &golden_document).unwrap();
            let actual_cmp = fs::read_to_string(&golden_path).unwrap();
            let expected_cmp = fs::read_to_string(fixture.join("expected.cmp")).unwrap();
            assert_eq!(actual_cmp.strip_suffix('\n').unwrap(), expected_cmp);

            let document =
                mock_document(case, &quests.display().to_string(), mode, &entries, &items);
            let cmp_path = directory.path().join("review.cmp");
            cmp::write(&cmp_path, &document).unwrap();
            let source_before_validation = if mode == "lang" {
                vec![(
                    quests.join("lang/en_us.snbt"),
                    fs::read(quests.join("lang/en_us.snbt")).unwrap(),
                )]
            } else {
                chapters::files(&quests)
                    .into_iter()
                    .map(|path| (path.clone(), fs::read(path).unwrap()))
                    .collect()
            };
            let cmp_before_validation = fs::read(&cmp_path).unwrap();
            let validation =
                validate_cmp(&json!({"cmp_path":cmp_path,"quests_dir":quests}), &[]).unwrap();
            assert!(!validation.blocking);
            assert_eq!(validation.applicable_entries, entries.len());
            assert_eq!(validation.format_guard_failures, 0);
            assert_eq!(
                validation.files_to_modify,
                if mode == "lang" {
                    vec!["lang/zh_cn.snbt".to_string()]
                } else {
                    expected_files.keys().cloned().collect::<Vec<_>>()
                }
            );
            assert_eq!(fs::read(&cmp_path).unwrap(), cmp_before_validation);
            for (path, content) in source_before_validation {
                assert_eq!(fs::read(path).unwrap(), content);
            }
            assert!(!quests.join(".ftb-translater").exists());
            assert!(!quests.join("lang/zh_cn.snbt").exists());
            let result = apply_cmp(
                &directory.path().join("app-data"),
                &json!({"cmp_path":cmp_path,"quests_dir":quests}),
            )
            .unwrap();
            assert_eq!(result["report"]["total_entries"], entries.len());
            assert_eq!(
                result["report"]["warnings"].as_object().unwrap().len(),
                expected.iter().filter(|entry| entry.untouched).count()
            );

            let expected_root = fixture.join("expected/config/ftbquests/quests");
            if mode == "lang" {
                assert_eq!(
                    fs::read(quests.join("lang/zh_cn.snbt")).unwrap(),
                    fs::read(expected_root.join("lang/zh_cn.snbt")).unwrap()
                );
            } else {
                for path in chapters::files(&quests) {
                    let name = path.file_name().unwrap();
                    assert_eq!(
                        fs::read(&path).unwrap(),
                        fs::read(expected_root.join("chapters").join(name)).unwrap()
                    );
                }
            }
            assert!(quests.join(".ftb-translater/backups").is_dir());
        }

        #[test]
        fn lang_pipeline_matches_golden_files_offline() {
            assert_golden_case("lang-rich", "lang");
        }

        #[test]
        fn chapters_pipeline_matches_golden_files_offline() {
            assert_golden_case("chapters-nested", "chapters");
        }

        #[test]
        fn invalid_late_record_in_multi_file_cmp_writes_nothing() {
            let fixture = Path::new(FIXTURES).join("chapters-nested");
            let directory = tempdir().unwrap();
            copy_tree(&fixture.join("input"), directory.path());
            let quests = directory
                .path()
                .join("config/ftbquests/quests")
                .canonicalize()
                .unwrap();
            let before = chapters::files(&quests)
                .into_iter()
                .map(|path| (path.clone(), fs::read(&path).unwrap()))
                .collect::<Vec<_>>();
            let (entries, items, _) = prepare(&quests, "chapters");
            let mut document = mock_document(
                "chapters-nested",
                &quests.display().to_string(),
                "chapters",
                &entries,
                &items,
            );
            let late = document
                .records
                .iter_mut()
                .find(|record| record.entry_id == "beta.snbt:1:description")
                .unwrap();
            late.target = "错误地删除颜色码".into();
            let cmp_path = directory.path().join("invalid-late-record.cmp");
            cmp::write(&cmp_path, &document).unwrap();

            let error = apply_cmp(
                &directory.path().join("app-data"),
                &json!({"cmp_path":cmp_path,"quests_dir":quests}),
            )
            .unwrap_err();
            assert!(error.contains("格式守卫"));
            for (path, content) in before {
                assert_eq!(fs::read(path).unwrap(), content);
            }
            assert!(!quests.join(".ftb-translater/backups").exists());
        }

        #[test]
        fn commit_failure_restores_earlier_file_from_fixture() {
            let fixture = Path::new(FIXTURES).join("multi-file-rollback");
            let directory = tempdir().unwrap();
            copy_tree(&fixture.join("input"), directory.path());
            let first = directory.path().join("first.snbt");
            let second = directory.path().join("second.snbt");
            fs::write(&second, "{ title: \"Original second\" }\n").unwrap();
            let outputs = vec![
                FileOutput {
                    path: first.clone(),
                    archive_name: "chapters/first.snbt".into(),
                    content: "{ title: \"Translated first\" }\n".into(),
                },
                FileOutput {
                    path: second.clone(),
                    archive_name: "chapters/second.snbt".into(),
                    content: "{ title: \"Translated second\" }\n".into(),
                },
            ];
            let options = WritebackOptions {
                commit_fault: CommitFault::Replace(1),
                ..WritebackOptions::default()
            };

            let error =
                commit_outputs_with_options(&outputs, "fixture-rollback", &options).unwrap_err();
            assert_eq!(error.code, ErrorCode::CommitFailed);
            assert!(error.user_message.contains("已恢复此前写入的文件"));
            assert!(!error.task_book_modified);
            assert_eq!(
                fs::read(&first).unwrap(),
                fs::read(fixture.join("expected/first.snbt")).unwrap()
            );
            assert_eq!(
                fs::read_to_string(second).unwrap(),
                "{ title: \"Original second\" }\n"
            );
        }

        #[test]
        fn backend_module_boundaries_exclude_forbidden_dependencies() {
            let writeback = include_str!("../src/core/writeback.rs");
            assert!(
                !writeback.contains("providers::"),
                "writeback must not call translation providers"
            );

            let translation = include_str!("../src/core/translation.rs");
            assert!(
                !translation.contains("commit_outputs") && !translation.contains("backup("),
                "translation must not commit or back up task-book files"
            );

            let review = include_str!("../src/core/review.rs");
            assert!(
                !review.contains("commit_outputs") && !review.contains("backup("),
                "review must not commit or back up task-book files"
            );

            let provider = include_str!("../src/providers.rs");
            assert!(
                !provider.contains("cmp::"),
                "providers must not parse or write CMP documents"
            );
        }
    }
}
