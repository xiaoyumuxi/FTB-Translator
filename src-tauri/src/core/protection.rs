use super::{glossary, rich_text, Entry, EntryKind, Item};
use regex::Regex;
use std::{
    collections::{BTreeMap, HashMap},
    sync::OnceLock,
};

const COLOUR_PATTERN: &str = r"(?i)[&§][0-9a-fk-orz]";

fn colour_pattern() -> &'static Regex {
    static COLOUR: OnceLock<Regex> = OnceLock::new();
    COLOUR
        .get_or_init(|| Regex::new(COLOUR_PATTERN).expect("colour-protection regex must be valid"))
}

fn patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            [
                COLOUR_PATTERN,
                r"%(?:\d+\$)?[-+# 0,(]*\d*(?:\.\d+)?[bcdeEufFgGosxX]",
                r"<[^<>\n]+>",
                r"\{\{[^{}\n]+\}\}",
                r"\{[@A-Za-z][^{}\n]*\}",
                r"@[pares]\b(?:\[[^\]\n]*\])?",
                r"#[a-z][a-z0-9_.-]*:[a-z0-9_./-]+",
                r"[a-z][a-z0-9_.-]*:[a-z0-9_./-]+",
                r"(?i)\b(?:[a-z0-9_.-]+:[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+|(?:assets|config|data|kubejs|models|recipes|textures|ftbquests|chapters|lang|scripts)/[a-z0-9_./-]+|[a-z0-9_.-]+(?:/[a-z0-9_.-]+)+\.[a-z0-9]+)\b",
                r#"\\[nrt\"'\\]"#,
                r#"https?://[^\s\"')\]]+"#,
                r"#[0-9a-fA-F]{6}\b",
                r"\b[vV]?\d+(?:\.\d+)+\b",
                r"\b\d+(?:[.,]\d+)?(?:%|[a-zA-Z]+)?\b",
            ]
            .iter()
            .map(|pattern| Regex::new(pattern).expect("format-protection regex must be valid"))
            .collect()
        })
        .as_slice()
}
pub(crate) fn protect(text: &str) -> (String, Vec<(String, String)>) {
    let mut found = vec![];
    for re in patterns() {
        for m in re.find_iter(text) {
            found.push((m.start(), m.end(), m.as_str().to_string()))
        }
    }
    found.sort_by_key(|x| x.0);
    found.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    let mut out = String::new();
    let mut last = 0;
    let mut tokens = vec![];
    for (start, end, t) in found {
        if start < last {
            continue;
        }
        out.push_str(&text[last..start]);
        let ph = format!("⟨P_{}⟩", tokens.len());
        out.push_str(&ph);
        tokens.push((ph, t));
        last = end;
    }
    out.push_str(&text[last..]);
    (out, tokens)
}
pub(crate) fn restore(text: &str, tokens: &[(String, String)]) -> Result<String, String> {
    static OPAQUE_PLACEHOLDER: OnceLock<Regex> = OnceLock::new();
    let placeholder = OPAQUE_PLACEHOLDER
        .get_or_init(|| Regex::new(r"⟨[PG]_\d+⟩").expect("opaque-placeholder regex must be valid"));
    let mut expected = tokens
        .iter()
        .map(|(token, _)| token.clone())
        .collect::<Vec<_>>();
    let mut actual = placeholder
        .find_iter(text)
        .map(|matched| matched.as_str().to_string())
        .collect::<Vec<_>>();
    expected.sort();
    actual.sort();
    if actual != expected {
        return Err("翻译接口修改、增删或重复了不透明占位符".into());
    }
    Ok(tokens
        .iter()
        .fold(text.to_string(), |value, (token, original)| {
            value.replace(token, original)
        }))
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ColourAnomaly {
    ResetWithoutActiveStyle,
    StyleWithoutText,
}

#[derive(Default)]
struct ColourAst {
    tokens: Vec<String>,
    spans: Vec<Vec<String>>,
    anomalies: Vec<ColourAnomaly>,
}

fn extract_flat_colour_ast(text: &str) -> ColourAst {
    let mut ast = ColourAst::default();
    let mut active = Vec::new();
    let mut position = 0;
    let mut style_without_text = false;

    for matched in colour_pattern().find_iter(text) {
        let chunk = &text[position..matched.start()];
        if !chunk.is_empty() && !active.is_empty() {
            ast.spans.push(active.clone());
        }

        let token = matched.as_str().to_string();
        let code = token
            .chars()
            .nth(1)
            .expect("a matched colour token always contains a code")
            .to_ascii_lowercase();
        ast.tokens.push(token.clone());
        match code {
            'r' => {
                if active.is_empty() {
                    ast.anomalies.push(ColourAnomaly::ResetWithoutActiveStyle);
                }
                active.clear();
                style_without_text = false;
            }
            '0'..='9' | 'a'..='f' | 'z' => {
                active = vec![token];
                style_without_text = true;
            }
            _ => {
                if !active.contains(&token) {
                    active.push(token);
                }
                style_without_text = true;
            }
        }
        position = matched.end();
    }

    let tail = &text[position..];
    if !tail.is_empty() {
        if !active.is_empty() {
            ast.spans.push(active);
        }
        style_without_text = false;
    }
    if style_without_text {
        ast.anomalies.push(ColourAnomaly::StyleWithoutText);
    }
    ast
}

fn extract_colour_ast(text: &str) -> ColourAst {
    let Some(document) = rich_text::Document::parse(text) else {
        return extract_flat_colour_ast(text);
    };
    let mut ast = ColourAst::default();
    for unit in document.units() {
        let unit_ast = extract_flat_colour_ast(&unit.source);
        ast.tokens.extend(unit_ast.tokens);
        ast.spans.extend(unit_ast.spans);
        ast.anomalies.extend(unit_ast.anomalies);
    }
    ast
}

fn counts<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeMap<T, usize> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
}

