use super::{
    chapters, cmp, cmp_records, crate_root, fs, glossary, json, logging, mode, parse_auto,
    prepare_entry, protect, providers, render_entry, restore, rich_text, snbt, source_fingerprint,
    stream, validate_cmp_identity, validate_cmp_source, AppError, AppResult, BTreeMap, Client,
    Duration, Emitter, EntryKind, ErrorCode, HashMap, HashSet, Instant, Item, LangValue, Local,
    Path, PathBuf, Settings, Sha256, StreamExt, Value,
};
use serde::Serialize;
use sha2::Digest;
use tauri::AppHandle;

#[derive(Serialize)]
struct TranslationUnitRecord<'a> {
    entry_id: &'a str,
    path: &'a str,
    source: &'a str,
    target: &'a str,
    status: &'static str,
}

fn save_translation_units(
    quests_dir: &Path,
    items: &[Item],
    results: &HashMap<String, String>,
    failure_statuses: &HashMap<String, &'static str>,
) -> Result<(), String> {
    let directory = quests_dir.join(".ftb-translator");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let mut output = String::new();
    for item in items {
        let target = results.get(&item.id).unwrap_or(&item.source);
        let record = TranslationUnitRecord {
            entry_id: &item.entry_id,
            path: &item.path,
            source: &item.source,
            target,
            status: failure_statuses
                .get(&item.entry_id)
                .copied()
                .unwrap_or("translated"),
        };
        output.push_str(&serde_json::to_string(&record).map_err(|e| e.to_string())?);
        output.push('\n');
    }
    fs::write(directory.join("translation-units-latest.jsonl"), output)
        .map_err(|e| format!("无法保存翻译中间文件：{e}"))
}

fn request_failure_status(error: &AppError) -> &'static str {
    if error.code == ErrorCode::RateLimited {
        "rate_limited"
    } else {
        "request_failed"
    }
}

