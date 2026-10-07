//! Order-preserving JSON with JSONC input (comments, trailing commas).
//!
//! The keypad daemon's config is hand-edited JSONC. Settings keeps the key
//! order and number spelling of everything it does not edit; comments are
//! dropped on save (the previous file is backed up first).

use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// Raw number token, written back exactly as read.
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn obj() -> Json {
        Json::Obj(Vec::new())
    }

    pub fn str(s: &str) -> Json {
        Json::Str(s.to_string())
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Json::Obj(entries) => entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Replaces an existing key in place (keeping its position) or appends it.
    pub fn set(&mut self, key: &str, value: Json) {
        if let Json::Obj(entries) = self {
            match entries.iter_mut().find(|(k, _)| k == key) {
                Some(entry) => entry.1 = value,
                None => entries.push((key.to_string(), value)),
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Json> {
        match self {
            Json::Obj(entries) => {
                let index = entries.iter().position(|(k, _)| k == key)?;
                Some(entries.remove(index).1)
            }
            _ => None,
        }
    }

    /// The object at `key`, created (or replaced if not an object) on demand.
    pub fn object_mut(&mut self, key: &str) -> &mut Json {
        if !matches!(self.get(key), Some(Json::Obj(_))) {
            self.set(key, Json::obj());
        }
        self.get_mut(key).expect("just inserted")
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(String, Json)] {
        match self {
            Json::Obj(entries) => entries,
            _ => &[],
        }
    }

    pub fn is_empty_container(&self) -> bool {
        matches!(self, Json::Obj(e) if e.is_empty()) || matches!(self, Json::Arr(a) if a.is_empty())
    }
}

// ── Parsing ──────────────────────────────────────────────────────────────────

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
}

pub fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser { text: text.as_bytes(), pos: 0 };
    p.skip()?;
    let value = p.value()?;
    p.skip()?;
    if p.pos < p.text.len() {
        return Err(p.error("unexpected text after the document"));
    }
    Ok(value)
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        let before = &self.text[..self.pos.min(self.text.len())];
        let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
        let col = before.iter().rev().take_while(|&&b| b != b'\n').count() + 1;
        format!("line {line}, column {col}: {what}")
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    /// Skips whitespace and comments.
    fn skip(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'/') if self.text.get(self.pos + 1) == Some(&b'/') => {
                    while self.peek().is_some_and(|b| b != b'\n') {
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.text.get(self.pos + 1) == Some(&b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        match self.peek() {
                            None => {
                                self.pos = start;
                                return Err(self.error("unterminated comment"));
                            }
                            Some(b'*') if self.text.get(self.pos + 1) == Some(&b'/') => {
                                self.pos += 2;
                                break;
                            }
                            _ => self.pos += 1,
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("expected a value")),
            None => Err(self.error("unexpected end of file")),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.text[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("expected a value"))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E'))
        {
            self.pos += 1;
        }
        let token = std::str::from_utf8(&self.text[start..self.pos]).unwrap_or_default();
        if token.parse::<f64>().is_err() {
            self.pos = start;
            return Err(self.error("invalid number"));
        }
        Ok(Json::Num(token.to_string()))
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return String::from_utf8(out).map_err(|_| self.error("invalid UTF-8"));
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let escaped = match self.peek() {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            let first = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&first)
                                && self.text[self.pos + 1..].starts_with(b"\\u")
                            {
                                self.pos += 2;
                                let second = self.hex4()?;
                                0x10000 + ((first - 0xD800) << 10) + (second.wrapping_sub(0xDC00) & 0x3FF)
                            } else {
                                first
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.error("invalid escape")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(escaped.encode_utf8(&mut buf).as_bytes());
                    self.pos += 1;
                }
                Some(b) => {
                    out.push(b);
                    self.pos += 1;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .text
            .get(self.pos + 1..self.pos + 5)
            .ok_or_else(|| self.error("short \\u escape"))?;
        let value = std::str::from_utf8(digits)
            .ok()
            .and_then(|d| u32::from_str_radix(d, 16).ok())
            .ok_or_else(|| self.error("invalid \\u escape"))?;
        self.pos += 4;
        Ok(value)
    }

    fn array(&mut self) -> Result<Json, String> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip()?;
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(Json::Arr(items));
            }
            items.push(self.value()?);
            self.skip()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {}
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.pos += 1;
        let mut entries: Vec<(String, Json)> = Vec::new();
        loop {
            self.skip()?;
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(entries));
                }
                Some(b'"') => {}
                _ => return Err(self.error("expected a quoted key or '}'")),
            }
            let key = self.string()?;
            self.skip()?;
            if self.peek() != Some(b':') {
                return Err(self.error("expected ':'"));
            }
            self.pos += 1;
            self.skip()?;
            let value = self.value()?;
            match entries.iter_mut().find(|(k, _)| *k == key) {
                Some(entry) => entry.1 = value, // the last duplicate wins, as in Qt
                None => entries.push((key, value)),
            }
            self.skip()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {}
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }
}

