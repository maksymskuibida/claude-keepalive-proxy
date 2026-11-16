//! A small dependency-free JSON reader and writer.
//!
//! Objects keep their key order, numbers keep the text they were written with,
//! and strings are stored decoded and written back with minimal escaping.
#![allow(dead_code)]

use std::fmt;

const MAX_DEPTH: usize = 128;

/// A JSON value. Objects are an ordered list of key/value pairs.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number exactly as written in the source text.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// A parse failure: what went wrong and the byte offset where it happened.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub pos: usize,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid JSON at byte {}: {}", self.pos, self.msg)
    }
}

impl std::error::Error for ParseError {}

/// Parse a complete JSON document (one value, optional surrounding whitespace).
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
    };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("unexpected data after the value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> ParseError {
        ParseError {
            pos: self.i,
            msg: msg.to_string(),
        }
    }

    fn ws(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn lit(&mut self, word: &str, v: Value) -> Result<Value, ParseError> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err(self.err("expected ',' or ']'")),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut items: Vec<(String, Value)> = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(Value::Object(items));
                }
                loop {
                    self.ws();
                    if self.peek() != Some(b'"') {
                        return Err(self.err("expected a string key"));
                    }
                    let key = self.string()?;
                    self.ws();
                    if self.peek() != Some(b':') {
                        return Err(self.err("expected ':'"));
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    // Duplicate keys: the last one wins, in the first one's place.
                    if let Some(slot) = items.iter_mut().find(|(k, _)| *k == key) {
                        slot.1 = v;
                    } else {
                        items.push((key, v));
                    }
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Object(items));
                        }
                        _ => return Err(self.err("expected ',' or '}'")),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.err("expected a digit")),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if self.digits() == 0 {
                return Err(self.err("expected a digit after '.'"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.err("expected a digit in the exponent"));
            }
        }
        Ok(Value::Number(self.text[start..self.i].to_string()))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let mut n = 0u32;
        for _ in 0..4 {
            let d = match self.peek() {
                Some(c @ b'0'..=b'9') => c - b'0',
                Some(c @ b'a'..=b'f') => c - b'a' + 10,
                Some(c @ b'A'..=b'F') => c - b'A' + 10,
                _ => return Err(self.err("expected four hex digits")),
            };
            n = n * 16 + d as u32;
            self.i += 1;
        }
        Ok(n)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let run = self.i;
            while let Some(c) = self.peek() {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(&self.text[run..self.i]);
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let at = self.i;
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if self.s[self.i..].starts_with(b"\\u") {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        self.i = at;
                                        return Err(self.err("invalid surrogate pair"));
                                    }
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    self.i = at;
                                    return Err(self.err("lone surrogate"));
                                }
                            } else if (0xDC00..0xE000).contains(&hi) {
                                self.i = at;
                                return Err(self.err("lone surrogate"));
                            } else {
                                hi
                            };
                            match char::from_u32(cp) {
                                Some(ch) => out.push(ch),
                                None => return Err(self.err("invalid code point")),
                            }
                        }
                        _ => {
                            self.i -= 1;
                            return Err(self.err("invalid escape"));
                        }
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
            }
        }
    }
}

impl Value {
    /// The value under `key` if this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Mutable access to the value under `key`.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Object(items) => items.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Set `key`: replaces in place (keeping its position) or appends at the end.
    /// Returns false (and changes nothing) if this is not an object.
    pub fn set(&mut self, key: &str, value: Value) -> bool {
        match self {
            Value::Object(items) => {
                if let Some(slot) = items.iter_mut().find(|(k, _)| k == key) {
                    slot.1 = value;
                } else {
                    items.push((key.to_string(), value));
                }
                true
            }
            _ => false,
        }
    }