pub(crate) fn warnings(source: &str, target: &str) -> Vec<String> {
    let (_, st) = protect(source);
    let (_, tt) = protect(target);
    let mut w = vec![];
    for (c, n) in [('\n', "换行"), ('\r', "回车"), ('\t', "制表符")] {
        if source.matches(c).count() != target.matches(c).count() {
            w.push(format!("{n}数量不一致"))
        }
    }
    let mut sc = st.iter().map(|x| x.1.clone()).collect::<Vec<_>>();
    let mut tc = tt.iter().map(|x| x.1.clone()).collect::<Vec<_>>();
    sc.sort();
    tc.sort();
    if sc != tc {
        w.push("格式码、占位符或资源标识不一致".into())
    }
    w.extend(super::protection::colour_style_warnings(source, target));
    if let Some(src) = rich_text::Document::parse(source) {
        match rich_text::Document::parse(target) {
            Some(tgt) => {
                if src.structure() != tgt.structure() {
                    w.push("JSON 文本组件结构发生变化".into())
                }
            }
            None => w.push("JSON 文本组件不再是有效 JSON".into()),
        }
    }
    w
}
pub(crate) fn cache_key(source: &str, s: &Settings) -> String {
    let mut h = Sha256::new();
    let cache_model = if s.provider == providers::OPENAI_COMPATIBLE {
        s.model.clone()
    } else {
        format!(
            "{}:{}:{}",
            s.provider,
            s.model,
            s.base_url.trim_end_matches('/')
        )
    };
    let cache_data = if s.glossary_enabled {
        let mut value = json!({
            "source_text":source,
            "model":cache_model,
            "target_locale":"zh_cn",
            "style":s.style,
            "glossary_enabled":true,
            "glossary_fingerprint":s.glossary_fingerprint
        });
        if rich_text::Document::parse(source).is_some_and(|document| !document.units().is_empty()) {
            value["rich_text_pipeline"] = json!("display-fields-v1");
        }
        value
    } else {
        let mut value = json!({"source_text":source,"model":cache_model,"target_locale":"zh_cn","style":s.style});
        if rich_text::Document::parse(source).is_some_and(|document| !document.units().is_empty()) {
            value["rich_text_pipeline"] = json!("display-fields-v1");
        }
        value
    };
    h.update(cache_data.to_string());
    hex::encode(h.finalize())
}
pub(crate) fn load_cache(q: &Path) -> HashMap<String, String> {
    [".ftb-translator", ".ftb-translater"]
        .into_iter()
        .find_map(|directory| {
            fs::read(q.join(directory).join("cache.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        })
        .unwrap_or_default()
}
pub(crate) fn save_cache(q: &Path, c: &HashMap<String, String>) -> Result<(), String> {
    let p = q.join(".ftb-translator/cache.json");
    fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(p, serde_json::to_vec_pretty(c).unwrap()).map_err(|e| e.to_string())
}
async fn request(
    client: &Client,
    s: &Settings,
    batch: &[Item],
    task_id: &str,
) -> AppResult<HashMap<String, String>> {
    let input = batch
        .iter()
        .map(|x| (x.id.clone(), x.protected.clone()))
        .collect::<Vec<_>>();
    providers::request(client, s, &input, task_id).await
}
pub async fn translate(app: AppHandle, data_dir: PathBuf, payload: Value) -> Result<(), String> {
    let started_at = Instant::now();
    let task_id = payload["_task_id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(logging::task_id);
    let q = PathBuf::from(payload["quests_dir"].as_str().ok_or("缺少任务书目录")?);
    let retry_document = payload["retry_cmp_path"]
        .as_str()
        .filter(|path| !path.trim().is_empty())
        .map(|path| cmp::load(Path::new(path)))
        .transpose()?;
    let retry_locations = retry_document
        .as_ref()
        .map_or_else(HashSet::new, |document| {
            document
                .records
                .iter()
                .filter(|record| record.status == "rate_limited")
                .map(|record| (record.entry_id.clone(), record.path.clone()))
                .collect()
        });
    if payload["retry_cmp_path"].is_string() && retry_locations.is_empty() {
        return Err("CMP 中没有可重试的限流条目".into());
    }
    let m = mode(&q)?;
    if let Some(document) = &retry_document {
        validate_cmp_identity(document, &q, m)?;
    }
    let mut settings = crate_root::storage::load_settings(&data_dir);
    for k in [
        "api_key",
        "provider",
        "base_url",
        "model",
        "style",
        "batch_size",
        "concurrency",
        "glossary_path",
    ] {
        if let Some(v) = payload[k].as_str() {
            match k {
                "api_key" => settings.api_key = v.into(),
                "provider" => settings.provider = v.into(),
                "base_url" => settings.base_url = v.into(),
                "model" => settings.model = v.into(),
                "style" => settings.style = v.into(),
                "batch_size" => settings.batch_size = v.into(),
                "glossary_path" => settings.glossary_path = v.into(),
                _ => settings.concurrency = v.into(),
            }
        }
    }
    if let Some(enabled) = payload["glossary_enabled"].as_bool() {
        settings.glossary_enabled = enabled;
    }
    providers::normalize(&settings.provider)?;
    if !providers::requires_api_key(&settings.provider) {
        settings.glossary_enabled = false;
        settings.batch_size = "auto".into();
        settings.concurrency = "auto".into();
    }
    logging::info(
        "translation",
        "task_started",
        "翻译任务已开始",
        json!({
            "task_id":task_id,
            "quests_dir":q,
            "mode":m,
            "provider":settings.provider,
            "model":settings.model,
            "glossary_enabled":settings.glossary_enabled
        }),
    );
    if providers::requires_api_key(&settings.provider) && settings.api_key.trim().is_empty() {
        settings.api_key = crate_root::storage::translation_api_key(&settings.provider)?;
    }
    let loaded_glossary = if settings.glossary_enabled {
        let path = if settings.glossary_path.trim().is_empty() {
            glossary::ensure_default(&data_dir)?
        } else {
            PathBuf::from(&settings.glossary_path)
        };
        let loaded = glossary::Loaded::load(&path)?;
        settings.glossary_path = path.display().to_string();
        settings.glossary_fingerprint = loaded.fingerprint().to_string();
        Some(loaded)
    } else {
        None
    };
    let mut entries = vec![];
    let mut items = vec![];
    if m == "lang" {
        let map = snbt::load(&q.join("lang/en_us.snbt"))?;
        for (entry_index, (k, v)) in map.iter().enumerate() {
            let source = match v {
                LangValue::Text(x) => x.clone(),
                LangValue::Lines(x) => x.join("\n"),
            };
            let (entry, entry_items) =
                prepare_entry(k.clone(), source, entry_index, loaded_glossary.as_ref());
            entries.push(entry);
            items.extend(entry_items);
        }
    } else {
        let mut entry_index = 0;
        for file in chapters::files(&q) {
            for s in chapters::extract(&file)? {
                let (entry, entry_items) = prepare_entry(
                    s.cache_id.clone(),
                    s.source.clone(),
                    entry_index,
                    loaded_glossary.as_ref(),
                );
                entries.push(entry);
                items.extend(entry_items);
                entry_index += 1;
            }
        }
    }
    if let Some(document) = &retry_document {
        validate_cmp_source(document, &q, m, &entries, &items)?;
    }
    logging::info(
        "translation",
        "content_prepared",
        "待翻译内容已解析",
        json!({"task_id":task_id,"entries":entries.len(),"translation_units":items.len()}),
    );
    for item in &items {
        logging::trace(
            "translation",
            "unit_prepared",
            "翻译单元已准备",
            json!({"task_id":task_id,"entry_id":item.entry_id,"path":item.path}),
        );
    }
    let mut cache = load_cache(&q);
    let items_by_id = items
        .iter()
        .cloned()
        .map(|item| (item.id.clone(), item))
        .collect::<HashMap<_, _>>();
    let mut results = HashMap::new();
    let mut pending = vec![];
    let mut hits = 0;
    for entry in &entries {
        if entry.unit_ids().is_empty() {
            results.insert(entry.id.clone(), entry.source.clone());
            continue;
        }
        if let Some(value) = cache.get(&cache_key(&entry.source, &settings)) {
            results.insert(entry.id.clone(), value.clone());
            hits += 1
        } else {
            pending.extend(
                entry
                    .unit_ids()
                    .into_iter()
                    .filter_map(|id| items_by_id.get(id).cloned()),
            );
        }
    }
    if !retry_locations.is_empty() {
        pending
            .retain(|item| retry_locations.contains(&(item.entry_id.clone(), item.path.clone())));
    }
    let bs = parse_auto(&settings.batch_size, 25)?;
    let mut concurrency = parse_auto(&settings.concurrency, 6)?.min(12);
    if let Some(limit) = providers::concurrency_limit(&settings.provider) {
        concurrency = concurrency.min(limit);
    }
    let batches = pending.chunks(bs).map(|x| x.to_vec()).collect::<Vec<_>>();
    let batch_count = batches.len();
    let total = pending.len();
    logging::info(
        "translation",
        "execution_plan",
        "翻译执行计划已生成",
        json!({
            "task_id":task_id,
            "entries":entries.len(),
            "cache_hits":hits,
            "pending_units":total,
            "batch_size":bs,
            "batch_count":batch_count,
            "concurrency":concurrency
        }),
    );
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| format!("无法初始化翻译网络客户端：{e}"))?;
    let app2 = app.clone();
    let settings2 = settings.clone();
    let stream = stream::iter(batches.into_iter().enumerate().map(|(batch_index, batch)| {
        let c = client.clone();
        let s = settings2.clone();
        let task_id = task_id.clone();
        async move {
            let batch_started = Instant::now();
            logging::debug(
                "provider",
                "batch_request_started",
                "翻译批次请求已发送",
                json!({
                    "task_id":task_id,
                    "batch_index":batch_index + 1,
                    "entries":batch.len(),
                    "provider":s.provider
                }),
            );
            let r = request(&c, &s, &batch, &task_id).await;
            match &r {
                Ok(values) => logging::debug(
                    "provider",
                    "batch_request_completed",
                    "翻译批次请求已完成",
                    json!({
                        "task_id":task_id,
                        "batch_index":batch_index + 1,
                        "returned_entries":values.len(),
                        "duration_ms":batch_started.elapsed().as_millis()
                    }),
                ),
                Err(error) => logging::warn(
                    "provider",
                    "batch_request_failed",
                    "翻译批次请求失败",
                    json!({
                        "task_id":task_id,
                        "batch_index":batch_index + 1,
                        "duration_ms":batch_started.elapsed().as_millis(),
                        "error":error
                    }),
                ),
            }
            (batch, r)
        }
    }))
    .buffer_unordered(concurrency);
    tokio::pin!(stream);
    let mut unit_results = HashMap::new();
    let mut failed_units = HashSet::new();
    let mut failed_entries = HashSet::new();
    let mut failure_statuses = HashMap::new();
    while let Some((batch, r)) = stream.next().await {
        match r {
            Ok(map) => {
                for x in batch {
                    let raw = map.get(&x.id).cloned().unwrap_or(x.protected);
                    match restore(&raw, &x.tokens) {
                        Ok(restored) => {
                            let _ = app2.emit(
                                "translation-event",
                                json!({
                                    "type":"translation_preview",
                                    "task_id":task_id,
                                    "entry_id":x.entry_id,
                                    "source":x.source,
                                    "target":restored,
                                    "status":"translated",
                                }),
                            );
                            unit_results.insert(x.id, restored);
                            let _ = app2.emit(
                                "translation-event",
                                json!({"type":"progress","task_id":task_id,"stage":"translating","done":unit_results.len(),"total":total}),
                            );
                        }
                        Err(error) => {
                            logging::warn(
                                "translation",
                                "placeholder_restore_failed",
                                "翻译单元占位符恢复失败",
                                json!({"task_id":task_id,"entry_id":x.entry_id,"path":x.path,"error":error}),
                            );
                            failed_entries.insert(x.entry_id.clone());
                            failure_statuses.insert(x.entry_id.clone(), "format_guard");
                            failed_units.insert(x.id.clone());
                            unit_results.insert(x.id, x.source);
                            let _ = app2.emit(
                                "translation-event",
                                json!({"type":"progress","task_id":task_id,"stage":"format_guard","done":unit_results.len(),"total":total}),
                            );
                        }
                    }
                }
            }
            Err(e) => {
                let status = request_failure_status(&e);
                for x in batch {
                    logging::warn(
                        "translation",
                        "unit_translation_failed",
                        "翻译单元请求失败",
                        json!({"task_id":task_id,"entry_id":x.entry_id,"path":x.path,"error":&e}),
                    );
                    failed_entries.insert(x.entry_id.clone());
                    failure_statuses.insert(x.entry_id.clone(), status);
                    failed_units.insert(x.id.clone());
                    unit_results.insert(x.id, x.source);
                    let _ = app2.emit(
                        "translation-event",
                        json!({"type":"progress","task_id":task_id,"stage":status,"done":unit_results.len(),"total":total}),
                    );
                }
            }
        }
    }
    save_translation_units(&q, &pending, &unit_results, &failure_statuses)?;
    logging::debug(
        "translation",
        "provider_phase_completed",
        "所有翻译批次已处理",
        json!({
            "task_id":task_id,
            "completed_units":unit_results.len(),
            "failed_units":failed_units.len()
        }),
    );
    for entry in &entries {
        if results.contains_key(&entry.id) {
            continue;
        }
        match render_entry(entry, &unit_results) {
            Ok(target) => {
                results.insert(entry.id.clone(), target);
            }
            Err(error) => {
                logging::warn(
                    "translation",
                    "entry_render_failed",
                    "翻译条目重建失败",
                    json!({"task_id":task_id,"entry_id":entry.id,"error":error}),
                );
                failed_entries.insert(entry.id.clone());
                results.insert(entry.id.clone(), entry.source.clone());
            }
        }
    }
    let mut warns = BTreeMap::new();
    for entry in &entries {
        if let EntryKind::Untouched(reason) = &entry.kind {
            warns.insert(entry.id.clone(), vec![reason.clone()]);
            failed_entries.insert(entry.id.clone());
            continue;
        }
        let translated = results.get(&entry.id).unwrap_or(&entry.source);
        let w = warnings(&entry.source, translated);
        if !w.is_empty() {
            warns.insert(entry.id.clone(), w);
            results.insert(entry.id.clone(), entry.source.clone());
        } else if translated != &entry.source {
            cache.insert(cache_key(&entry.source, &settings), translated.clone());
        }
    }
    save_cache(&q, &cache)?;
    let mut records = cmp_records(
        m,
        &entries,
        &items,
        &results,
        &warns,
        &failed_entries,
        &failure_statuses,
    )?;
    if let Some(previous) = &retry_document {
        let generated = records
            .into_iter()
            .map(|record| ((record.entry_id.clone(), record.path.clone()), record))
            .collect::<HashMap<_, _>>();
        records = previous
            .records
            .iter()
            .map(|record| {
                if retry_locations.contains(&(record.entry_id.clone(), record.path.clone())) {
                    generated
                        .get(&(record.entry_id.clone(), record.path.clone()))
                        .cloned()
                        .unwrap_or_else(|| record.clone())
                } else {
                    record.clone()
                }
            })
            .collect();
    }
    let cmp_path = payload["retry_cmp_path"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            q.join(".ftb-translator/reviews").join(format!(
                "translation-{}.cmp",
                Local::now().format("%Y%m%d-%H%M%S")
            ))
        });
    cmp::write(
        &cmp_path,
        &cmp::Document {
            meta: cmp::Meta {
                version: 1,
                task_id: task_id.clone(),
                quests_dir: q.display().to_string(),
                mode: m.into(),
                source_fingerprint: source_fingerprint(&entries),
                provider: settings.provider.clone(),
                base_url: settings.base_url.clone(),
                model: settings.model.clone(),
                style: settings.style.clone(),
                glossary_enabled: settings.glossary_enabled,
                glossary_fingerprint: settings.glossary_fingerprint.clone(),
                total_entries: entries.len(),
                cache_hits: hits,
            },
            records,
        },
    )?;
    crate_root::task_state::TaskStateStore::new(&data_dir)
        .and_then(|store| store.translation_succeeded(&task_id))
        .map_err(String::from)?;
    if let Err(error) = app.emit(
        "translation-event",
        json!({
            "type":"review_ready",
            "task_id":task_id,
            "cmp_path":cmp_path,
            "total_entries":entries.len(),
            "warning_count":warns.len(),
            "failed_count":failed_entries.len()
        }),
    ) {
        logging::warn(
            "translation",
            "review_ready_event_failed",
            "CMP 已生成，但无法向界面发送完成事件",
            json!({"task_id":task_id,"cmp_path":cmp_path,"error":error.to_string()}),
        );
    }
    logging::info(
        "translation",
        "review_ready",
        "CMP 人工校对文件已生成",
        json!({
            "task_id":task_id,
            "cmp_path":cmp_path,
            "total_entries":entries.len(),
            "cache_hits":hits,
            "failed_entries":failed_entries.len(),
            "warnings":warns.len(),
            "duration_ms":started_at.elapsed().as_millis()
        }),
    );
    Ok(())
}