// ── Writing ──────────────────────────────────────────────────────────────────

const INLINE_WIDTH: usize = 100;

fn escape(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Single-line form: `{ "a": 1, "b": ["x", "y"] }`.
pub fn to_compact(value: &Json) -> String {
    let mut out = String::new();
    compact(value, &mut out);
    out
}

fn compact(value: &Json, out: &mut String) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(n) => out.push_str(n),
        Json::Str(s) => escape(out, s),
        Json::Arr(items) if items.is_empty() => out.push_str("[]"),
        Json::Obj(entries) if entries.is_empty() => out.push_str("{}"),
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                compact(item, out);
            }
            out.push(']');
        }
        Json::Obj(entries) => {
            out.push_str("{ ");
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                escape(out, k);
                out.push_str(": ");
                compact(v, out);
            }
            out.push_str(" }");
        }
    }
}

/// Pretty form with 4-space indents; short nested containers stay on one line.
pub fn to_pretty(value: &Json) -> String {
    let mut out = String::new();
    pretty(value, 0, &mut out);
    out.push('\n');
    out
}

fn pretty(value: &Json, depth: usize, out: &mut String) {
    if !matches!(value, Json::Arr(_) | Json::Obj(_)) || value.is_empty_container() {
        compact(value, out);
        return;
    }
    let indent = "    ".repeat(depth);
    if depth > 1 {
        let line = to_compact(value);
        if line.len() + indent.len() <= INLINE_WIDTH {
            out.push_str(&line);
            return;
        }
    }
    let inner = "    ".repeat(depth + 1);
    let (open, close, count) = match value {
        Json::Arr(items) => ('[', ']', items.len()),
        Json::Obj(entries) => ('{', '}', entries.len()),
        _ => unreachable!(),
    };
    out.push(open);
    out.push('\n');
    for i in 0..count {
        out.push_str(&inner);
        match value {
            Json::Arr(items) => pretty(&items[i], depth + 1, out),
            Json::Obj(entries) => {
                escape(out, &entries[i].0);
                out.push_str(": ");
                pretty(&entries[i].1, depth + 1, out);
            }
            _ => unreachable!(),
        }
        out.push_str(if i + 1 < count { ",\n" } else { "\n" });
    }
    out.push_str(&indent);
    out.push(close);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jsonc_and_keeps_order_and_number_spelling() {
        let doc = parse(
            "// header\n{\n  \"z\": 1.50, /* inline */ \"a\": [1, 2,],\n  \"s\": \"a\\\"b\\u00e9 // not a comment\",\n}\n",
        )
        .unwrap();
        let keys: Vec<&str> = doc.entries().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["z", "a", "s"]);
        assert_eq!(doc.get("z"), Some(&Json::Num("1.50".into())));
        assert_eq!(doc.get("s").and_then(Json::as_str), Some("a\"bé // not a comment"));
        assert_eq!(
            to_compact(&doc),
            r#"{ "z": 1.50, "a": [1, 2], "s": "a\"bé // not a comment" }"#
        );
    }

    #[test]
    fn reports_errors_with_line_and_column() {
        let err = parse("{\n  \"a\": tru\n}").unwrap_err();
        assert!(err.starts_with("line 2, column 8"), "{err}");
        assert!(parse("{\"a\": 1} x").is_err());
        assert!(parse("/* open").is_err());
        assert!(parse("{\"a\" 1}").is_err());
    }

    #[test]
    fn pretty_output_round_trips_and_inlines_short_leaves() {
        let text = r#"{"device":{"vendor":"1189"},"profiles":[{"name":"global","bindings":{"key1":"ctrl+z","knob1":{"ccw":"volumedown","cw":"volumeup"}}}]}"#;
        let doc = parse(text).unwrap();
        let out = to_pretty(&doc);
        assert_eq!(parse(&out).unwrap(), doc);
        assert!(
            out.contains(r#""knob1": { "ccw": "volumedown", "cw": "volumeup" }"#),
            "{out}"
        );
        assert!(out.starts_with("{\n    \"device\": {\n"), "{out}");
    }

    #[test]
    fn set_replaces_in_place_and_remove_deletes() {
        let mut doc = parse(r#"{"a":1,"b":2}"#).unwrap();
        doc.set("a", Json::str("x"));
        doc.set("c", Json::Bool(true));
        assert_eq!(to_compact(&doc), r#"{ "a": "x", "b": 2, "c": true }"#);
        assert_eq!(doc.remove("b"), Some(Json::Num("2".into())));
        assert!(doc.remove("missing").is_none());
        doc.object_mut("o").set("k", Json::Null);
        assert_eq!(to_compact(&doc), r#"{ "a": "x", "c": true, "o": { "k": null } }"#);
    }
}
