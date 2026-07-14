use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
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
pub fn extract(path: &Path) -> Result<Vec<Segment>, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let re=Regex::new(r#"(?s)(?:\b(title|subtitle|description|text|name)|["'](title|subtitle|description|text|name)["'])\s*:\s*(\[[^\]]*]|"(?:\\.|[^"])*"|'(?:\\.|[^'])*')"#).unwrap();
    let strings = Regex::new(r#""((?:\\.|[^"])*)"|'((?:\\.|[^'])*)'"#).unwrap();
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
    matches.sort_by_key(|x| std::cmp::Reverse(x.0.start));
    for (s, t) in &matches {
        text.replace_range(s.start..s.end, &quote(t, s.quote));
    }
    Ok((text, matches.len()))
}
pub fn replace(path: &Path, replacements: &[(usize, String)]) -> Result<usize, String> {
    let (text, count) = render_replacements(path, replacements)?;
    fs::write(path, text).map_err(|e| e.to_string())?;
    Ok(count)
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
        assert_eq!(replace(&p, &[(0, "你好".into())]).unwrap(), 1);
        assert!(fs::read_to_string(p).unwrap().contains("你好"));
    }
}
