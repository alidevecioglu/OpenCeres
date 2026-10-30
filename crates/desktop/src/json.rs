//! Just enough JSON for the GitHub release reply and the search history:
//! a small parser into [`Value`] and helpers for writing.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
}

/// `text` as a JSON string literal, quotes included.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `["a", "b"]` on one line.
pub fn string_array<S: AsRef<str>>(items: &[S]) -> String {
    let items: Vec<String> = items.iter().map(|s| quote(s.as_ref())).collect();
    format!("[{}]", items.join(", "))
}

pub fn parse(text: &str) -> Result<Value, ()> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    let v = p.value()?;
    p.ws();
    if p.i == p.s.len() { Ok(v) } else { Err(()) }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Result<(), ()> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(())
        }
    }

    fn value(&mut self) -> Result<Value, ()> {
        self.depth += 1;
        if self.depth > 64 {
            return Err(());
        }
        self.ws();
        let v = match self.s.get(self.i).ok_or(())? {
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        let key = self.string()?;
                        self.eat(b':')?;
                        fields.push((key, self.value()?));
                        self.ws();
                        match self.s.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(()),
                        }
                    }
                }
                Value::Object(fields)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                } else {
                    loop {
                        items.push(self.value()?);
                        self.ws();
                        match self.s.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(()),
                        }
                    }
                }
                Value::Array(items)
            }
            b'"' => Value::String(self.string()?),
            b't' => self.word("true", Value::Bool(true))?,
            b'f' => self.word("false", Value::Bool(false))?,
            b'n' => self.word("null", Value::Null)?,
            _ => {
                let start = self.i;
                while self.i < self.s.len()
                    && matches!(
                        self.s[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                let n = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| ())?;
                Value::Number(n.parse().map_err(|_| ())?)
            }
        };
        self.depth -= 1;
        Ok(v)
    }

    fn word(&mut self, w: &str, v: Value) -> Result<Value, ()> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(())
        }
    }

    fn string(&mut self) -> Result<String, ()> {
        if self.s.get(self.i) != Some(&b'"') {
            return Err(());
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            match *self.s.get(self.i).ok_or(())? {
                b'"' => {
                    self.i += 1;
                    return String::from_utf8(out).map_err(|_| ());
                }
                b'\\' => {
                    self.i += 1;
                    let c = *self.s.get(self.i).ok_or(())?;
                    self.i += 1;
                    match c {
                        b'"' | b'\\' | b'/' => out.push(c),
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code)
                                && self.s[self.i..].starts_with(b"\\u")
                            {
                                self.i += 2;
                                let low = self.hex4()?;
                                code = 0x10000
                                    + ((code - 0xD800) << 10)
                                    + (low.wrapping_sub(0xDC00) & 0x3ff);
                            }
                            let ch = char::from_u32(code).unwrap_or('\u{fffd}');
                            out.extend_from_slice(ch.to_string().as_bytes());
                        }
                        _ => return Err(()),
                    }
                }
                c => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, ()> {
        let digits = self.s.get(self.i..self.i + 4).ok_or(())?;
        self.i += 4;
        u32::from_str_radix(std::str::from_utf8(digits).map_err(|_| ())?, 16).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_strings_read_back() {
        let tricky = "Ayşe \"the comet\" \\ 1\n2\t3 \u{1} é 🚀";
        let v = parse(&quote(tricky)).unwrap();
        assert_eq!(v.as_str(), Some(tricky));
        let v = parse(&string_array(&["a", "b\"c"])).unwrap();
        let items: Vec<_> = v
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(items, ["a", "b\"c"]);
    }
}
