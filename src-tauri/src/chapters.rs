use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    pub path: PathBuf,
    pub key: String,
    pub source: String,
    pub start: usize,
    pub end: usize,
    pub quote: char,
    pub index: usize,
    pub cache_id: String,
}
pub fn files(q: &Path) -> Vec<PathBuf> {
    let mut v = fs::read_dir(q.join("chapters"))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "snbt"))
        .collect::<Vec<_>>();
    v.sort();
    v
}
fn decode(s: &str) -> String {
    let mut o = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            o.push(match it.next().unwrap_or('\\') {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                x => x,
            })
        } else {
            o.push(c)
        }
    }
    o
}
fn quote(s: &str, q: char) -> String {
    format!(
        "{q}{}{q}",
        s.replace('\\', "\\\\")
            .replace(q, &format!("\\{q}"))
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}

fn validate_structure(text: &str) -> Result<(), String> {
    let mut chars = text.chars().peekable();
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    while let Some(character) = chars.next() {
        if comment {
            if character == '\n' {
                comment = false;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        if character == '/' && chars.peek() == Some(&'/') {
            chars.next();
            comment = true;
            continue;
        }
        if character == '#' {
            comment = true;
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '{' | '[' | '(' => stack.push(character),
            '}' | ']' | ')' => {
                let expected = match character {
                    '}' => '{',
                    ']' => '[',
                    ')' => '(',
                    _ => unreachable!(),
                };
                if stack.pop() != Some(expected) {
                    return Err(format!("章节 SNBT 的 {character} 没有匹配的起始括号"));
                }
            }
            _ => {}
        }
    }
    if quote.is_some() || escaped {
        return Err("章节 SNBT 存在未闭合的字符串".into());
    }
    if !stack.is_empty() {
        return Err("章节 SNBT 存在未闭合的括号".into());
    }
    Ok(())
}

pub fn extract(path: &Path) -> Result<Vec<Segment>, String> {
    static FIELD: OnceLock<Regex> = OnceLock::new();
    static STRINGS: OnceLock<Regex> = OnceLock::new();
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let re = FIELD.get_or_init(|| {
        Regex::new(r#"(?s)(?:\b(title|subtitle|description|text|name)|["'](title|subtitle|description|text|name)["'])\s*:\s*(\[[^\]]*]|"(?:\\.|[^"])*"|'(?:\\.|[^'])*')"#)
            .expect("chapter-field regex must be valid")
    });
    let strings = STRINGS.get_or_init(|| {
        Regex::new(r#""((?:\\.|[^"])*)"|'((?:\\.|[^'])*)'"#)
            .expect("chapter-string regex must be valid")
    });
    let mut out = vec![];
    for cap in re.captures_iter(&text) {
        let match_start = cap.get(0).unwrap().start();
        let line_start = text[..match_start].rfind('\n').map_or(0, |i| i + 1);
        let prefix = &text[line_start..match_start];
        if prefix.trim_start().starts_with('#') || prefix.contains("//") {
            continue;
        }
        let key = cap.get(1).or(cap.get(2)).unwrap().as_str();
        let value = cap.get(3).unwrap();
        for sc in strings.captures_iter(value.as_str()) {
            let m = sc.get(0).unwrap();
            let raw = sc.get(1).or(sc.get(2)).unwrap().as_str();
            let source = decode(raw);
            if !source.chars().any(|c| c.is_ascii_alphabetic()) {
                continue;
            }
            let start = value.start() + m.start();
            let idx = out.len();
            out.push(Segment {
                path: path.to_path_buf(),
                key: key.into(),
                source,
                start,
                end: value.start() + m.end(),
                quote: m.as_str().chars().next().unwrap(),
                index: idx,
                cache_id: format!(
                    "{}:{idx}:{key}",
                    path.file_name().unwrap().to_string_lossy()
                ),
            });
        }
    }
    Ok(out)
}
pub fn render_replacements(
    path: &Path,
    replacements: &[(usize, String)],
) -> Result<(String, usize), String> {
    let mut text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    validate_structure(&text)?;
    let mut indices = HashSet::new();
    if replacements
        .iter()
        .any(|(index, _)| !indices.insert(*index))
    {
        return Err("章节写回包含重复的翻译位置".into());
    }
    let segs = extract(path)?;
    let mut matches = segs
        .into_iter()
        .filter_map(|s| {
            replacements
                .iter()
                .find(|x| x.0 == s.index)
                .map(|x| (s, x.1.clone()))
        })
        .collect::<Vec<_>>();
    if matches.len() != replacements.len() {
        return Err("章节写回位置与原文件不一致，已取消写入".into());
    }
    matches.sort_by_key(|x| std::cmp::Reverse(x.0.start));
    for (s, t) in &matches {
        text.replace_range(s.start..s.end, &quote(t, s.quote));
    }
    validate_structure(&text)?;
    Ok((text, matches.len()))
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn extracts_and_replaces() {
        let d = tempdir().unwrap();
        let p = d.path().join("a.snbt");
        fs::write(
            &p,
            "{\n // title: \"Comment\"\n title: \"Hello\", description: [\"Line one\", \"第二行\"]\n}",
        )
        .unwrap();
        let s = extract(&p).unwrap();
        assert_eq!(s.len(), 2);
        let (rendered, count) = render_replacements(&p, &[(0, "你好".into())]).unwrap();
        assert_eq!(count, 1);
        assert!(rendered.contains("你好"));
    }

    #[test]
    fn replacement_text_cannot_escape_the_original_snbt_string() {
        let d = tempdir().unwrap();
        let p = d.path().join("a.snbt");
        fs::write(&p, r#"{ title: "Hello" }"#).unwrap();
        let injected = r#""}], malicious: { value: 1 }, text: ""#;
        let (rendered, count) = render_replacements(&p, &[(0, injected.into())]).unwrap();
        assert_eq!(count, 1);
        validate_structure(&rendered).unwrap();
        assert!(rendered.contains(r#"\"}], malicious"#));
    }

    #[test]
    fn rejects_missing_or_duplicate_replacement_positions() {
        let d = tempdir().unwrap();
        let p = d.path().join("a.snbt");
        fs::write(&p, r#"{ title: "Hello" }"#).unwrap();
        assert!(render_replacements(&p, &[(4, "你好".into())]).is_err());
        assert!(render_replacements(&p, &[(0, "甲".into()), (0, "乙".into())]).is_err());
    }
}
