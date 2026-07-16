use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
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

#[derive(Debug)]
struct StringToken {
    start: usize,
    end: usize,
    quote: char,
    value: String,
}

fn quoted_token(text: &str, start: usize) -> Result<StringToken, String> {
    let quote = text[start..]
        .chars()
        .next()
        .filter(|character| matches!(character, '\'' | '"'))
        .ok_or_else(|| format!("章节 SNBT 在偏移 {start} 处需要引号字符串"))?;
    let mut position = start + quote.len_utf8();
    let content_start = position;
    let mut escaped = false;
    while position < text.len() {
        let character = text[position..]
            .chars()
            .next()
            .ok_or_else(|| "章节 SNBT 字符串意外结束".to_string())?;
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote {
            return Ok(StringToken {
                start,
                end: position + character.len_utf8(),
                quote,
                value: decode(&text[content_start..position]),
            });
        }
        position += character.len_utf8();
    }
    Err(format!("章节 SNBT 在偏移 {start} 处的字符串没有结束引号"))
}

fn skip_trivia(text: &str, mut position: usize) -> usize {
    loop {
        while position < text.len() {
            let character = text[position..].chars().next().unwrap();
            if !character.is_whitespace() {
                break;
            }
            position += character.len_utf8();
        }
        if text[position..].starts_with("//") || text[position..].starts_with('#') {
            position = text[position..]
                .find('\n')
                .map_or(text.len(), |offset| position + offset + 1);
            continue;
        }
        return position;
    }
}

fn bare_token_end(text: &str, mut position: usize) -> usize {
    while position < text.len() {
        let character = text[position..].chars().next().unwrap();
        if character.is_whitespace()
            || matches!(character, '{' | '}' | '[' | ']' | '(' | ')' | ':' | ',')
            || text[position..].starts_with("//")
            || character == '#'
        {
            break;
        }
        position += character.len_utf8();
    }
    position
}

fn is_translatable_key(value: &str) -> bool {
    matches!(
        value,
        "title" | "subtitle" | "description" | "text" | "name"
    )
}

fn push_segment(path: &Path, key: &str, token: StringToken, output: &mut Vec<Segment>) {
    if !token
        .value
        .chars()
        .any(|character| character.is_ascii_alphabetic())
    {
        return;
    }
    let index = output.len();
    output.push(Segment {
        path: path.to_path_buf(),
        key: key.into(),
        source: token.value,
        start: token.start,
        end: token.end,
        quote: token.quote,
        index,
        cache_id: format!(
            "{}:{index}:{key}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ),
    });
}

fn collect_list_strings(
    text: &str,
    start: usize,
    path: &Path,
    key: &str,
    output: &mut Vec<Segment>,
) -> Result<usize, String> {
    let mut stack = vec!['['];
    let mut position = start + 1;
    while position < text.len() {
        position = skip_trivia(text, position);
        if position >= text.len() {
            break;
        }
        let character = text[position..].chars().next().unwrap();
        if matches!(character, '\'' | '"') {
            let token = quoted_token(text, position)?;
            position = token.end;
            if stack.as_slice() == ['['] {
                push_segment(path, key, token, output);
            }
            continue;
        }
        match character {
            '[' | '{' | '(' => stack.push(character),
            ']' | '}' | ')' => {
                let expected = match character {
                    ']' => '[',
                    '}' => '{',
                    ')' => '(',
                    _ => unreachable!(),
                };
                if stack.pop() != Some(expected) {
                    return Err(format!("章节 SNBT 的 {character} 没有匹配的起始括号"));
                }
                if stack.is_empty() {
                    return Ok(position + character.len_utf8());
                }
            }
            _ => {}
        }
        position += character.len_utf8();
    }
    Err("章节 SNBT 的目标字符串列表没有结束括号".into())
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
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    validate_structure(&text)?;
    let mut output = Vec::new();
    let mut position = 0;
    while position < text.len() {
        position = skip_trivia(&text, position);
        if position >= text.len() {
            break;
        }
        let character = text[position..].chars().next().unwrap();
        let (key, token_end) = if matches!(character, '\'' | '"') {
            let token = quoted_token(&text, position)?;
            (token.value, token.end)
        } else if matches!(character, '{' | '}' | '[' | ']' | '(' | ')' | ':' | ',') {
            position += character.len_utf8();
            continue;
        } else {
            let end = bare_token_end(&text, position);
            if end == position {
                position += character.len_utf8();
                continue;
            }
            (text[position..end].to_string(), end)
        };
        let colon = skip_trivia(&text, token_end);
        if !is_translatable_key(&key) || colon >= text.len() || !text[colon..].starts_with(':') {
            position = token_end;
            continue;
        }
        let value = skip_trivia(&text, colon + 1);
        if value >= text.len() {
            return Err(format!("章节字段 {key} 缺少值"));
        }
        let value_start = text[value..].chars().next().unwrap();
        if matches!(value_start, '\'' | '"') {
            let token = quoted_token(&text, value)?;
            position = token.end;
            push_segment(path, &key, token, &mut output);
        } else if value_start == '[' {
            collect_list_strings(&text, value, path, &key, &mut output)?;
            // Keep walking inside the list so rich/nested components such as
            // `description: [{ text: "..." }]` are found as normal fields.
            position = value + 1;
        } else {
            position = value;
        }
    }
    output.sort_by_key(|segment| segment.start);
    for (index, segment) in output.iter_mut().enumerate() {
        segment.index = index;
        segment.cache_id = format!(
            "{}:{index}:{}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            segment.key
        );
    }
    Ok(output)
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
    fn token_walker_ignores_field_names_and_comment_markers_inside_strings() {
        let d = tempdir().unwrap();
        let p = d.path().join("a.snbt");
        fs::write(
            &p,
            r#"{
  description: ["Visit https://example.com/a]b", "Line two"], title: "Real title",
  note: "title: \"Not a field\"",
  // subtitle: "Commented"
  "name": 'Quoted key'
}"#,
        )
        .unwrap();

        let segments = extract(&p).unwrap();
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.source.as_str())
                .collect::<Vec<_>>(),
            [
                "Visit https://example.com/a]b",
                "Line two",
                "Real title",
                "Quoted key"
            ]
        );
    }

    #[test]
    fn token_walker_finds_target_fields_in_nested_compounds() {
        let d = tempdir().unwrap();
        let p = d.path().join("nested.snbt");
        fs::write(
            &p,
            r#"{ group: { quests: [{ title: "First" }, { subtitle: 'Second' }] } }"#,
        )
        .unwrap();

        let segments = extract(&p).unwrap();
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.source.as_str())
                .collect::<Vec<_>>(),
            ["First", "Second"]
        );
    }

    #[test]
    fn token_walker_finds_rich_text_fields_inside_target_lists_in_source_order() {
        let d = tempdir().unwrap();
        let p = d.path().join("rich-list.snbt");
        fs::write(
            &p,
            r#"{ description: [{ text: "Nested first" }, "Direct second"] }"#,
        )
        .unwrap();

        let segments = extract(&p).unwrap();
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.source.as_str())
                .collect::<Vec<_>>(),
            ["Nested first", "Direct second"]
        );
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.index)
                .collect::<Vec<_>>(),
            [0, 1]
        );
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