    /// Remove `key`, returning its value if it was there.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        match self {
            Value::Object(items) => {
                let at = items.iter().position(|(k, _)| k == key)?;
                Some(items.remove(at).1)
            }
            _ => None,
        }
    }

    /// Compact output, no whitespace.
    pub fn write(&self) -> String {
        let mut out = String::new();
        self.emit(&mut out, None, 0);
        out
    }

    /// Indented output (two spaces per level), no trailing newline.
    pub fn write_pretty(&self) -> String {
        let mut out = String::new();
        self.emit(&mut out, Some(2), 0);
        out
    }

    fn emit(&self, out: &mut String, indent: Option<usize>, level: usize) {
        let newline = |out: &mut String, level: usize| {
            if let Some(n) = indent {
                out.push('\n');
                out.extend(std::iter::repeat_n(' ', n * level));
            }
        };
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(n),
            Value::String(s) => write_string(out, s),
            Value::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (n, v) in items.iter().enumerate() {
                    if n > 0 {
                        out.push(',');
                    }
                    newline(out, level + 1);
                    v.emit(out, indent, level + 1);
                }
                newline(out, level);
                out.push(']');
            }
            Value::Object(items) => {
                if items.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (n, (k, v)) in items.iter().enumerate() {
                    if n > 0 {
                        out.push(',');
                    }
                    newline(out, level + 1);
                    write_string(out, k);
                    out.push(':');
                    if indent.is_some() {
                        out.push(' ');
                    }
                    v.emit(out, indent, level + 1);
                }
                newline(out, level);
                out.push('}');
            }
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(text: &str) -> String {
        parse(text).expect("valid json").write()
    }

    #[test]
    fn json_round_trips_nested_values() {
        let t = r#"{"a":[1,2,{"b":[true,false,null,[]]}],"c":{},"d":{"e":{"f":"g"}}}"#;
        assert_eq!(round(t), t);
        assert_eq!(parse(&format!(" \n{t}\t ")).unwrap().write(), t);
    }

    #[test]
    fn json_round_trips_scalars() {
        for t in ["null", "true", "false", "\"x\"", "0", "[]", "{}"] {
            assert_eq!(round(t), t);
        }
    }

    #[test]
    fn json_keeps_key_order() {
        let t = r#"{"zeta":1,"alpha":2,"mid":{"y":1,"b":2},"beta":3}"#;
        assert_eq!(round(t), t);
        match parse(t).unwrap() {
            Value::Object(items) => {
                let keys: Vec<&str> = items.iter().map(|(k, _)| k.as_str()).collect();
                assert_eq!(keys, ["zeta", "alpha", "mid", "beta"]);
            }
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn json_keeps_numbers_as_written() {
        let t = "[0,-0,1.50,1e5,1E+5,-2.5e-3,12345678901234567890123,0.1000000000000000055511151231257827]";
        assert_eq!(round(t), t.to_string());
    }

    #[test]
    fn json_handles_unicode_and_escapes() {
        let v = parse(r#""caf\u00e9 \ud83d\ude00 \n\t\"\\\/ \b\f\r""#).unwrap();
        assert_eq!(
            v,
            Value::String("caf\u{e9} \u{1F600} \n\t\"\\/ \u{8}\u{c}\r".to_string())
        );
        let again = parse(&v.write()).unwrap();
        assert_eq!(again, v);
        assert_eq!(round("\"日本語 ключ 😀\""), "\"日本語 ключ 😀\"");
        assert_eq!(Value::String("\u{1}".into()).write(), "\"\\u0001\"");
        assert_eq!(round(r#"{"k\u0041":"v"}"#), r#"{"kA":"v"}"#);
    }

    #[test]
    fn json_pretty_output_parses_back() {
        let v = parse(r#"{"a":[1,{"b":null}],"c":"d","e":[],"f":{}}"#).unwrap();
        let pretty = v.write_pretty();
        assert!(pretty.contains("\n  \"a\": ["));
        assert_eq!(parse(&pretty).unwrap(), v);
    }

    #[test]
    fn json_get_set_remove() {
        let mut v = parse(r#"{"one":1,"two":2,"three":3}"#).unwrap();
        assert_eq!(v.get("two"), Some(&Value::Number("2".into())));
        assert_eq!(v.get("nope"), None);
        assert!(v.set("two", Value::Bool(true)));
        assert!(v.set("four", Value::Null));
        assert_eq!(v.write(), r#"{"one":1,"two":true,"three":3,"four":null}"#);
        assert_eq!(v.remove("one"), Some(Value::Number("1".into())));
        assert_eq!(v.remove("one"), None);
        assert_eq!(v.write(), r#"{"two":true,"three":3,"four":null}"#);
        let mut not_obj = Value::Null;
        assert!(!not_obj.set("a", Value::Null));
        assert_eq!(not_obj.remove("a"), None);
        assert_eq!(not_obj.get("a"), None);
        if let Some(Value::Bool(b)) = v.get_mut("two") {
            *b = false;
        }
        assert_eq!(v.get("two"), Some(&Value::Bool(false)));
    }

    #[test]
    fn json_duplicate_key_last_wins_in_first_place() {
        assert_eq!(round(r#"{"a":1,"b":2,"a":3}"#), r#"{"a":3,"b":2}"#);
    }

    #[test]
    fn json_refuses_invalid_input() {
        let bad = [
            "",
            "   ",
            "{",
            "[1,]",
            "[1 2]",
            "{\"a\":1,}",
            "{a:1}",
            "{\"a\" 1}",
            "'x'",
            "01",
            "1.",
            ".5",
            "-",
            "1e",
            "+1",
            "nul",
            "truex",
            "\"abc",
            "\"a\nb\"",
            "\"\\x\"",
            "\"\\u12\"",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "\"\\ud800\\u0041\"",
            "1 2",
            "[1]]",
        ];
        for t in bad {
            assert!(parse(t).is_err(), "should refuse {t:?}");
        }
    }

    #[test]
    fn json_error_names_the_position() {
        let e = parse("[1, 2, x]").unwrap_err();
        assert_eq!(e.pos, 7);
        assert!(e.to_string().contains("byte 7"));
        let e = parse("{\"a\":1} extra").unwrap_err();
        assert_eq!(e.pos, 8);
        assert_eq!(parse("").unwrap_err().pos, 0);
    }

    #[test]
    fn json_refuses_excess_nesting() {
        let deep = format!("{}{}", "[".repeat(1000), "]".repeat(1000));
        assert!(parse(&deep).is_err());
        let ok = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(parse(&ok).is_ok());
    }
}
