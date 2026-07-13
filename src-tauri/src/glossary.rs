use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub const DEFAULT_FILENAME: &str = "minecraft_glossary.json";
const DEFAULT_CONTENT: &str = include_str!("../resources/minecraft_glossary.json");

#[derive(Clone, Debug, Deserialize)]
struct Entry {
    source: String,
    target: String,
    #[serde(default)]
    case_sensitive: bool,
}

#[derive(Deserialize)]
struct GlossaryFile {
    version: u32,
    entries: Vec<Entry>,
}

#[derive(Clone, Debug)]
pub struct Loaded {
    entries: Vec<Entry>,
    fingerprint: String,
}

pub fn default_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DEFAULT_FILENAME)
}

pub fn ensure_default(data_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(data_dir).map_err(|e| format!("无法创建应用数据目录：{e}"))?;
    let path = default_path(data_dir);
    if !path.is_file() {
        fs::write(&path, DEFAULT_CONTENT)
            .map_err(|e| format!("无法创建默认 Minecraft 词表：{e}"))?;
    }
    Ok(path)
}

impl Loaded {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path)
            .map_err(|e| format!("无法读取 Minecraft 词表 {}：{e}", path.display()))?;
        Self::from_bytes(&bytes).map_err(|e| format!("Minecraft 词表 {} 无效：{e}", path.display()))
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut file: GlossaryFile =
            serde_json::from_slice(bytes).map_err(|e| format!("JSON 解析失败：{e}"))?;
        if file.version == 0 {
            return Err("version 必须大于 0".into());
        }
        if file.entries.is_empty() {
            return Err("entries 不能为空".into());
        }
        let mut seen = HashSet::new();
        for entry in &file.entries {
            if entry.source.trim().is_empty() || entry.target.trim().is_empty() {
                return Err("source 和 target 不能为空".into());
            }
            let key = if entry.case_sensitive {
                format!("case:{}", entry.source)
            } else {
                format!("fold:{}", entry.source.to_ascii_lowercase())
            };
            if !seen.insert(key) {
                return Err(format!("存在重复术语：{}", entry.source));
            }
        }
        file.entries
            .sort_by_key(|entry| std::cmp::Reverse(entry.source.len()));
        Ok(Self {
            entries: file.entries,
            fingerprint: hex::encode(Sha256::digest(bytes)),
        })
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn protect(&self, text: &str) -> (String, Vec<(String, String)>) {
        let mut output = String::with_capacity(text.len());
        let mut tokens = vec![];
        let mut pos = 0;
        while pos < text.len() {
            let found = self
                .entries
                .iter()
                .find_map(|entry| matches_at(text, pos, entry).map(|end| (entry, end)));
            if let Some((entry, end)) = found {
                let placeholder = format!("⟨G_{}⟩", tokens.len());
                output.push_str(&placeholder);
                tokens.push((placeholder, entry.target.clone()));
                pos = end;
            } else {
                let ch = text[pos..].chars().next().expect("pos is a char boundary");
                output.push(ch);
                pos += ch.len_utf8();
            }
        }
        (output, tokens)
    }
}

fn boundary(text: &str, start: usize, end: usize, source: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let needs_left = source.chars().next().is_some_and(word);
    let needs_right = source.chars().last().is_some_and(word);
    let left_ok = !needs_left || text[..start].chars().next_back().is_none_or(|c| !word(c));
    let right_ok = !needs_right || text[end..].chars().next().is_none_or(|c| !word(c));
    left_ok && right_ok
}

fn matches_at(text: &str, start: usize, entry: &Entry) -> Option<usize> {
    let end = start.checked_add(entry.source.len())?;
    let candidate = text.get(start..end)?;
    let same = if entry.case_sensitive {
        candidate == entry.source
    } else {
        candidate.eq_ignore_ascii_case(&entry.source)
    };
    (same && boundary(text, start, end, &entry.source)).then_some(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn default_glossary() -> Loaded {
        Loaded::from_bytes(DEFAULT_CONTENT.as_bytes()).unwrap()
    }

    #[test]
    fn creates_an_editable_default_file_without_overwriting_it() {
        let dir = tempdir().unwrap();
        let path = ensure_default(dir.path()).unwrap();
        assert_eq!(Loaded::load(&path).unwrap().len(), default_glossary().len());
        fs::write(
            &path,
            r#"{"version":2,"entries":[{"source":"Demo","target":"演示"}]}"#,
        )
        .unwrap();
        ensure_default(dir.path()).unwrap();
        assert_eq!(Loaded::load(&path).unwrap().len(), 1);
    }

    #[test]
    fn content_changes_produce_a_new_cache_fingerprint() {
        let a = Loaded::from_bytes(
            r#"{"version":1,"entries":[{"source":"Demo","target":"演示"}]}"#.as_bytes(),
        )
        .unwrap();
        let b = Loaded::from_bytes(
            r#"{"version":1,"entries":[{"source":"Demo","target":"示例"}]}"#.as_bytes(),
        )
        .unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn loads_a_large_curated_glossary() {
        assert!(default_glossary().len() >= 620);
    }

    #[test]
    fn prefers_longest_term_and_respects_boundaries() {
        let glossary = default_glossary();
        let (text, tokens) =
            glossary.protect("Use an Enchanting Table, not a timetable, with Mekanism.");
        assert_eq!(text.matches("⟨G_").count(), 2);
        assert!(text.contains("timetable"));
        assert_eq!(tokens[0].1, "附魔台");
        assert_eq!(tokens[1].1, "通用机械");
    }

    #[test]
    fn preserves_uncertain_mod_names_instead_of_machine_translating_them() {
        let (_, tokens) = default_glossary().protect("Oritech and XNet");
        assert_eq!(tokens[0].1, "Oritech");
        assert_eq!(tokens[1].1, "XNet");
    }

    #[test]
    fn covers_terms_from_different_modpack_ecosystems() {
        let glossary = default_glossary();
        let (_, tokens) = glossary.protect(
            "Use a Mechanical Press from the Create mod, a Smeltery Controller from Tinkers' Construct, and a Mana Pool from Botania.",
        );
        let targets = tokens
            .into_iter()
            .map(|(_, target)| target)
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            vec![
                "动力冲压机",
                "机械动力模组",
                "冶炼炉控制器",
                "匠魂",
                "魔力池",
                "植物魔法"
            ]
        );
    }

    #[test]
    fn avoids_ambiguous_standalone_mod_names() {
        let glossary = default_glossary();
        let (text, tokens) = glossary.protect("Create a spectrum carefully.");
        assert_eq!(text, "Create a spectrum carefully.");
        assert!(tokens.is_empty());
    }
}
