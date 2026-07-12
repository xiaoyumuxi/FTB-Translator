use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum LangValue {
    Text(String),
    Lines(Vec<String>),
}

pub type LangMap = Vec<(String, LangValue)>;

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}
impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }
    fn peek(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }
    fn skip(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
            if self.text[self.pos..].starts_with("//") || self.peek() == Some('#') {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.bump();
                }
            } else {
                break;
            }
        }
    }
    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.bump() == Some(c) {
            Ok(())
        } else {
            Err(format!("SNBT 在偏移 {} 处缺少 {c}", self.pos))
        }
    }
    fn string(&mut self) -> Result<String, String> {
        let q = self.bump().ok_or("字符串意外结束")?;
        if q != '\'' && q != '"' {
            return Err("需要引号字符串".into());
        }
        let mut out = String::new();
        while let Some(c) = self.bump() {
            if c == q {
                return Ok(out);
            }
            if c == '\\' {
                let e = self.bump().ok_or("转义序列不完整")?;
                out.push(match e {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    x => x,
                })
            } else {
                out.push(c)
            }
        }
        Err("字符串没有结束引号".into())
    }
    fn key(&mut self) -> Result<String, String> {
        if matches!(self.peek(), Some('\'' | '"')) {
            return self.string();
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| !c.is_whitespace() && c != ':') {
            self.bump();
        }
        if start == self.pos {
            Err("缺少 SNBT key".into())
        } else {
            Ok(self.text[start..self.pos].to_string())
        }
    }
    fn value(&mut self) -> Result<LangValue, String> {
        if matches!(self.peek(), Some('\'' | '"')) {
            Ok(LangValue::Text(self.string()?))
        } else if self.peek() == Some('[') {
            self.bump();
            let mut v = vec![];
            loop {
                self.skip();
                if self.peek() == Some(']') {
                    self.bump();
                    break;
                }
                v.push(self.string()?);
                self.skip();
                if self.peek() == Some(',') {
                    self.bump();
                }
            }
            Ok(LangValue::Lines(v))
        } else {
            Err("语言值必须是字符串或字符串数组".into())
        }
    }
    fn parse(mut self) -> Result<LangMap, String> {
        self.skip();
        self.expect('{')?;
        let mut out = vec![];
        loop {
            self.skip();
            if self.peek() == Some('}') {
                self.bump();
                break;
            }
            let k = self.key()?;
            self.skip();
            self.expect(':')?;
            self.skip();
            out.push((k, self.value()?));
            self.skip();
            if self.peek() == Some(',') {
                self.bump();
            }
        }
        Ok(out)
    }
}
pub fn parse(text: &str) -> Result<LangMap, String> {
    Parser::new(text.trim_start_matches('\u{feff}')).parse()
}
pub fn load(path: &Path) -> Result<LangMap, String> {
    parse(&fs::read_to_string(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?)
}
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
pub fn dump(values: &LangMap) -> String {
    let mut out = String::from("{\n");
    for (i, (k, v)) in values.iter().enumerate() {
        out.push_str(&format!("  \"{}\": ", escape(k)));
        match v {
            LangValue::Text(s) => out.push_str(&format!("\"{}\"", escape(s))),
            LangValue::Lines(lines) => {
                out.push_str("[\n");
                for (j, s) in lines.iter().enumerate() {
                    out.push_str(&format!(
                        "    \"{}\"{}\n",
                        escape(s),
                        if j + 1 < lines.len() { "," } else { "" }
                    ));
                }
                out.push_str("  ]");
            }
        }
        if i + 1 < values.len() {
            out.push(',')
        }
        out.push('\n')
    }
    out.push_str("}\n");
    out
}
pub fn write(path: &Path, values: &LangMap) -> Result<(), String> {
    let text = dump(values);
    parse(&text)?;
    fs::write(path, text).map_err(|e| format!("无法写入 {}：{e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let src = "{ title: \"Hello\", desc: [\"A\", \"B\"] }";
        let v = parse(src).unwrap();
        assert_eq!(parse(&dump(&v)).unwrap(), v)
    }
}
