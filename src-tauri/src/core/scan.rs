use super::{chapters, logging, snbt, Scan, ScanFile};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn has_lang(p: &Path) -> bool {
    p.join("lang/en_us.snbt").is_file()
}
fn has_chapters(p: &Path) -> bool {
    !chapters::files(p).is_empty()
}
pub fn resolve(selected: &Path) -> Result<PathBuf, String> {
    let s = selected
        .canonicalize()
        .map_err(|e| format!("无法打开所选目录：{e}"))?;
    let mut candidates = vec![s.clone()];
    if s.file_name()
        .is_some_and(|x| x == "lang" || x == "chapters")
    {
        if let Some(p) = s.parent() {
            candidates.push(p.into())
        }
    }
    for a in s.ancestors() {
        if a.file_name().is_some_and(|x| x == "quests") {
            candidates.push(a.into())
        }
        if a.file_name().is_some_and(|x| x == "ftbquests") {
            candidates.push(a.join("quests"))
        }
        if a.file_name().is_some_and(|x| x == "config") {
            candidates.push(a.join("ftbquests/quests"))
        }
    }
    candidates.extend([
        s.join("config/ftbquests/quests"),
        s.join("ftbquests/quests"),
        s.join("quests"),
    ]);
    for e in WalkDir::new(&s)
        .max_depth(5)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir() && e.file_name() == "quests")
    {
        candidates.push(e.path().into())
    }
    candidates
        .into_iter()
        .find(|p| has_lang(p) || has_chapters(p))
        .ok_or("没有找到 FTB Quests 的 lang/en_us.snbt 或 chapters/*.snbt。".into())
}
pub(crate) fn mode(q: &Path) -> Result<&'static str, String> {
    if has_lang(q) {
        Ok("lang")
    } else if has_chapters(q) {
        Ok("chapters")
    } else {
        Err("任务书目录中没有可翻译内容".into())
    }
}
fn pack_name(q: &Path) -> String {
    q.ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "config"))
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            q.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
}
pub fn scan(payload: &Value) -> Result<Value, String> {
    let q = resolve(Path::new(payload["path"].as_str().unwrap_or("")))?;
    let m = mode(&q)?;
    let (files, count, source, file_details) = if m == "lang" {
        let p = q.join("lang/en_us.snbt");
        let count = snbt::load(&p)?.len();
        (
            1,
            count,
            p,
            vec![ScanFile {
                path: "lang/en_us.snbt".into(),
                entry_count: count,
            }],
        )
    } else {
        let fs = chapters::files(&q);
        let file_details = fs
            .iter()
            .map(|path| {
                chapters::extract(path).map(|entries| ScanFile {
                    path: format!(
                        "chapters/{}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    entry_count: entries.len(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let count = file_details.iter().map(|file| file.entry_count).sum();
        (fs.len(), count, q.join("chapters"), file_details)
    };
    let bs = parse_auto(payload["batch_size"].as_str().unwrap_or("auto"), 25)?;
    let scan = Scan {
        quests_dir: q.display().to_string(),
        pack_name: pack_name(&q),
        mode: m.into(),
        mode_label: if m == "lang" {
            "语言文件"
        } else {
            "章节文件"
        }
        .into(),
        source: source.display().to_string(),
        entry_count: count,
        file_count: files,
        files: file_details,
        estimated_batches: count.div_ceil(bs),
    };
    logging::info(
        "scanner",
        "scan_completed",
        "任务书扫描完成",
        json!({
            "quests_dir":scan.quests_dir,
            "mode":scan.mode,
            "entry_count":scan.entry_count,
            "file_count":scan.file_count
        }),
    );
    Ok(serde_json::to_value(scan).unwrap())
}
pub(crate) fn parse_auto(s: &str, default: usize) -> Result<usize, String> {
    if s.trim().is_empty() || s.eq_ignore_ascii_case("auto") {
        Ok(default)
    } else {
        s.parse::<usize>()
            .ok()
            .filter(|x| *x > 0)
            .ok_or("批大小与并发数必须是 auto 或正整数".into())
    }
}
