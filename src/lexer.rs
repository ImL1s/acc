use crate::token::{Token, TokenKind};

pub struct Lexer<'a> {
    src: &'a [u8],
    i: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            i: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.i).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.src.get(self.i + 1).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_ws_and_directives(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.bump();
            }
            if self.peek() == Some(b'/') && self.peek2() == Some(b'/') {
                while self.peek().is_some() && self.peek() != Some(b'\n') {
                    self.bump();
                }
                continue;
            }
            if self.peek() == Some(b'/') && self.peek2() == Some(b'*') {
                self.bump();
                self.bump();
                while let Some(c) = self.bump() {
                    if c == b'*' && self.peek() == Some(b'/') {
                        self.bump();
                        break;
                    }
                }
                continue;
            }
            // Skip full preprocessor lines (#include, #define, ...)
            if self.peek() == Some(b'#') {
                while let Some(c) = self.peek() {
                    if c == b'\n' {
                        break;
                    }
                    if c == b'\\' {
                        self.bump();
                        if self.peek() == Some(b'\n') {
                            self.bump();
                        }
                        continue;
                    }
                    self.bump();
                }
                continue;
            }
            break;
        }
    }

    fn make(&self, kind: TokenKind, line: usize, col: usize) -> Token {
        Token { kind, line, col }
    }

    fn ident_or_kw(&mut self) -> Token {
        let line = self.line;
        let col = self.col;
        let start = self.i;
        while matches!(self.peek(), Some(b) if b.is_ascii_alphanumeric() || b == b'_') {
            self.bump();
        }
        let s = std::str::from_utf8(&self.src[start..self.i]).unwrap();
        let kind = match s {
            "int" => TokenKind::Int,
            "void" => TokenKind::Void,
            "char" => TokenKind::Char,
            "long" => TokenKind::Long,
            "short" => TokenKind::Short,
            "float" => TokenKind::Float,
            "double" => TokenKind::Double,
            "struct" => TokenKind::Struct,
            "union" => TokenKind::Union,
            "typedef" => TokenKind::Typedef,
            "enum" => TokenKind::Enum,
            "unsigned" => TokenKind::Unsigned,
            "signed" => TokenKind::Signed,
            "static" => TokenKind::Static,
            "extern" => TokenKind::Extern,
            "register" => TokenKind::Register,
            "inline" => TokenKind::Inline,
            "__inline" => TokenKind::Inline,
            "__inline__" => TokenKind::Inline,
            "restrict" => TokenKind::Restrict,
            "__restrict" => TokenKind::Restrict,
            "__restrict__" => TokenKind::Restrict,
            "auto" => TokenKind::Auto,
            "const" => TokenKind::Const,
            "volatile" => TokenKind::Volatile,
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "do" => TokenKind::Do,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "goto" => TokenKind::Goto,
            "switch" => TokenKind::Switch,
            "case" => TokenKind::Case,
            "default" => TokenKind::Default,
            "sizeof" => TokenKind::Sizeof,
            "_Bool" | "bool" => TokenKind::Int, // treat as int for suite smoke
            // Mark GNU attributes as a special ident for next_token to erase.
            "__attribute__" | "__attribute" | "__extension__" => {
                TokenKind::Ident(s.to_string())
            }
            _ => TokenKind::Ident(s.to_string()),
        };
        self.make(kind, line, col)
    }

    /// Skip `__attribute__((...))` balanced paren group after the keyword was consumed.
    fn skip_gnu_attribute_suffix(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.bump();
        }
        if self.peek() != Some(b'(') {
            return;
        }
        let mut depth = 0i32;
        while let Some(c) = self.peek() {
            if c == b'(' {
                depth += 1;
                self.bump();
            } else if c == b')' {
                self.bump();
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else if c == b'"' || c == b'\'' {
                let quote = c;
                self.bump();
                while let Some(c2) = self.peek() {
                    self.bump();
                    if c2 == b'\\' {
                        let _ = self.bump();
                        continue;
                    }
                    if c2 == quote {
                        break;
                    }
                }
            } else {
                self.bump();
            }
        }
    }

    fn number(&mut self) -> Token {
        let line = self.line;
        let col = self.col;
        let start = self.i;
        // hex
        if self.peek() == Some(b'0')
            && matches!(self.peek2(), Some(b'x' | b'X'))
        {
            self.bump();
            self.bump();
            while matches!(self.peek(), Some(b) if b.is_ascii_hexdigit()) {
                self.bump();
            }
            let s = std::str::from_utf8(&self.src[start + 2..self.i]).unwrap_or("0");
            // large hex constants like 0xffffffff
            let n = u64::from_str_radix(s, 16).unwrap_or(0) as i64;
            // optional integer suffixes: u Ul LL ULL llu etc.
            while matches!(self.peek(), Some(b'u' | b'U' | b'l' | b'L')) {
                self.bump();
            }
            return self.make(TokenKind::IntLit(n), line, col);
        }
        // leading 0 may be octal (0123) unless it's a float (0.5 / 0e1)
        let leading_zero = self.peek() == Some(b'0');
        while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
            self.bump();
        }
        // float: 1.0 or 1. or 1e10
        let is_float = self.peek() == Some(b'.')
            || matches!(self.peek(), Some(b'e' | b'E'));
        if is_float {
            if self.peek() == Some(b'.') {
                self.bump();
                while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                    self.bump();
                }
            }
            if matches!(self.peek(), Some(b'e' | b'E')) {
                self.bump();
                if matches!(self.peek(), Some(b'+' | b'-')) {
                    self.bump();
                }
                while matches!(self.peek(), Some(b) if b.is_ascii_digit()) {
                    self.bump();
                }
            }
            while matches!(self.peek(), Some(b'f' | b'F' | b'l' | b'L')) {
                self.bump();
            }
            let s = std::str::from_utf8(&self.src[start..self.i]).unwrap();
            let cleaned: String = s
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == 'e' || *c == 'E' || *c == '+' || *c == '-')
                .collect();
            let f: f64 = cleaned.parse().unwrap_or(0.0);
            // encode as IntLit of bits? Use StringLit-like - store as FloatLit via IntLit with tag
            // Use TokenKind::IntLit with bit cast for simplicity - better add FloatLit
            return self.make(TokenKind::FloatLit(f), line, col);
        }
        // skip optional ul UL suffixes
        while matches!(self.peek(), Some(b'u' | b'U' | b'l' | b'L')) {
            self.bump();
        }
        let s = std::str::from_utf8(&self.src[start..self.i]).unwrap();
        let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        let n: i64 = if leading_zero && digits.len() > 1 {
            // octal: 01234567
            i64::from_str_radix(&digits, 8).unwrap_or(0)
        } else {
            digits.parse().unwrap_or(0)
        };
        self.make(TokenKind::IntLit(n), line, col)
    }

    fn string(&mut self) -> Result<Token, String> {
        let line = self.line;
        let col = self.col;
        let mut out = String::new();
        // Adjacent string literal concatenation: "a" "b" => "ab"
        loop {
            self.bump(); // opening "
            while let Some(c) = self.bump() {
                if c == b'"' {
                    break;
                }
                if c == b'\\' {
                    let e = self
                        .bump()
                        .ok_or_else(|| format!("unterminated escape at {line}:{col}"))?;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'\\' => out.push('\\'),
                        b'"' => out.push('"'),
                        b'\'' => out.push('\''),
                        b'x' | b'X' => {
                            let mut val = 0u8;
                            let mut digits = 0;
                            while digits < 2 {
                                match self.peek() {
                                    Some(d) if d.is_ascii_hexdigit() => {
                                        self.bump();
                                        val = val.wrapping_mul(16).wrapping_add(match d {
                                            b'0'..=b'9' => d - b'0',
                                            b'a'..=b'f' => d - b'a' + 10,
                                            b'A'..=b'F' => d - b'A' + 10,
                                            _ => 0,
                                        });
                                        digits += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(val as char);
                        }
                        b'0'..=b'7' => {
                            let mut val = e - b'0';
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        self.bump();
                                        val = val * 8 + (d - b'0');
                                    }
                                    _ => break,
                                }
                            }
                            out.push(val as char);
                        }
                        other => out.push(other as char),
                    }
                } else {
                    out.push(c as char);
                }
            }
            // skip whitespace between strings
            let save_i = self.i;
            let save_line = self.line;
            let save_col = self.col;
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.bump();
            }
            if self.peek() == Some(b'"') {
                continue;
            }
            // restore if not another string
            self.i = save_i;
            self.line = save_line;
            self.col = save_col;
            return Ok(self.make(TokenKind::StringLit(out), line, col));
        }
    }

    fn char_lit(&mut self) -> Result<Token, String> {
        let line = self.line;
        let col = self.col;
        // wide char L'\0'
        if self.peek() == Some(b'L') && self.peek2() == Some(b'\'') {
            self.bump();
        }
        self.bump(); // '
        let c = self
            .bump()
            .ok_or_else(|| format!("unterminated char at {line}:{col}"))?;
        let v = if c == b'\\' {
            let e = self
                .bump()
                .ok_or_else(|| format!("unterminated char escape at {line}:{col}"))?;
            match e {
                b'n' => b'\n' as i64,
                b't' => b'\t' as i64,
                b'r' => b'\r' as i64,
                b'\\' => b'\\' as i64,
                b'\'' => b'\'' as i64,
                b'"' => b'"' as i64,
                b'x' | b'X' => {
                    // hex escape \xhh
                    let mut val = 0i64;
                    let mut digits = 0;
                    while digits < 2 {
                        match self.peek() {
                            Some(d) if d.is_ascii_hexdigit() => {
                                self.bump();
                                val = val * 16
                                    + match d {
                                        b'0'..=b'9' => (d - b'0') as i64,
                                        b'a'..=b'f' => (d - b'a' + 10) as i64,
                                        b'A'..=b'F' => (d - b'A' + 10) as i64,
                                        _ => 0,
                                    };
                                digits += 1;
                            }
                            _ => break,
                        }
                    }
                    val
                }
                b'0'..=b'7' => {
                    // octal escape \ooo (up to 3 digits, first already in e)
                    let mut val = (e - b'0') as i64;
                    for _ in 0..2 {
                        match self.peek() {
                            Some(d @ b'0'..=b'7') => {
                                self.bump();
                                val = val * 8 + (d - b'0') as i64;
                            }
                            _ => break,
                        }
                    }
                    val
                }
                other => other as i64,
            }
        } else {
            c as i64
        };
        if self.bump() != Some(b'\'') {
            return Err(format!("expected closing ' at {line}:{col}"));
        }
        Ok(self.make(TokenKind::CharLit(v), line, col))
    }

    pub fn next_token(&mut self) -> Result<Token, String> {
        loop {
            self.skip_ws_and_directives();
            let line = self.line;
            let col = self.col;
            let Some(c) = self.peek() else {
                return Ok(self.make(TokenKind::Eof, line, col));
            };
            // wide char/string: L'x' / L"..." (before general ident path)
            if c == b'L'
                && (self.peek2() == Some(b'\'') || self.peek2() == Some(b'"'))
            {
                return if self.peek2() == Some(b'\'') {
                    self.char_lit()
                } else {
                    self.bump(); // L
                    self.string()
                };
            }
            // Erase GNU __attribute__((...)) / __extension__ from the token stream.
            if c.is_ascii_alphabetic() || c == b'_' {
                let t = self.ident_or_kw();
                if let TokenKind::Ident(ref s) = t.kind {
                    if s == "__attribute__" || s == "__attribute" {
                        self.skip_gnu_attribute_suffix();
                        continue;
                    }
                    if s == "__extension__" {
                        continue;
                    }
                }
                return Ok(t);
            }
            return self.next_token_punct(c, line, col);
        }
    }

    fn next_token_punct(&mut self, c: u8, line: usize, col: usize) -> Result<Token, String> {
        match c {
            b'(' => {
                self.bump();
                Ok(self.make(TokenKind::LParen, line, col))
            }
            b')' => {
                self.bump();
                Ok(self.make(TokenKind::RParen, line, col))
            }
            b'{' => {
                self.bump();
                Ok(self.make(TokenKind::LBrace, line, col))
            }
            b'}' => {
                self.bump();
                Ok(self.make(TokenKind::RBrace, line, col))
            }
            b'[' => {
                self.bump();
                Ok(self.make(TokenKind::LBracket, line, col))
            }
            b']' => {
                self.bump();
                Ok(self.make(TokenKind::RBracket, line, col))
            }
            b';' => {
                self.bump();
                Ok(self.make(TokenKind::Semicolon, line, col))
            }
            b',' => {
                self.bump();
                Ok(self.make(TokenKind::Comma, line, col))
            }
            b':' => {
                self.bump();
                Ok(self.make(TokenKind::Colon, line, col))
            }
            b'?' => {
                self.bump();
                Ok(self.make(TokenKind::Question, line, col))
            }
            b'~' => {
                self.bump();
                Ok(self.make(TokenKind::Tilde, line, col))
            }
            b'.' => {
                if self.peek2() == Some(b'.') && self.src.get(self.i + 2) == Some(&b'.') {
                    self.bump();
                    self.bump();
                    self.bump();
                    Ok(self.make(TokenKind::Ellipsis, line, col))
                } else {
                    self.bump();
                    Ok(self.make(TokenKind::Dot, line, col))
                }
            }
            b'+' => {
                self.bump();
                if self.peek() == Some(b'+') {
                    self.bump();
                    Ok(self.make(TokenKind::PlusPlus, line, col))
                } else if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::PlusEq, line, col))
                } else {
                    Ok(self.make(TokenKind::Plus, line, col))
                }
            }
            b'-' => {
                self.bump();
                if self.peek() == Some(b'-') {
                    self.bump();
                    Ok(self.make(TokenKind::MinusMinus, line, col))
                } else if self.peek() == Some(b'>') {
                    self.bump();
                    Ok(self.make(TokenKind::Arrow, line, col))
                } else if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::MinusEq, line, col))
                } else {
                    Ok(self.make(TokenKind::Minus, line, col))
                }
            }
            b'*' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::StarEq, line, col))
                } else {
                    Ok(self.make(TokenKind::Star, line, col))
                }
            }
            b'/' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::SlashEq, line, col))
                } else {
                    Ok(self.make(TokenKind::Slash, line, col))
                }
            }
            b'%' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::PercentEq, line, col))
                } else {
                    Ok(self.make(TokenKind::Percent, line, col))
                }
            }
            b'&' => {
                self.bump();
                if self.peek() == Some(b'&') {
                    self.bump();
                    Ok(self.make(TokenKind::AndAnd, line, col))
                } else if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::AndEq, line, col))
                } else {
                    Ok(self.make(TokenKind::Amp, line, col))
                }
            }
            b'|' => {
                self.bump();
                if self.peek() == Some(b'|') {
                    self.bump();
                    Ok(self.make(TokenKind::OrOr, line, col))
                } else if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::OrEq, line, col))
                } else {
                    Ok(self.make(TokenKind::Pipe, line, col))
                }
            }
            b'^' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::XorEq, line, col))
                } else {
                    Ok(self.make(TokenKind::Caret, line, col))
                }
            }
            b'!' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::Ne, line, col))
                } else {
                    Ok(self.make(TokenKind::Bang, line, col))
                }
            }
            b'=' => {
                self.bump();
                if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::Eq, line, col))
                } else {
                    Ok(self.make(TokenKind::Assign, line, col))
                }
            }
            b'<' => {
                self.bump();
                if self.peek() == Some(b'<') {
                    self.bump();
                    if self.peek() == Some(b'=') {
                        self.bump();
                        Ok(self.make(TokenKind::ShlEq, line, col))
                    } else {
                        Ok(self.make(TokenKind::Shl, line, col))
                    }
                } else if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::Le, line, col))
                } else {
                    Ok(self.make(TokenKind::Lt, line, col))
                }
            }
            b'>' => {
                self.bump();
                if self.peek() == Some(b'>') {
                    self.bump();
                    if self.peek() == Some(b'=') {
                        self.bump();
                        Ok(self.make(TokenKind::ShrEq, line, col))
                    } else {
                        Ok(self.make(TokenKind::Shr, line, col))
                    }
                } else if self.peek() == Some(b'=') {
                    self.bump();
                    Ok(self.make(TokenKind::Ge, line, col))
                } else {
                    Ok(self.make(TokenKind::Gt, line, col))
                }
            }
            b'"' => self.string(),
            b'\'' => self.char_lit(),
            b if b.is_ascii_digit() => Ok(self.number()),
            // alphabetic / wide literals handled in next_token
            other => Err(format!(
                "unexpected character {:?} at {line}:{col}",
                other as char
            )),
        }
    }

    pub fn tokenize(src: &str) -> Result<Vec<Token>, String> {
        let mut lx = Lexer::new(src);
        let mut toks = Vec::new();
        loop {
            let t = lx.next_token()?;
            let done = t.kind == TokenKind::Eof;
            toks.push(t);
            if done {
                break;
            }
        }
        Ok(toks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_include_and_lexes_main() {
        let src = "#include <stdio.h>\nint main(void) { return 0; }";
        let t = Lexer::tokenize(src).unwrap();
        assert!(t.iter().any(|x| matches!(&x.kind, TokenKind::Ident(s) if s == "main")));
    }

    #[test]
    fn skips_gnu_attribute() {
        let src = "int __attribute__((packed)) x;";
        let t = Lexer::tokenize(src).unwrap();
        let kinds: Vec<&TokenKind> = t.iter().map(|x| &x.kind).collect();
        assert!(
            !kinds.iter().any(|k| matches!(k, TokenKind::Ident(s) if s.contains("attribute"))),
            "attribute not erased: {kinds:?}"
        );
        assert!(matches!(kinds[0], TokenKind::Int));
        assert!(matches!(kinds[1], TokenKind::Ident(s) if s == "x"));
    }
}