pub(crate) fn colour_style_warnings(source: &str, target: &str) -> Vec<String> {
    let source = extract_colour_ast(source);
    let target = extract_colour_ast(target);
    let source_tokens = counts(source.tokens);
    let target_tokens = counts(target.tokens);

    // The general token guard reports missing, changed, or additional codes.
    // The AST comparison is useful only when both sides contain the same codes.
    if source_tokens.is_empty() || source_tokens != target_tokens {
        return Vec::new();
    }

    let mut warnings = Vec::new();
    if counts(source.spans) != counts(target.spans) {
        warnings.push("颜色/样式码的生效作用域发生变化".into());
    }

    let source_anomalies = counts(source.anomalies);
    let target_anomalies = counts(target.anomalies);
    let extra_anomalies = target_anomalies.iter().any(|(anomaly, target_count)| {
        *target_count > source_anomalies.get(anomaly).copied().unwrap_or_default()
    });
    if extra_anomalies {
        warnings.push("译文新增了无生效样式的重置码或末尾悬空样式码".into());
    }
    warnings
}

pub(crate) fn protect_for_translation(
    text: &str,
    glossary: Option<&glossary::Loaded>,
) -> (String, Vec<(String, String)>) {
    let (protected, mut tokens) = protect(text);
    let Some(glossary) = glossary else {
        return (protected, tokens);
    };
    let (protected, glossary_tokens) = glossary.protect(&protected);
    tokens.extend(glossary_tokens);
    (protected, tokens)
}

pub(crate) fn prepare_entry(
    id: String,
    source: String,
    entry_index: usize,
    glossary: Option<&glossary::Loaded>,
) -> (Entry, Vec<Item>) {
    if let Some(document) = rich_text::Document::parse(&source) {
        let mut items = Vec::new();
        let mut units = Vec::new();
        for (unit_index, unit) in document.units().iter().enumerate() {
            let unit_id = format!("__ftb_unit_{entry_index}_{unit_index}");
            let (protected, tokens) = protect_for_translation(&unit.source, glossary);
            items.push(Item {
                id: unit_id.clone(),
                entry_id: id.clone(),
                path: unit.pointer.clone(),
                source: unit.source.clone(),
                protected,
                tokens,
            });
            units.push((unit.pointer.clone(), unit_id));
        }
        return (
            Entry {
                id,
                source,
                kind: EntryKind::RichText { document, units },
            },
            items,
        );
    }
    if rich_text::looks_like_component(&source) {
        return (
            Entry {
                id,
                source,
                kind: EntryKind::Untouched(
                    "疑似 JSON 富文本无法安全解析或包含重复键，已保留原文".into(),
                ),
            },
            vec![],
        );
    }

    let unit_id = format!("__ftb_unit_{entry_index}_0");
    let (protected, tokens) = protect_for_translation(&source, glossary);
    let item = Item {
        id: unit_id.clone(),
        entry_id: id.clone(),
        path: "$".into(),
        source: source.clone(),
        protected,
        tokens,
    };
    (
        Entry {
            id,
            source,
            kind: EntryKind::Plain(unit_id),
        },
        vec![item],
    )
}

pub(crate) fn render_entry(
    entry: &Entry,
    results: &HashMap<String, String>,
) -> Result<String, String> {
    match &entry.kind {
        EntryKind::Untouched(_) => Ok(entry.source.clone()),
        EntryKind::Plain(unit_id) => results
            .get(unit_id)
            .cloned()
            .ok_or_else(|| format!("缺少翻译单元：{unit_id}")),
        EntryKind::RichText { document, units } => {
            if units.is_empty() {
                return Ok(entry.source.clone());
            }
            let translations = units
                .iter()
                .map(|(pointer, unit_id)| {
                    results
                        .get(unit_id)
                        .cloned()
                        .map(|target| (pointer.clone(), target))
                        .ok_or_else(|| format!("缺少 JSON 富文本翻译单元：{unit_id}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            document.render(&translations)
        }
    }
}
