use crate::ast::*;
use crate::token::{Token, TokenKind};

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    /// typedef names known at parse time
    typedefs: Vec<String>,
    typedef_map: std::collections::HashMap<String, Type>,
    /// named structs/unions
    structs: Vec<String>,
    unions: Vec<String>,
    /// struct/union field layouts recorded while parsing
    struct_fields: std::collections::HashMap<String, Vec<Field>>,
    /// Struct/union tags declared `__attribute__((packed))` (1-byte alignment).
    packed_structs: std::collections::HashSet<String>,
    /// scope-unique tag renames: (scope_id, tag) -> unique_name
    tag_scope: Vec<std::collections::HashMap<String, String>>,
    tag_serial: usize,
    pending_enum_globals: Vec<VarDecl>,
    /// Enumerator name → value (for `E = PREV` style enum initializers).
    enum_values: std::collections::HashMap<String, i64>,
    /// Nesting depth of GNU statement expressions `({...})`. Deep nesting from
    /// kernel do/while(0) macros can thrash the recursive parser; soft-skip.
    #[allow(dead_code)]
    stmt_expr_depth: u32,
    pub last_section: Option<String>,
    /// Names declared `__weak` without initializer in this TU — later
    /// definitions of the same name (e.g. version.c #include of
    /// version-timestamp.c) stay weak so version-timestamp.o can override.
    weak_names: std::collections::HashSet<String>,
    /// Block/function-scope variable types (params + locals). Used so
    /// `char buf[sizeof(p->field)]` can fold sizeof at parse time for array
    /// bounds. Innermost scope is last.
    local_types: Vec<std::collections::HashMap<String, Type>>,
    /// File-scope variable types. Needed for array bounds like
    /// `u8 zHeader[sizeof(aJournalMagic)+4]` where `aJournalMagic` is a
    /// static global — without this, sizeof folds to None → unwrap_or(0) →
    /// zero-length local → later `sqlite3OsWrite(..., sizeof(zHeader), ...)`
    /// writes 0 bytes and journal magic is never patched (SQLite savepoint
    /// rollback fails with SQLITE_DONE).
    global_types: std::collections::HashMap<String, Type>,
}

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        Self {
            toks,
            i: 0,
            typedefs: Vec::new(),
            typedef_map: std::collections::HashMap::new(),
            structs: Vec::new(),
            unions: Vec::new(),
            struct_fields: std::collections::HashMap::new(),
            packed_structs: std::collections::HashSet::new(),
            tag_scope: vec![std::collections::HashMap::new()],
            tag_serial: 0,
            pending_enum_globals: Vec::new(),
            enum_values: std::collections::HashMap::new(),
            stmt_expr_depth: 0,
            last_section: None,
            weak_names: std::collections::HashSet::new(),
            local_types: vec![std::collections::HashMap::new()],
            global_types: std::collections::HashMap::new(),
        }
    }

    /// Consume zero or more sticky `Packed` tokens (from `__attribute__((packed))`).
    fn eat_packed_attrs(&mut self) -> bool {
        let mut saw = false;
        while self.eat(TokenKind::Packed) {
            saw = true;
        }
        saw
    }

    fn eval_enum_const(&self, e: &Expr) -> Option<i64> {
        match e {
            Expr::Int(n) | Expr::Char(n) => Some(*n),
            Expr::Var(name) => self.enum_values.get(name).copied(),
            Expr::Unary { op, expr } => match op {
                UnaryOp::Neg => self.eval_enum_const(expr).map(|v| -v),
                UnaryOp::BitNot => self.eval_enum_const(expr).map(|v| !v),
                UnaryOp::Not => self.eval_enum_const(expr).map(|v| if v == 0 { 1 } else { 0 }),
                _ => None,
            },
            Expr::Binary { op, left, right } => {
                let l = self.eval_enum_const(left)?;
                let r = self.eval_enum_const(right)?;
                Some(match op {
                    BinOp::Add => l.wrapping_add(r),
                    BinOp::Sub => l.wrapping_sub(r),
                    BinOp::Mul => l.wrapping_mul(r),
                    BinOp::Div if r != 0 => l / r,
                    BinOp::Mod if r != 0 => l % r,
                    BinOp::BitOr => l | r,
                    BinOp::BitAnd => l & r,
                    BinOp::BitXor => l ^ r,
                    BinOp::Shl => l.wrapping_shl((r as u32) & 63),
                    BinOp::Shr => l.wrapping_shr((r as u32) & 63),
                    BinOp::Eq => (l == r) as i64,
                    BinOp::Ne => (l != r) as i64,
                    BinOp::Lt => (l < r) as i64,
                    BinOp::Gt => (l > r) as i64,
                    BinOp::Le => (l <= r) as i64,
                    BinOp::Ge => (l >= r) as i64,
                    BinOp::And => ((l != 0) && (r != 0)) as i64,
                    BinOp::Or => ((l != 0) || (r != 0)) as i64,
                    _ => return None,
                })
            }
            Expr::Cond { cond, then_e, else_e } => {
                let c = self.eval_enum_const(cond)?;
                if c != 0 {
                    self.eval_enum_const(then_e)
                } else {
                    self.eval_enum_const(else_e)
                }
            }
            Expr::Cast { expr, .. } => self.eval_enum_const(expr),
            // sizeof in enum constants — fall back to const_array_len
            other => self.const_array_len(other),
        }
    }

    fn push_scope(&mut self) {
        self.tag_scope.push(std::collections::HashMap::new());
        self.local_types.push(std::collections::HashMap::new());
    }
    fn pop_scope(&mut self) {
        if self.tag_scope.len() > 1 {
            self.tag_scope.pop();
        }
        if self.local_types.len() > 1 {
            self.local_types.pop();
        }
    }
    fn insert_local_type(&mut self, name: &str, ty: Type) {
        if name.is_empty() {
            return;
        }
        if let Some(scope) = self.local_types.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }
    fn insert_global_type(&mut self, name: &str, ty: Type) {
        if name.is_empty() {
            return;
        }
        // Prefer a sized array over a prior incomplete `[]` declaration.
        if let Type::Array(_, n) = &ty {
            if *n > 0 {
                self.global_types.insert(name.to_string(), ty);
                return;
            }
        }
        self.global_types.entry(name.to_string()).or_insert(ty);
    }
    fn lookup_local_type(&self, name: &str) -> Option<&Type> {
        for scope in self.local_types.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t);
            }
        }
        None
    }
    #[allow(dead_code)]
    fn lookup_var_type(&self, name: &str) -> Option<&Type> {
        if let Some(t) = self.lookup_local_type(name) {
            return Some(t);
        }
        self.global_types.get(name)
    }
    fn resolve_tag(&mut self, tag: &str, define: bool) -> String {
        if tag.is_empty() {
            self.tag_serial += 1;
            return format!("__anon_struct_{}", self.tag_serial);
        }
        for scope in self.tag_scope.iter().rev() {
            if let Some(u) = scope.get(tag) {
                return u.clone();
            }
        }
        if define {
            let uniq = if self.tag_scope.len() > 1 {
                self.tag_serial += 1;
                format!("{tag}__s{}", self.tag_serial)
            } else {
                tag.to_string()
            };
            if let Some(scope) = self.tag_scope.last_mut() {
                scope.insert(tag.to_string(), uniq.clone());
            }
            return uniq;
        }
        tag.to_string()
    }

    fn peek(&self) -> &Token {
        &self.toks[self.i.min(self.toks.len() - 1)]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.i < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn at(&self, k: &TokenKind) -> bool {
        std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(k)
            || self.peek_kind() == k
    }

    fn eat(&mut self, k: TokenKind) -> bool {
        if self.peek_kind() == &k
            || std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(&k)
                && !matches!(
                    k,
                    TokenKind::Ident(_)
                        | TokenKind::IntLit(_)
                        | TokenKind::StringLit(_)
                        | TokenKind::CharLit(_)
                )
        {
            // exact match for unit variants
            if self.peek_kind() == &k {
                self.bump();
                return true;
            }
        }
        if self.peek_kind() == &k {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, k: TokenKind) -> Result<Token, String> {
        let t = self.peek().clone();
        let ok = match (&t.kind, &k) {
            (TokenKind::Ident(_), TokenKind::Ident(_)) => true,
            _ => t.kind == k,
        };
        if !ok {
            return Err(format!(
                "expected {:?}, got {:?} at {}:{}",
                k, t.kind, t.line, t.col
            ));
        }
        Ok(self.bump())
    }

    fn is_typename(&self) -> bool {
        match self.peek_kind() {
            TokenKind::Int
            | TokenKind::Void
            | TokenKind::Char
            | TokenKind::Long
            | TokenKind::Short
            | TokenKind::Float
            | TokenKind::Double
            | TokenKind::Struct
            | TokenKind::Union
            | TokenKind::Enum
            | TokenKind::Unsigned
            | TokenKind::Signed
            | TokenKind::Static
            | TokenKind::Extern
            | TokenKind::Register
            | TokenKind::Inline
            | TokenKind::Restrict
            | TokenKind::Auto
            | TokenKind::Const
            | TokenKind::Volatile => true,
            TokenKind::Ident(s) => {
                // C ordinary-identifier scope: a parameter/local named the same
                // as a typedef shadows it in expressions. Redis does this heavily:
                //   void *raxFind(rax *rax, ...) { raxLowWalk(rax, ...); }
                // Without this, call-arg soft path treats `rax` as a type and
                // substitutes Int(0) → SEGV in raxLowWalk (null tree pointer).
                if self.lookup_local_type(s).is_some() {
                    return false;
                }
                if let Some(next) = self.toks.get(self.i + 1) {
                    if matches!(
                        next.kind,
                        TokenKind::Arrow
                            | TokenKind::Dot
                            | TokenKind::Assign
                            | TokenKind::PlusEq
                            | TokenKind::MinusEq
                            | TokenKind::StarEq
                            | TokenKind::SlashEq
                            | TokenKind::PlusPlus
                            | TokenKind::MinusMinus
                    ) {
                        return false;
                    }
                }
                Self::is_typeof_kw(&s)
                    || Self::is_gnu_attr_name(&s)
                    || s == "__builtin_va_list"
                    || s == "__gnuc_va_list"
                    || s.ends_with("_t")
                    || s.ends_with("_T")
                    || matches!(s.as_str(), "__u8" | "__u16" | "__u32" | "__u64" | "__s8" | "__s16" | "__s32" | "__s64" | "__le16" | "__le32" | "__le64" | "__be16" | "__be32" | "__be64" | "__sum16" | "__wsum" | "bool" | "_Bool" | "HIST_ENTRY" | "HIST_STATE" | "HISTORY_STATE")
                    || self.typedefs.iter().any(|t| t == s)
            }
            TokenKind::Section(_) | TokenKind::Packed | TokenKind::Weak => true,
            _ => false,
        }
    }

    fn is_gnu_attr_name(s: &str) -> bool {
        // Do NOT treat bare `attribute` as a GNU attr name: Redis and other
        // C code use it as a struct field / identifier (callReplyAttribute
        // bodies use `rep->attribute`). Lexer already keeps bare `attribute`
        // without `(...`). Only `__attribute` / `__attribute__` spellings.
        s.starts_with("__attribute")
            || s == "__section__"
            || s == "__section"
            || s == "__asm"
            || s == "__asm__"
            || s == "asm"
            || s == "__signed_wrap"
            || s == "noinstr"
            || s == "__noinstr_section"
            || s == "__no_caller_saved_registers"
            || s == "__no_caller_saved_registers__"
            || s == "__no_sanitize_coverage"
            || s == "__no_sanitize_address"
            || s == "__no_sanitize_undefined"
            || s == "__no_kasan_or_inline"
            || s == "__no_profile"
            || s == "__no_stack_protector"
            || s == "__noclone"
            || s == "__always_inline"
            || s == "__gnu_inline"
            || s == "__attribute_const__"
            || s == "__attribute_pure__"
            || s == "__maybe_unused"
            || s == "__used"
            || s == "__pure"
            || s == "__cold"
            || s == "__hot"
            || s == "__noreturn"
            || s == "__malloc"
            || s == "__read_mostly"
            || s == "__ro_after_init"
            || s == "__init"
            || s == "__exit"
            || s == "__ref"
            || s == "__head"
            || s == "__must_check"
            || s == "__packed"
            || s == "__aligned"
            || s == "__visible"
            || s.starts_with("__no_sanitize")
            || s.starts_with("__no_caller")
            || s.starts_with("__no_kasan")
            || s.starts_with("__no_profile")
            || s.starts_with("__no_stack")
            || s.starts_with("__read_mostly")
            || s.starts_with("__ro_after_init")
            // Kernel attr spellings only — do NOT match bare `*ATTRIBUTE*`
            // (postgres `RELOPT_KIND_ATTRIBUTE` / `attribute_reloptions`).
            || s.starts_with("__ATTRIBUTE")
            || s.starts_with("___ATTRIBUTE")
            || s == "ATTRIBUTE_UNUSED"
            || s == "ATTRIBUTE_USED"
            || s == "ATTRIBUTE_CONST"
            || s == "ATTRIBUTE_PURE"
            || s == "ATTRIBUTES"
            || s.starts_with("PER_CPU_")
    }

    fn parse_field_name(&mut self) -> Result<String, String> {
        let tok = self.peek().clone();
        let field = match tok.kind {
            TokenKind::Ident(s) => {
                self.bump();
                s
            }
            TokenKind::Int => { self.bump(); "int".into() }
            TokenKind::Void => { self.bump(); "void".into() }
            TokenKind::Char => { self.bump(); "char".into() }
            TokenKind::Long => { self.bump(); "long".into() }
            TokenKind::Short => { self.bump(); "short".into() }
            TokenKind::Float => { self.bump(); "float".into() }
            TokenKind::Double => { self.bump(); "double".into() }
            TokenKind::Struct => { self.bump(); "struct".into() }
            TokenKind::Union => { self.bump(); "union".into() }
            TokenKind::Typedef => { self.bump(); "typedef".into() }
            TokenKind::Enum => { self.bump(); "enum".into() }
            TokenKind::Unsigned => { self.bump(); "unsigned".into() }
            TokenKind::Signed => { self.bump(); "signed".into() }
            TokenKind::Static => { self.bump(); "static".into() }
            TokenKind::Extern => { self.bump(); "extern".into() }
            TokenKind::Register => { self.bump(); "register".into() }
            TokenKind::Inline => { self.bump(); "inline".into() }
            TokenKind::Restrict => { self.bump(); "restrict".into() }
            TokenKind::Auto => { self.bump(); "auto".into() }
            TokenKind::Const => { self.bump(); "const".into() }
            TokenKind::Volatile => { self.bump(); "volatile".into() }
            TokenKind::Return => { self.bump(); "return".into() }
            TokenKind::If => { self.bump(); "if".into() }
            TokenKind::Else => { self.bump(); "else".into() }
            TokenKind::While => { self.bump(); "while".into() }
            TokenKind::For => { self.bump(); "for".into() }
            TokenKind::Do => { self.bump(); "do".into() }
            TokenKind::Break => { self.bump(); "break".into() }
            TokenKind::Continue => { self.bump(); "continue".into() }
            TokenKind::Goto => { self.bump(); "goto".into() }
            TokenKind::Switch => { self.bump(); "switch".into() }
            TokenKind::Case => { self.bump(); "case".into() }
            TokenKind::Default => { self.bump(); "default".into() }
            TokenKind::Sizeof => { self.bump(); "sizeof".into() }
            TokenKind::Packed => { self.bump(); "packed".into() }
            TokenKind::Section(s) => { self.bump(); s }
            _ => {
                return Err(format!(
                    "expected field name identifier after '.' or '->', got {:?} at {}:{}",
                    tok.kind, tok.line, tok.col
                ));
            }
        };
        Ok(field)
    }

    fn is_typeof_kw(s: &str) -> bool {
        s == "typeof" || s == "__typeof" || s == "__typeof__" || s == "__auto_type"
    }

    /// After seeing '(', decide if this is nested `(declarator)` vs function params.
    fn lparen_starts_nested_declarator(&self) -> bool {
        let j = self.i + 1;
        match self.toks.get(j).map(|t| &t.kind) {
            Some(TokenKind::Star | TokenKind::LParen | TokenKind::LBracket) => true,
            Some(TokenKind::RParen) => false,
            Some(TokenKind::Ellipsis) => false,
            Some(
                TokenKind::Int
                | TokenKind::Void
                | TokenKind::Char
                | TokenKind::Long
                | TokenKind::Short
                | TokenKind::Float
                | TokenKind::Double
                | TokenKind::Struct
                | TokenKind::Union
                | TokenKind::Enum
                | TokenKind::Unsigned
                | TokenKind::Signed
                | TokenKind::Const
                | TokenKind::Volatile
                | TokenKind::Restrict
                | TokenKind::Register
                | TokenKind::Static,
            ) => false,
            Some(TokenKind::Ident(s)) => {
                if matches!(self.toks.get(j + 1).map(|t| &t.kind), Some(TokenKind::RParen))
                    && matches!(self.toks.get(j + 2).map(|t| &t.kind), Some(TokenKind::LParen | TokenKind::LBracket))
                {
                    true
                } else {
                    !self.typedefs.iter().any(|t| t == s)
                }
            }
            _ => false,
        }
    }

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut items = Vec::new();
        while !self.at(&TokenKind::Eof) {
            if self.eat(TokenKind::Semicolon) {
                continue;
            }
            // storage class at file scope (may repeat / interleave)
            let mut file_static = false;
            let mut file_extern = false;
            loop {
                if self.eat(TokenKind::Static) {
                    file_static = true;
                    continue;
                }
                if self.eat(TokenKind::Extern) {
                    file_extern = true;
                    continue;
                }
                if self.eat(TokenKind::Register)
                    || self.eat(TokenKind::Inline)
                    || self.eat(TokenKind::Restrict)
                    || self.eat(TokenKind::Auto)
                    || self.eat(TokenKind::Const)
                    || self.eat(TokenKind::Volatile)
                {
                    continue;
                }
                break;
            }
            if self.at(&TokenKind::Typedef) {
                items.push(self.parse_typedef()?);
                continue;
            }
            if self.peek_kind() == &TokenKind::Do
                || (matches!(self.peek_kind(), TokenKind::Int | TokenKind::Void | TokenKind::Char | TokenKind::Long | TokenKind::Short)
                    && self.toks.get(self.i + 1).map(|t| &t.kind) == Some(&TokenKind::Do))
            {
                if matches!(self.peek_kind(), TokenKind::Int | TokenKind::Void | TokenKind::Char | TokenKind::Long | TokenKind::Short) {
                    self.bump();
                }
                self.expect(TokenKind::Do)?;
                if self.at(&TokenKind::LBrace) {
                    let _ = self.skip_balanced_braces();
                }
                if self.eat(TokenKind::While) {
                    if self.at(&TokenKind::LParen) {
                        let _ = self.skip_balanced_parens();
                    }
                }
                let _ = self.eat(TokenKind::Semicolon);
                continue;
            }
            if self.eat(TokenKind::While) {
                if self.at(&TokenKind::LParen) {
                    let _ = self.skip_balanced_parens();
                }
                let _ = self.eat(TokenKind::Semicolon);
                continue;
            }
            if self.eat(TokenKind::If) {
                if self.at(&TokenKind::LParen) {
                    let _ = self.skip_balanced_parens();
                }
                if self.at(&TokenKind::LBrace) {
                    let _ = self.skip_balanced_braces();
                } else {
                    while !self.at(&TokenKind::Semicolon) && !self.at(&TokenKind::Else) && !self.at(&TokenKind::Eof) {
                        self.bump();
                    }
                    let _ = self.eat(TokenKind::Semicolon);
                }
                if self.eat(TokenKind::Else) {
                    if self.at(&TokenKind::LBrace) {
                        let _ = self.skip_balanced_braces();
                    } else {
                        while !self.at(&TokenKind::Semicolon) && !self.at(&TokenKind::Eof) {
                            self.bump();
                        }
                        let _ = self.eat(TokenKind::Semicolon);
                    }
                }
                continue;
            }
            if self.eat(TokenKind::For) || self.eat(TokenKind::Switch) {
                if self.at(&TokenKind::LParen) {
                    let _ = self.skip_balanced_parens();
                }
                if self.at(&TokenKind::LBrace) {
                    let _ = self.skip_balanced_braces();
                } else {
                    while !self.at(&TokenKind::Semicolon) && !self.at(&TokenKind::Eof) {
                        self.bump();
                    }
                    let _ = self.eat(TokenKind::Semicolon);
                }
                continue;
            }
            if self.eat(TokenKind::Return) || self.eat(TokenKind::Goto) {
                while !self.at(&TokenKind::Semicolon) && !self.at(&TokenKind::Eof) {
                    self.bump();
                }
                let _ = self.eat(TokenKind::Semicolon);
                continue;
            }
            // C11 `_Static_assert(cond, "msg");` / C23 `static_assert(...)` — skip.
            if matches!(
                self.peek_kind(),
                TokenKind::Ident(s) if s == "_Static_assert"
                    || s == "static_assert"
                    || s.starts_with("ALLOW_")
                    || s.starts_with("EXPORT_")
                    || s.starts_with("MODULE_")
                    || s.starts_with("TRACE_")
                    || s.starts_with("SYSCALL_")
                    || s.starts_with("KBUILD_")
            ) {
                self.bump();
                if self.at(&TokenKind::LParen) {
                    let _ = self.skip_balanced_parens();
                }
                let _ = self.eat(TokenKind::Semicolon);
                continue;
            }
            if self.at(&TokenKind::Enum) && self.is_enum_tag_decl() {
                items.extend(self.parse_enum_item(file_extern)?);
                continue;
            }
            // struct/union definition or forward decl at file scope: struct S { ... }; / struct S;
            if matches!(self.peek_kind(), TokenKind::Struct | TokenKind::Union)
                && self.is_struct_tag_decl()
            {
                items.push(self.parse_struct_or_union_item()?);
                continue;
            }
            // File-scope asm (kernel export symbols, sections, etc.)
            if matches!(
                self.peek_kind(),
                TokenKind::Ident(s) if s == "asm" || s == "__asm" || s == "__asm__"
            ) {
                let _ = self.parse_asm_stmt()?;
                continue;
            }
            // Soft recovery: one bad top-level decl/func must not abort the whole
            // kernel TU (cleanup.h CLASS expansions + huge headers). Skip to a
            // plausible next top-level boundary and keep going.
            let save_i = self.i;
            match self.parse_decl_or_func(file_static, file_extern) {
                Ok(more) => items.extend(more),
                Err(e) => {
                    // Surface soft-skip reasons for large-TU debugging (ACC_PARSE_TRACE=1).
                    if std::env::var_os("ACC_PARSE_TRACE").is_some() {
                        let t = self.peek();
                        eprintln!(
                            "ACC_PARSE_TRACE: soft-skip at {}:{} tok={:?}: {e}",
                            t.line, t.col, t.kind
                        );
                    }
                    self.i = save_i;
                    if !self.skip_to_next_toplevel() {
                        break;
                    }
                }
            }
        }
        // Flush enum constants discovered inside type specs (e.g. struct { enum { X } x; })
        for g in self.pending_enum_globals.drain(..) {
            items.insert(0, Item::Global(g));
        }
        let mut type_layouts = Vec::new();
        for (name, fields) in &self.struct_fields {
            let is_union = self.unions.iter().any(|u| u == name);
            let packed = self.packed_structs.contains(name);
            type_layouts.push((name.clone(), is_union, packed, fields.clone()));
        }
        Ok(Program {
            items,
            type_layouts,
        })
    }

    /// After a top-level parse error, advance to the next likely top-level item.
    /// Returns false if we hit EOF without progress.
    fn skip_to_next_toplevel(&mut self) -> bool {
        let start = self.i;
        // Prefer: skip one balanced {...} if we're at/inside a brace-heavy item,
        // else skip until `;` at brace-depth 0, then resume after it.
        let mut depth = 0i32;
        let mut saw = false;
        while !self.at(&TokenKind::Eof) {
            match self.peek_kind() {
                TokenKind::LBrace => {
                    depth += 1;
                    saw = true;
                    self.bump();
                }
                TokenKind::RBrace => {
                    self.bump();
                    if depth > 0 {
                        depth -= 1;
                        if depth == 0 && saw {
                            let _ = self.eat(TokenKind::Semicolon);
                            return self.i > start;
                        }
                    }
                }
                TokenKind::Semicolon if depth == 0 => {
                    self.bump();
                    return self.i > start;
                }
                _ => {
                    self.bump();
                }
            }
        }
        self.i > start
    }

    fn is_struct_tag_decl(&self) -> bool {
        // struct Ident { ... } ;  OR  struct Ident ;
        let mut j = self.i;
        if !matches!(
            self.toks.get(j).map(|t| &t.kind),
            Some(TokenKind::Struct | TokenKind::Union)
        ) {
            return false;
        }
        j += 1;
        if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Ident(_))) {
            j += 1;
        } else {
            return false;
        }
        if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::LBrace)) {
            j = self.skip_balanced_braces_offset(j);
            j = self.skip_gnu_attrs_offset(j);
            return matches!(
                self.toks.get(j).map(|t| &t.kind),
                Some(TokenKind::Semicolon)
            );
        }
        matches!(
            self.toks.get(j).map(|t| &t.kind),
            Some(TokenKind::Semicolon)
        )
    }

    fn is_enum_tag_decl(&self) -> bool {
        // enum E;  OR  enum E { ... }  OR  enum { ... }
        // NOT: enum E foo(...); / enum E x; / enum { ... } x;
        let mut j = self.i;
        if !matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Enum)) {
            return false;
        }
        j += 1;
        if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Ident(_))) {
            j += 1;
        }
        if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::LBrace)) {
            j = self.skip_balanced_braces_offset(j);
            j = self.skip_gnu_attrs_offset(j);
            return matches!(
                self.toks.get(j).map(|t| &t.kind),
                Some(TokenKind::Semicolon)
            );
        }
        matches!(
            self.toks.get(j).map(|t| &t.kind),
            Some(TokenKind::Semicolon)
        )
    }

    fn parse_struct_or_union_item(&mut self) -> Result<Item, String> {
        let is_union = self.at(&TokenKind::Union);
        self.bump();
        // `struct __attribute__((packed)) S { ... }`
        let mut packed = self.eat_packed_attrs();
        let name = if let TokenKind::Ident(s) = &self.peek_kind() {
            let s = s.clone();
            self.bump();
            s
        } else {
            return Err("anonymous struct at file scope needs a name".into());
        };
        packed = self.eat_packed_attrs() || packed;
        if self.eat(TokenKind::Semicolon) {
            // forward declaration
            if packed {
                self.packed_structs.insert(name.clone());
            }
            if is_union {
                self.unions.push(name.clone());
                Ok(Item::UnionDef {
                    name,
                    fields: Vec::new(),
                })
            } else {
                self.structs.push(name.clone());
                Ok(Item::StructDef {
                    name,
                    fields: Vec::new(),
                })
            }
        } else {
            self.expect(TokenKind::LBrace)?;
            let fields = self.parse_fields()?;
            self.expect(TokenKind::RBrace)?;
            // `struct S { ... } __attribute__((packed));`
            packed = self.eat_packed_attrs() || packed;
            if packed {
                self.packed_structs.insert(name.clone());
            }
            if is_union {
                self.unions.push(name.clone());
                self.struct_fields.insert(name.clone(), fields.clone());
            } else {
                self.structs.push(name.clone());
                self.struct_fields.insert(name.clone(), fields.clone());
            }
            // `struct S { ... };` pure def, or `struct S { ... } var;` with trailing decl.
            if self.eat(TokenKind::Semicolon) {
                if is_union {
                    Ok(Item::UnionDef { name, fields })
                } else {
                    Ok(Item::StructDef { name, fields })
                }
            } else {
                // Register layout then parse as a normal global of this type.
                // Push a synthetic StructDef via returning first, then globals —
                // caller only accepts one Item. Emit Global; layout already recorded.
                let base = if is_union {
                    Type::Union(name.clone())
                } else {
                    Type::Struct(name.clone())
                };
                let (vname, vty, _) = self.parse_declarator(base)?;
                let init = if self.eat(TokenKind::Assign) {
                    Some(self.parse_initializer()?)
                } else {
                    None
                };
                self.expect(TokenKind::Semicolon)?;
                // Layout is already in struct_fields; StructDef item is optional for
                // codegen as long as type_layouts is filled from struct_fields.
                self.insert_global_type(&vname, vty.clone());
                Ok(Item::Global(VarDecl {
                    name: vname,
                    ty: vty,
                    init,
                is_static: false,
                is_extern: false,
                is_weak: false,
                section: None,
            }))
            }
        }
    }

    /// If PP glued a typedef type to a field name (`__u8pkt_type`), peel them apart.
    fn peel_glued_type_name(&self, s: &str) -> Option<(Type, String)> {
        // Longest typedef prefix match so `__u16` wins over `__u1` if both exist.
        let mut best: Option<(usize, Type)> = None;
        for (name, ty) in &self.typedef_map {
            if name.len() >= 3 && s.starts_with(name.as_str()) && s.len() > name.len() {
                let rest = &s[name.len()..];
                // Rest must look like an identifier continuation.
                if rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && best.as_ref().map(|(n, _)| name.len() > *n).unwrap_or(true)
                {
                    best = Some((name.len(), ty.clone()));
                }
            }
        }
        // Also peel common fixed-width C types if not already typedef'd.
        for (prefix, ty) in [
            ("__u8", Type::Char),
            ("__s8", Type::SChar),
            ("__u16", Type::UShort),
            ("__s16", Type::Short),
            ("__u32", Type::UInt),
            ("__s32", Type::Int),
            ("__u64", Type::ULong),
            ("__s64", Type::Long),
            ("u8", Type::Char),
            ("u16", Type::UShort),
            ("u32", Type::UInt),
            ("u64", Type::ULong),
        ] {
            if s.starts_with(prefix) && s.len() > prefix.len() {
                let rest = &s[prefix.len()..];
                if rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && best.as_ref().map(|(n, _)| prefix.len() > *n).unwrap_or(true)
                {
                    best = Some((prefix.len(), ty));
                }
            }
        }
        best.map(|(n, ty)| (ty, s[n..].to_string()))
    }

    fn parse_fields(&mut self) -> Result<Vec<Field>, String> {
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            // Empty field: `struct { void *lock;; }` (double semicolon from macros).
            if self.eat(TokenKind::Semicolon) {
                continue;
            }
            // Glued type+name from PP macro expansion: `__u8pkt_type:3`
            if let TokenKind::Ident(s) = self.peek_kind().clone() {
                if !self.typedefs.iter().any(|t| t == &s) {
                    if let Some((base, name)) = self.peel_glued_type_name(&s) {
                        self.bump();
                        let bit_width = if self.eat(TokenKind::Colon) {
                            let w = match self.parse_assign()? {
                                Expr::Int(n) => n.max(0) as u32,
                                other => self.eval_enum_const(&other).unwrap_or(1).max(0) as u32,
                            };
                            Some(w)
                        } else {
                            None
                        };
                        // Optional array: name[N]
                        let mut ty = base;
                        while self.eat(TokenKind::LBracket) {
                            let nsz = if let TokenKind::IntLit(v) = self.peek_kind().clone() {
                                self.bump();
                                v
                            } else {
                                0
                            };
                            self.expect(TokenKind::RBracket)?;
                            ty = Type::Array(Box::new(ty), nsz);
                        }
                        fields.push(Field {
                            name,
                            ty,
                            bit_width,
                        });
                        // More glued fields may follow with commas (rare) or just `;`
                        let _ = self.eat(TokenKind::Semicolon);
                        continue;
                    }
                }
            }
            let base = self.parse_type_specifier()?;
            // Anonymous struct/union field: `union { int c; int d; };`
            if self.eat(TokenKind::Semicolon) {
                fields.push(Field {
                    name: String::new(),
                    ty: base,
                    bit_width: None,
                });
                continue;
            }
            // Unnamed bit-field: `unsigned : 3;` / `int : 0;`
            if self.eat(TokenKind::Colon) {
                // Use assign-expr (not full expr) so `,` separates bitfields, not comma-op.
                let w = match self.parse_assign()? {
                    Expr::Int(n) => n.max(0) as u32,
                    _ => 0,
                };
                fields.push(Field {
                    name: String::new(),
                    ty: base,
                    bit_width: Some(w),
                });
                self.expect(TokenKind::Semicolon)?;
                continue;
            }
            loop {
                // Comma-separated bitfields may insert unnamed `: N` mid-list:
                // `u64 a:16, b:2, :45;`
                if self.eat(TokenKind::Colon) {
                    let w = match self.parse_assign()? {
                        Expr::Int(n) => n.max(0) as u32,
                        _ => 0,
                    };
                    fields.push(Field {
                        name: String::new(),
                        ty: base.clone(),
                        bit_width: Some(w),
                    });
                    if self.eat(TokenKind::Comma) {
                        continue;
                    }
                    break;
                }
                let (name, ty, _) = self.parse_declarator(base.clone())?;
                // Bit-field: `unsigned flags : 1;`
                let bit_width = if self.eat(TokenKind::Colon) {
                    let w = match self.parse_assign()? {
                        Expr::Int(n) => n.max(0) as u32,
                        other => self.eval_enum_const(&other).unwrap_or(1).max(0) as u32,
                    };
                    Some(w)
                } else {
                    None
                };
                fields.push(Field {
                    name,
                    ty,
                    bit_width,
                });
                self.skip_trailing_gnu_attrs();
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                break;
            }
            while !self.at(&TokenKind::Semicolon) && !self.at(&TokenKind::Comma) && !self.at(&TokenKind::Eof) {
                self.skip_trailing_gnu_attrs();
                if self.at(&TokenKind::Semicolon) || self.at(&TokenKind::Comma) || self.at(&TokenKind::Eof) {
                    break;
                }
                self.bump();
                if self.at(&TokenKind::LParen) {
                    let _ = self.skip_balanced_parens();
                }
            }
            self.eat(TokenKind::Semicolon);
        }
        Ok(fields)
    }

    fn parse_typedef(&mut self) -> Result<Item, String> {
        self.expect(TokenKind::Typedef)?;
        let base = self.parse_type_specifier()?;
        // Linux headers: `typedef _Bool _Bool;` is a no-op. Lexer maps `_Bool`
        // → TokenKind::Char, so this looks like `typedef char char;` and the second
        // Char is not a valid declarator name. Accept a type-keyword "name" as a
        // no-op typedef of `_Bool` (keeps subsequent parse alive for kernel TUs).
        let (name, ty, _) = if matches!(
            self.peek_kind(),
            TokenKind::Int
                | TokenKind::Void
                | TokenKind::Char
                | TokenKind::Long
                | TokenKind::Short
                | TokenKind::Float
                | TokenKind::Double
                | TokenKind::Unsigned
                | TokenKind::Signed
        ) {
            self.bump();
            ("_Bool".to_string(), base, None)
        } else {
            self.parse_declarator(base)?
        };
        self.expect(TokenKind::Semicolon)?;
        // Normalize anon struct/union typedefs to named layouts under the typedef name
        let ty = match ty {
            Type::AnonStruct(fs) => {
                self.struct_fields.insert(name.clone(), fs.clone());
                self.structs.push(name.clone());
                Type::Struct(name.clone())
            }
            Type::AnonUnion(fs) => {
                self.struct_fields.insert(name.clone(), fs.clone());
                self.unions.push(name.clone());
                Type::Union(name.clone())
            }
            other => other,
        };
        self.typedefs.push(name.clone());
        self.typedef_map.insert(name.clone(), ty.clone());
        Ok(Item::Typedef { name, ty })
    }

    fn parse_type_specifier(&mut self) -> Result<Type, String> {
        // skip storage / signedness / long prefixes (may interleave with long/int)
        let mut saw_unsigned = false;
        let mut saw_signed = false;
        let mut saw_long = false;
        loop {
            // Residual GNU attributes that escaped the lexer (nested explosion).
            self.skip_trailing_gnu_attrs();
            if self.eat(TokenKind::Static)
                || self.eat(TokenKind::Extern)
                || self.eat(TokenKind::Register)
                || self.eat(TokenKind::Inline)
                || self.eat(TokenKind::Restrict)
                || self.eat(TokenKind::Auto)
                || self.eat(TokenKind::Const)
                || self.eat(TokenKind::Volatile)
            {
                continue;
            }
            if let TokenKind::Ident(s) = self.peek_kind().clone() {
                let next_is_builtin_type = matches!(
                    self.toks.get(self.i + 1).map(|t| &t.kind),
                    Some(
                        TokenKind::Int
                            | TokenKind::Void
                            | TokenKind::Char
                            | TokenKind::Long
                            | TokenKind::Short
                            | TokenKind::Float
                            | TokenKind::Double
                            | TokenKind::Struct
                            | TokenKind::Union
                            | TokenKind::Enum
                            | TokenKind::Unsigned
                            | TokenKind::Signed
                    )
                );
                if Self::is_gnu_attr_name(&s)
                    || (next_is_builtin_type && !self.typedefs.iter().any(|t| t == &s))
                {
                    self.bump();
                    if self.at(&TokenKind::LParen) {
                        let _ = self.skip_balanced_parens();
                    }
                    continue;
                }
            }
            if self.eat(TokenKind::Signed) {
                saw_signed = true;
                continue;
            }
            if self.eat(TokenKind::Unsigned) {
                saw_unsigned = true;
                continue;
            }
            if self.eat(TokenKind::Long) {
                saw_long = true;
                self.skip_trailing_gnu_attrs();
                let _ = self.eat(TokenKind::Long);
                self.skip_trailing_gnu_attrs();
                let _ = self.eat(TokenKind::Int);
                self.skip_trailing_gnu_attrs();
                continue;
            }
            break;
        }
        if saw_long {
            if self.eat(TokenKind::Double) {
                return Ok(Type::Double);
            }
            return Ok(if saw_unsigned {
                Type::ULong
            } else {
                Type::Long
            });
        }
        let ty = match self.peek_kind().clone() {
            TokenKind::Void => {
                self.bump();
                Type::Void
            }
            TokenKind::Char => {
                self.bump();
                // `signed char` is a distinct type (1-byte signed). Plain/`unsigned char`
                // stay as Char. Lemon tables use `signed char` with negative RHS counts.
                if saw_signed && !saw_unsigned {
                    Type::SChar
                } else {
                    Type::Char
                }
            }
            TokenKind::Int => {
                self.bump();
                if saw_unsigned {
                    Type::UInt
                } else {
                    Type::Int
                }
            }
            TokenKind::Long => {
                self.bump();
                self.skip_trailing_gnu_attrs();
                // long long / long int / long double
                if self.eat(TokenKind::Double) {
                    Type::Double
                } else {
                    self.skip_trailing_gnu_attrs();
                    let _ = self.eat(TokenKind::Long);
                    self.skip_trailing_gnu_attrs();
                    let _ = self.eat(TokenKind::Int);
                    self.skip_trailing_gnu_attrs();
                    if saw_unsigned {
                        Type::ULong
                    } else {
                        Type::Long
                    }
                }
            }
            TokenKind::Short => {
                self.bump();
                let _ = self.eat(TokenKind::Int);
                if saw_unsigned {
                    Type::UShort
                } else {
                    Type::Short
                }
            }
            TokenKind::Float => {
                self.bump();
                Type::Float
            }
            TokenKind::Double => {
                self.bump();
                Type::Double
            }
            // bare `unsigned;` / `unsigned x;` → unsigned int
            // bare `signed;` → signed int
            _ if saw_unsigned => Type::UInt,
            _ if saw_signed => Type::Int,
            TokenKind::Struct => self.parse_struct_type(false)?,
            TokenKind::Union => self.parse_struct_type(true)?,
            TokenKind::Enum => {
                // enum E or enum { ... } as type — register enumerators as globals
                self.bump();
                if let TokenKind::Ident(_) = self.peek_kind().clone() {
                    self.bump();
                }
                if self.eat(TokenKind::LBrace) {
                    let mut next_val = 0i64;
                    while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                        if self.eat(TokenKind::Comma) {
                            continue;
                        }
                        // PP may expand enumerator names that were also #define'd:
                        // `enum sock_type { SOCK_STREAM = 1 }` → `{ 1 = 1 }`.
                        if matches!(
                            self.peek_kind(),
                            TokenKind::IntLit(_) | TokenKind::CharLit(_)
                        ) {
                            if let TokenKind::IntLit(n) = self.peek_kind().clone() {
                                self.bump();
                                next_val = n;
                            } else if let TokenKind::CharLit(n) = self.peek_kind().clone() {
                                self.bump();
                                next_val = n;
                            }
                            if self.eat(TokenKind::Assign) {
                                let e = self.parse_assign()?;
                                if let Some(v) = self.eval_enum_const(&e) {
                                    next_val = v;
                                }
                            }
                            next_val += 1;
                            if self.eat(TokenKind::Comma) {
                                continue;
                            }
                            if matches!(
                                self.peek_kind(),
                                TokenKind::Ident(_)
                                    | TokenKind::IntLit(_)
                                    | TokenKind::CharLit(_)
                            ) {
                                continue;
                            }
                            break;
                        }
                        if let TokenKind::Ident(id) = self.peek_kind().clone() {
                            self.bump();
                            // Residue function-like macro not expanded by PP, e.g.
                            // `__BPF_ENUM_FN(set_hash, 48,)` — skip `(...)` and keep name.
                            if self.at(&TokenKind::LParen) {
                                let _ = self.skip_balanced_parens();
                            }
                            if self.eat(TokenKind::Assign) {
                                let e = self.parse_assign()?;
                                if let Some(v) = self.eval_enum_const(&e) {
                                    next_val = v;
                                }
                            }
                            self.enum_values.insert(id.clone(), next_val);
                            // Local (static) global for expression load; not .globl
                            // (kernel headers re-define the same enums in every TU).
                            self.pending_enum_globals.push(VarDecl {
                                name: id,
                                ty: Type::Int,
                                init: Some(Expr::Int(next_val)),
                                is_static: true,
                                is_extern: false,
                                is_weak: false,
                section: None,
                            });
                            next_val += 1;
                        } else {
                            break;
                        }
                        if self.eat(TokenKind::Comma) {
                            continue;
                        }
                        // Unexpanded macro residues may omit commas between
                        // `__BPF_ENUM_FN(a,1,) __BPF_ENUM_FN(b,2,)` entries.
                        if matches!(
                            self.peek_kind(),
                            TokenKind::Ident(_) | TokenKind::IntLit(_) | TokenKind::CharLit(_)
                        ) {
                            continue;
                        }
                        break;
                    }
                    self.expect(TokenKind::RBrace)?;
                }
                Type::Int
            }
            TokenKind::Ident(s) if Self::is_typeof_kw(&s) => {
                // GNU/C23 typeof(expr) / typeof(type) / __auto_type
                self.bump();
                if s == "__auto_type" {
                    Type::ULong
                } else if self.eat(TokenKind::LParen) {
                    // Prefer expression form for `typeof(*(p))`, `typeof(x->f[i])`, casts.
                    // Only take type-name when the next token is a plain type keyword/typedef
                    // (not `(` which starts cast/paren expr).
                    let t = if self.is_typename()
                        && !matches!(
                            self.peek_kind(),
                            TokenKind::LParen | TokenKind::Star | TokenKind::AndAnd | TokenKind::Amp
                        ) {
                        self.parse_type_name()?
                    } else {
                        let _e = self.parse_assign()?;
                        Type::ULong
                    };
                    self.expect(TokenKind::RParen)?;
                    t
                } else {
                    Type::ULong
                }
            }
            // Compiler builtins used as types in freestanding/kernel headers.
            TokenKind::Ident(s) if s == "__builtin_va_list" || s == "__gnuc_va_list" => {
                self.bump();
                Type::Ptr(Box::new(Type::Void))
            }
            TokenKind::Ident(s) if self.typedefs.iter().any(|t| t == &s) => {
                self.bump();
                self.typedef_map.get(&s).cloned().unwrap_or(Type::Int)
            }
            // Soft: unknown identifier as type name (incomplete headers / OS-specific
            // types like malloc_zone_t). Register as opaque int typedef and continue.
            TokenKind::Ident(s) => {
                self.bump();
                let ty = if s.ends_with("_t") || s.ends_with("_T") || matches!(s.as_str(), "__u8" | "__u16" | "__u32" | "__u64" | "__s8" | "__s16" | "__s32" | "__s64" | "__le16" | "__le32" | "__le64" | "__be16" | "__be32" | "__be64" | "__sum16" | "__wsum" | "bool" | "_Bool" | "HIST_ENTRY" | "HIST_STATE" | "HISTORY_STATE") {
                    match s.as_str() {
                        "bool" | "_Bool" | "__u8" | "u8" | "uint8_t" | "bool_t" => Type::UChar,
                        "__s8" | "s8" | "int8_t" | "flex_int8_t" | "yytype_int8" => Type::SChar,
                        "__u16" | "u16" | "uint16_t" => Type::UShort,
                        "__s16" | "s16" | "int16_t" | "flex_int16_t" | "yytype_int16" => Type::Short,
                        "__u64" | "u64" | "uint64_t" | "size_t" | "uintptr_t" | "uintmax_t" => Type::ULong,
                        "__s64" | "s64" | "int64_t" | "intptr_t" | "off_t" | "ssize_t" | "ptrdiff_t" | "intmax_t" => Type::Long,
                        _ if s.starts_with("uint") || s.ends_with("u8") => Type::UChar,
                        _ if s.starts_with("int8") || s.ends_with("s8") => Type::SChar,
                        _ if s.ends_with("8_t") || s.ends_with("8") => Type::UChar,
                        _ if s.ends_with("16_t") || s.ends_with("16") => Type::UShort,
                        _ if s.ends_with("64_t") || s.ends_with("64") || s.ends_with("ptr_t") || s.ends_with("size_t") => Type::ULong,
                        _ => Type::Int,
                    }
                } else {
                    Type::Int
                };
                if !self.typedefs.iter().any(|t| t == &s) {
                    self.typedefs.push(s.clone());
                    self.typedef_map.insert(s, ty.clone());
                }
                ty
            }
            _ => {
                return Err(format!(
                    "expected type at {}:{}",
                    self.peek().line,
                    self.peek().col
                ));
            }
        };
        // Trailing type qualifiers: `char const`, `char __user`, etc.
        self.skip_kernel_type_quals();
        Ok(ty)
    }

    fn parse_enum_item(&mut self, is_extern: bool) -> Result<Vec<Item>, String> {
        self.expect(TokenKind::Enum)?;
        let _name = if let TokenKind::Ident(s) = self.peek_kind().clone() {
            self.bump();
            Some(s)
        } else {
            None
        };
        // forward: enum efoo;
        if self.eat(TokenKind::Semicolon) {
            return Ok(Vec::new());
        }
        self.expect(TokenKind::LBrace)?;
        let mut items = Vec::new();
        let mut next_val = 0i64;
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            // Skip empty slots / extra commas.
            if self.eat(TokenKind::Comma) {
                continue;
            }
            // PP may expand enumerator names that were also #define'd:
            // `enum sock_type { SOCK_STREAM = 1 }` → `{ 1 = 1 }` after PP.
            // Accept IntLit / CharLit as nameless enumerators and keep counting.
            if matches!(
                self.peek_kind(),
                TokenKind::IntLit(_) | TokenKind::CharLit(_)
            ) {
                if let TokenKind::IntLit(n) = self.peek_kind().clone() {
                    self.bump();
                    next_val = n;
                } else if let TokenKind::CharLit(n) = self.peek_kind().clone() {
                    self.bump();
                    next_val = n;
                }
                if self.eat(TokenKind::Assign) {
                    let e = self.parse_assign()?;
                    if let Some(v) = self.eval_enum_const(&e) {
                        next_val = v;
                    }
                }
                next_val += 1;
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                if matches!(
                    self.peek_kind(),
                    TokenKind::Ident(_) | TokenKind::IntLit(_) | TokenKind::CharLit(_)
                ) {
                    continue;
                }
                break;
            }
            let id = if let TokenKind::Ident(s) = self.peek_kind().clone() {
                self.bump();
                s
            } else {
                return Err(format!(
                    "enum enumerator name expected at {}:{}",
                    self.peek().line,
                    self.peek().col
                ));
            };
            // Unexpanded function-like macro residue: `NAME(args)`
            if self.at(&TokenKind::LParen) {
                let _ = self.skip_balanced_parens();
            }
            if self.eat(TokenKind::Assign) {
                let e = self.parse_assign()?;
                if let Some(v) = self.eval_enum_const(&e) {
                    next_val = v;
                }
            }
            self.enum_values.insert(id.clone(), next_val);
            // Local (static) global — not .globl (avoids vdso multi-def of enum names).
            items.push(Item::Global(VarDecl {
                name: id,
                ty: Type::Int,
                init: Some(Expr::Int(next_val)),
                is_static: true,
                is_extern: false,
                is_weak: false,
                section: None,
            }));
            next_val += 1;
            if self.eat(TokenKind::Comma) {
                continue;
            }
            // Unexpanded macro residues may omit commas between entries.
            if matches!(
                self.peek_kind(),
                TokenKind::Ident(_) | TokenKind::IntLit(_) | TokenKind::CharLit(_)
            ) {
                continue;
            }
            break;
        }
        self.expect(TokenKind::RBrace)?;
        self.skip_trailing_gnu_attrs();
        // `enum { X, Y } name = Y;` / `enum E { ... } var;`
        // Trailing declarators with optional initializers (file-scope globals).
        loop {
            self.skip_kernel_type_quals();
            self.skip_trailing_gnu_attrs();
            while self.eat(TokenKind::Star) {
                self.skip_kernel_type_quals();
                self.skip_trailing_gnu_attrs();
            }
            if let TokenKind::Ident(id) = self.peek_kind().clone() {
                self.bump();
                // skip residual () if function-like residue
                if self.at(&TokenKind::LParen) {
                    let _ = self.skip_balanced_parens();
                }
                let init = if self.eat(TokenKind::Assign) {
                    Some(self.parse_initializer()?)
                } else {
                    None
                };
                // `extern enum E { ... } var;` → declaration only, no BSS.
                if is_extern && init.is_none() {
                    // skip define
                } else {
                    items.push(Item::Global(VarDecl {
                        name: id,
                        ty: Type::Int,
                        init,
                        is_static: false,
                        is_extern,
                        is_weak: false,
                section: None,
                    }));
                }
                if self.eat(TokenKind::Comma) {
                    continue;
                }
            }
            break;
        }
        self.eat(TokenKind::Semicolon);
        Ok(items)
    }

    fn parse_struct_type(&mut self, is_union: bool) -> Result<Type, String> {
        self.bump(); // struct/union
        // `struct __attribute__((packed)) name { ... }`
        let mut packed = self.eat_packed_attrs();
        let mut name: Option<String> = None;
        if let TokenKind::Ident(s) = self.peek_kind().clone() {
            if s == "__attribute__" || s == "__attribute" || s == "__extension__" || s == "__extension" {
                self.bump();
                if self.at(&TokenKind::LParen) {
                    let _ = self.skip_balanced_parens();
                }
                if let TokenKind::Ident(s2) = self.peek_kind().clone() {
                    name = Some(s2);
                    self.bump();
                }
            } else {
                name = Some(s);
                self.bump();
            }
        }
        packed = self.eat_packed_attrs() || packed;
        self.skip_trailing_gnu_attrs();
        if self.eat(TokenKind::LBrace) {
            // Register tag before fields so recursive `struct S *p` resolves.
            let uniq_opt = name.as_ref().map(|n| self.resolve_tag(n, true));
            if let Some(ref uniq) = uniq_opt {
                if is_union {
                    self.unions.push(uniq.clone());
                } else {
                    self.structs.push(uniq.clone());
                }
            }
            let fields = self.parse_fields()?;
            self.expect(TokenKind::RBrace)?;
            packed = self.eat_packed_attrs() || packed;
            if let Some(uniq) = uniq_opt {
                self.struct_fields.insert(uniq.clone(), fields.clone());
                if packed {
                    self.packed_structs.insert(uniq.clone());
                }
                if is_union {
                    Ok(Type::Union(uniq))
                } else {
                    Ok(Type::Struct(uniq))
                }
            } else if is_union {
                Ok(Type::AnonUnion(fields))
            } else {
                Ok(Type::AnonStruct(fields))
            }
        } else if let Some(n) = name {
            let uniq = self.resolve_tag(&n, false);
            if packed {
                self.packed_structs.insert(uniq.clone());
            }
            if is_union {
                Ok(Type::Union(uniq))
            } else {
                Ok(Type::Struct(uniq))
            }
        } else {
            let uniq = self.resolve_tag("", false);
            if packed {
                self.packed_structs.insert(uniq.clone());
            }
            if is_union {
                Ok(Type::Union(uniq))
            } else {
                Ok(Type::Struct(uniq))
            }
        }
    }

    /// Parse parameter list including leading '(' already consumed.
    /// Returns (params, is_variadic).
    fn parse_param_list_body(&mut self) -> Result<(Vec<(String, Type)>, bool), String> {
        let mut params = Vec::new();
        let mut variadic = false;
        if self.at(&TokenKind::RParen) {
            return Ok((params, false));
        }
        let bare_void = self.at(&TokenKind::Void)
            && matches!(
                self.toks.get(self.i + 1).map(|t| &t.kind),
                Some(TokenKind::RParen)
            );
        if bare_void {
            self.bump();
            return Ok((params, false));
        }
        loop {
            if self.eat(TokenKind::Ellipsis) {
                variadic = true;
                break;
            }
            let pb = self.parse_type_specifier()?;
            let (pn, mut pt, fp) = self.parse_declarator(pb)?;
            // D-redis / Phase B.2: C decays function-typed parameters to
            // pointers. Redis dict.c uses old-style
            // `void(callback)(dict*)` (== `void (*callback)(dict*)`).
            // Without decay, the local is typed Void and codegen emits
            // `bl callback` (undef) instead of load+blr.
            if fp.is_some() {
                pt = Type::Ptr(Box::new(pt));
            }
            params.push((pn, pt));
            if self.eat(TokenKind::Comma) {
                if self.eat(TokenKind::Ellipsis) {
                    variadic = true;
                    break;
                }
                continue;
            }
            break;
        }
        Ok((params, variadic))
    }

    /// Parse declarator: pointers, name / nested (*name), arrays, function suffix.
    /// Returns (name, type, outermost function params + variadic if this is a function).
    fn skip_kernel_type_quals(&mut self) {
        loop {
            match self.peek_kind().clone() {
                TokenKind::Const | TokenKind::Volatile | TokenKind::Restrict => {
                    self.bump();
                }
                TokenKind::Ident(s)
                    if matches!(
                        s.as_str(),
                        "__user"
                            | "__kernel"
                            | "__iomem"
                            | "__percpu"
                            | "__rcu"
                            | "__force"
                            | "__bitwise"
                            | "__bitwise__"
                            | "__private"
                            | "__safe"
                            | "__nocast"
                            | "__pmem"
                            | "restrict"
                            | "__restrict"
                            | "__restrict__"
                    ) =>
                {
                    self.bump();
                }
                _ => break,
            }
        }
    }

    fn parse_declarator(
        &mut self,
        base: Type,
    ) -> Result<(String, Type, Option<(Vec<(String, Type)>, bool)>), String> {
        let mut ty = base;
        self.skip_kernel_type_quals();
        self.skip_trailing_gnu_attrs();
        while self.eat(TokenKind::Star) {
            // pointer qualifiers: *const / *volatile / *restrict / *__user
            loop {
                match self.peek_kind().clone() {
                    TokenKind::Const | TokenKind::Volatile | TokenKind::Restrict => {
                        self.bump();
                    }
                    TokenKind::Ident(s)
                        if matches!(
                            s.as_str(),
                            "restrict"
                                | "__restrict"
                                | "__restrict__"
                                | "__user"
                                | "__kernel"
                                | "__iomem"
                                | "__percpu"
                                | "__rcu"
                                | "__force"
                                | "__bitwise"
                                | "__bitwise__"
                                | "__private"
                                | "__safe"
                                | "__nocast"
                                | "__pmem"
                        ) || Self::is_gnu_attr_name(&s) =>
                    {
                        self.bump();
                        if self.at(&TokenKind::LParen) {
                            let _ = self.skip_balanced_parens();
                        }
                    }
                    _ => break,
                }
            }
            self.skip_kernel_type_quals();
            self.skip_trailing_gnu_attrs();
            ty = Type::Ptr(Box::new(ty));
        }
        self.skip_kernel_type_quals();
        self.skip_trailing_gnu_attrs();
        // '(' starts a nested declarator (*name)/(*name()) only when the next
        // token looks like a declarator, not a type (function-parameter list).
        // So `int (int x)` / `int ()` are abstract function types, while
        // `int (*f)(void)` / `int (foo)` are nested declarators.
        // `nested_ptr_form`: true for `(*name)` / `(*name)(params)` function-pointer
        // variables. false for bare `(name)` including `T *(name)(params)` (function
        // returning T*). Distinguishing by post-hoc Type::Ptr is wrong: outer `*` for
        // the return type also yields Ptr (Redis/Lua `lua_State *(luaL_newstate)(void)`).
        let (name, mut ty, nested, nested_ptr_form, mut bubbled_fp) = if self.at(&TokenKind::LParen)
            && self.lparen_starts_nested_declarator()
        {
            self.bump(); // (
            self.skip_trailing_gnu_attrs();
            let nested_ptr_form = self.at(&TokenKind::Star);
            let (n, inner, inner_fp) = self.parse_declarator(ty)?;
            self.skip_trailing_gnu_attrs();
            self.expect(TokenKind::RParen)?;
            // Bubble function params from `(*name(params))` so definitions work:
            // `void (*f(T))(void) { ... }` is a function named f.
            (n, inner, true, nested_ptr_form, inner_fp)
        } else {
            self.skip_trailing_gnu_attrs();
            if self.eat(TokenKind::Do) {
                if self.at(&TokenKind::LBrace) {
                    let _ = self.skip_balanced_braces();
                }
                if self.at(&TokenKind::LParen) {
                    let _ = self.skip_balanced_parens();
                }
                self.skip_trailing_gnu_attrs();
            }
            if let TokenKind::Ident(s) = self.peek_kind().clone() {
                self.bump();
                let full_name = if s.starts_with("__UNIQUE_ID") && self.at(&TokenKind::LParen) {
                    self.bump(); // (
                    let inner_name = if let TokenKind::Ident(id) = self.peek_kind().clone() {
                        self.bump();
                        id
                    } else {
                        "name".to_string()
                    };
                    if self.at(&TokenKind::RParen) {
                        self.bump(); // )
                    }
                    format!("{}_{}", s, inner_name)
                } else {
                    s
                };
                (full_name, ty, false, false, None)
            } else if self.at(&TokenKind::LParen) && self.lparen_starts_nested_declarator() {
                self.bump();
                self.skip_trailing_gnu_attrs();
                let nested_ptr_form = self.at(&TokenKind::Star);
                let (n, inner, inner_fp) = self.parse_declarator(ty)?;
                self.skip_trailing_gnu_attrs();
                self.expect(TokenKind::RParen)?;
                (n, inner, true, nested_ptr_form, inner_fp)
            } else {
                (String::new(), ty, false, false, None)
            }
        };
        self.skip_trailing_gnu_attrs();
        // Arrays: collect dims left-to-right then wrap right-to-left so
        // `char a[2][4]` becomes Array(Array(Char,4), 2).
        // Nested `(*p)[4]` → Ptr(Array(Char,4)).
        let mut dims: Vec<i64> = Vec::new();
        while self.eat(TokenKind::LBracket) {
            loop {
                match self.peek_kind().clone() {
                    TokenKind::Const
                    | TokenKind::Volatile
                    | TokenKind::Static
                    | TokenKind::Restrict
                    | TokenKind::Register => {
                        self.bump();
                    }
                    TokenKind::Ident(s)
                        if matches!(
                            s.as_str(),
                            "restrict" | "__restrict" | "__restrict__"
                        ) || Self::is_gnu_attr_name(&s) =>
                    {
                        self.bump();
                        if self.at(&TokenKind::LParen) {
                            let _ = self.skip_balanced_parens();
                        }
                    }
                    _ => break,
                }
            }
            let n = if self.eat(TokenKind::Star) {
                0
            } else if self.at(&TokenKind::RBracket) {
                0
            } else {
                // Constant expression: 11, (11+1), sizeof, offsetof...
                let e = self.parse_expr()?;
                self.const_array_len(&e).unwrap_or(0)
            };
            self.expect(TokenKind::RBracket)?;
            self.skip_trailing_gnu_attrs();
            dims.push(n);
        }
        self.skip_trailing_gnu_attrs();
        for n in dims.into_iter().rev() {
            if nested {
                ty = Self::array_under_ptrs(ty, n);
            } else {
                ty = Type::Array(Box::new(ty), n);
            }
        }
        self.skip_trailing_gnu_attrs();
        // Function suffixes:
        // - bare `name(params)` → function prototype/definition (return params)
        // - `int (name)(params)` → same as bare (Lua: `void (luaL_register)(...)`)
        // - `T *(name)(params)` → function returning T* (Lua: `lua_State *(luaL_newstate)(void)`)
        // - `(*name)(params)` → pointer-to-function variable (no func params)
        // - `(*name(params))(params2)` → function returning function pointer
        //   (params bubbled from inner; params2 is return type sugar)
        self.skip_trailing_gnu_attrs();
        let mut func_params: Option<(Vec<(String, Type)>, bool)> = None;
        // Parenthesized bare name `(name)` is a function declarator even when the
        // return type is a pointer (`T *`). Only `(*name)` is a function-pointer var.
        let paren_bare_name = nested && !nested_ptr_form;
        if (!nested || paren_bare_name) && self.at(&TokenKind::LParen) {
            self.bump();
            let params = self.parse_param_list_body()?;
            self.expect(TokenKind::RParen)?;
            self.skip_trailing_gnu_attrs();
            func_params = Some(params);
            while self.at(&TokenKind::LParen) {
                self.bump();
                let _ = self.parse_param_list_body()?;
                self.expect(TokenKind::RParen)?;
                self.skip_trailing_gnu_attrs();
            }
        } else {
            // nested pointer / abstract: absorb trailing (params) as type sugar
            while self.at(&TokenKind::LParen) {
                self.bump();
                let _ = self.parse_param_list_body()?;
                self.expect(TokenKind::RParen)?;
                self.skip_trailing_gnu_attrs();
            }
            // Promote bubbled params from (*name(params)) form
            if bubbled_fp.is_some() {
                func_params = bubbled_fp.take();
            }
        }
        // GNU register asm: `register unsigned long sp asm("esp");`
        // Also appears as `__asm__("sp")` after the declarator name.
        if matches!(
            self.peek_kind(),
            TokenKind::Ident(s) if s == "asm" || s == "__asm" || s == "__asm__"
        ) {
            self.bump();
            if self.eat(TokenKind::LParen) {
                // one or more string literals
                while let TokenKind::StringLit(_) = self.peek_kind().clone() {
                    self.bump();
                }
                let _ = self.expect(TokenKind::RParen);
            }
        }
        Ok((name, ty, func_params))
    }

    /// Insert Array under outermost pointer chain: Ptr(T) + [n] → Ptr(Array(T,n)).
    fn array_under_ptrs(ty: Type, n: i64) -> Type {
        match ty {
            Type::Ptr(inner) => Type::Ptr(Box::new(Self::array_under_ptrs(*inner, n))),
            other => Type::Array(Box::new(other), n),
        }
    }

    fn align_up(n: i64, a: i64) -> i64 {
        if a <= 1 {
            return n;
        }
        (n + a - 1) & !(a - 1)
    }

    fn const_type_align(&self, ty: &Type) -> i64 {
        match ty.unqual() {
            Type::Void | Type::Char | Type::SChar | Type::UChar => 1,
            Type::Short | Type::UShort => 2,
            Type::Int | Type::UInt | Type::Float => 4,
            Type::Long | Type::ULong | Type::Double | Type::Ptr(_) => 8,
            Type::Array(e, _) => self.const_type_align(e),
            Type::Struct(n) | Type::Union(n) => self
                .layout_named(n)
                .map(|(_, a, _)| a)
                .unwrap_or(8),
            Type::AnonStruct(fs) => self.layout_fields_const(fs, false, false).1,
            Type::AnonUnion(fs) => self.layout_fields_const(fs, true, false).1,
            Type::Const(_) => unreachable!(),
        }
    }

    /// Layout from struct_fields (bit-offset packing, matches codegen).
    /// Returns (size, align, field_name → byte offset).
    /// When `packed` is true, field alignment is 1 and the overall align is 1
    /// (matches GCC `__attribute__((packed))`).
    ///
    /// Anonymous nested struct/union members are **flattened** into the parent
    /// map (same as codegen `layout_fields`). Skipping them made
    /// `offsetof(struct pt_regs, regs[N])` / `thread_info.preempt_count` return
    /// 0 and collapsed `sizeof(struct pt_regs)` — fatal for Linux asm-offsets.
    fn layout_fields_const(
        &self,
        fields: &[Field],
        is_union: bool,
        packed: bool,
    ) -> (i64, i64, std::collections::HashMap<String, i64>) {
        let mut map = std::collections::HashMap::new();
        let mut max_align = 1i64;
        let mut max_size = 0i64;
        let mut offset_bits: u64 = 0;

        for f in fields {
            // Anonymous nested struct/union: promote fields into this layout.
            if f.name.is_empty() && f.bit_width.is_none() {
                let nested_opt: Option<(i64, i64, std::collections::HashMap<String, i64>)> =
                    match &f.ty {
                        Type::AnonStruct(fs) => Some(self.layout_fields_const(fs, false, false)),
                        Type::AnonUnion(fs) => Some(self.layout_fields_const(fs, true, false)),
                        Type::Struct(n) | Type::Union(n) => self.layout_named(n),
                        _ => None,
                    };
                if let Some((nsz, nal, nmap)) = nested_opt {
                    let nalign = if packed { 1 } else { nal };
                    max_align = max_align.max(nalign);
                    if is_union {
                        for (fnm, fo) in &nmap {
                            map.insert(fnm.clone(), *fo);
                        }
                        max_size = max_size.max(nsz);
                    } else {
                        let mut byte_off = ((offset_bits + 7) / 8) as i64;
                        byte_off = Self::align_up(byte_off, nalign);
                        for (fnm, fo) in &nmap {
                            map.insert(fnm.clone(), byte_off + fo);
                        }
                        offset_bits = ((byte_off + nsz) as u64) * 8;
                    }
                    continue;
                }
                // Unknown empty-name field (e.g. incomplete type): skip size contribution.
                continue;
            }
            if let Some(width) = f.bit_width {
                let container_sz = self.const_type_size(&f.ty).unwrap_or(4).max(1) as u64;
                let container_bits = container_sz * 8;
                let al = if packed { 1 } else { self.const_type_align(&f.ty) };
                max_align = max_align.max(al);
                if is_union {
                    if !f.name.is_empty() && width > 0 {
                        map.insert(f.name.clone(), 0);
                    }
                    max_size = max_size.max(container_sz as i64);
                    continue;
                }
                if width == 0 {
                    let al_bits = (al as u64) * 8;
                    if al_bits > 0 {
                        offset_bits = ((offset_bits + al_bits - 1) / al_bits) * al_bits;
                    }
                    continue;
                }
                let w = width as u64;
                if container_bits > 0 && (offset_bits % container_bits) + w > container_bits {
                    offset_bits =
                        ((offset_bits + container_bits - 1) / container_bits) * container_bits;
                }
                let bit_pos = offset_bits;
                let cont_index = bit_pos / container_bits.max(1);
                let unit_start = (cont_index * container_sz) as i64;
                if !f.name.is_empty() {
                    map.insert(f.name.clone(), unit_start);
                }
                offset_bits = bit_pos + w;
                max_size = max_size.max(((offset_bits + 7) / 8) as i64);
                continue;
            }
            let sz = self.const_type_size(&f.ty).unwrap_or(8);
            let al = if packed { 1 } else { self.const_type_align(&f.ty) };
            max_align = max_align.max(al);
            if is_union {
                if !f.name.is_empty() {
                    map.insert(f.name.clone(), 0);
                }
                max_size = max_size.max(sz);
            } else {
                let mut byte_off = ((offset_bits + 7) / 8) as i64;
                byte_off = Self::align_up(byte_off, al);
                if !f.name.is_empty() {
                    map.insert(f.name.clone(), byte_off);
                }
                offset_bits = ((byte_off + sz) as u64) * 8;
            }
        }
        let final_align = if packed { 1 } else { max_align.max(1) };
        let size = if is_union {
            Self::align_up(max_size, final_align)
        } else {
            let byte_off = ((offset_bits + 7) / 8) as i64;
            Self::align_up(byte_off, final_align)
        };
        (size, final_align, map)
    }

    fn layout_named(
        &self,
        name: &str,
    ) -> Option<(i64, i64, std::collections::HashMap<String, i64>)> {
        let fields = self.struct_fields.get(name)?;
        let is_union = self.unions.iter().any(|u| u == name);
        let packed = self.packed_structs.contains(name);
        Some(self.layout_fields_const(fields, is_union, packed))
    }

    /// Sizeof for constant-expression evaluation at parse time (array bounds).
    fn const_type_size(&self, ty: &Type) -> Option<i64> {
        Some(match ty.unqual() {
            Type::Void => 0,
            Type::Char | Type::SChar | Type::UChar => 1,
            Type::Short | Type::UShort => 2,
            Type::Int | Type::UInt | Type::Float => 4,
            Type::Long | Type::ULong | Type::Double | Type::Ptr(_) => 8,
            Type::Array(e, n) => {
                if *n <= 0 {
                    return None;
                }
                self.const_type_size(e)? * n
            }
            Type::Struct(n) | Type::Union(n) => {
                return self.layout_named(n).map(|(s, _, _)| s);
            }
            Type::AnonStruct(fs) => self.layout_fields_const(fs, false, false).0,
            Type::AnonUnion(fs) => self.layout_fields_const(fs, true, false).0,
            Type::Const(_) => unreachable!(),
        })
    }

    /// Field byte offset within a struct/union type (named or anonymous).
    fn const_offsetof_type_field(&self, ty: &Type, field: &str) -> Option<i64> {
        match ty {
            Type::Struct(n) | Type::Union(n) => {
                let lay = self.layout_named(n)?;
                lay.2.get(field).copied()
            }
            Type::AnonStruct(fs) => {
                let lay = self.layout_fields_const(fs, false, false);
                lay.2.get(field).copied()
            }
            Type::AnonUnion(fs) => {
                let lay = self.layout_fields_const(fs, true, false);
                lay.2.get(field).copied()
            }
            // typedef alias stored as Struct(name) already handled; peel Ptr? no.
            _ => None,
        }
    }

    /// Resolve field type, searching into anonymous nested struct/union members.
    fn find_field_type(&self, ty: &Type, field: &str) -> Option<Type> {
        let fields: &[Field] = match ty {
            Type::Struct(n) | Type::Union(n) => self.struct_fields.get(n)?.as_slice(),
            Type::AnonStruct(fs) | Type::AnonUnion(fs) => fs.as_slice(),
            _ => return None,
        };
        for f in fields {
            if f.name == field {
                return Some(f.ty.clone());
            }
            if f.name.is_empty() && f.bit_width.is_none() {
                if let Some(t) = self.find_field_type(&f.ty, field) {
                    return Some(t);
                }
            }
        }
        None
    }

    /// Nested offsetof path: `a.b.c` → sum of successive field offsets.
    /// Each path element is `(field_name, array_indices)` so
    /// `regs[2]` / `stackframe[1]` add `index * elem_size`.
    fn const_offsetof_type_path(
        &self,
        ty: &Type,
        path: &[(String, Vec<i64>)],
    ) -> Option<i64> {
        let mut cur = ty.clone();
        let mut total = 0i64;
        for (field, indices) in path {
            let off = self.const_offsetof_type_field(&cur, field)?;
            total += off;
            let mut fty = self.find_field_type(&cur, field)?;
            for &idx in indices {
                match &fty {
                    Type::Array(elem, _) => {
                        let esz = self.const_type_size(elem)?;
                        total += idx * esz;
                        fty = elem.as_ref().clone();
                    }
                    // Soft: treat pointer as array for offsetof(T, p[n]) rare cases.
                    Type::Ptr(elem) => {
                        let esz = self.const_type_size(elem)?;
                        total += idx * esz;
                        fty = elem.as_ref().clone();
                    }
                    _ => return None,
                }
            }
            cur = fty;
        }
        Some(total)
    }

    /// Evaluate `offsetof(T, field)` patterns:
    /// `((int)((char*)&((T*)0)->field))` or `&((T*)0)->field`.
    fn const_offsetof(&self, e: &Expr) -> Option<i64> {
        // Strip outer casts
        let e = match e {
            Expr::Cast { expr, .. } => expr.as_ref(),
            other => other,
        };
        let e = match e {
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => expr.as_ref(),
            other => other,
        };
        match e {
            Expr::Member { base, field, .. } => {
                // base should be (T*)0 or cast chain ending at null
                let mut b = base.as_ref();
                let mut ty: Option<Type> = None;
                loop {
                    match b {
                        Expr::Cast { ty: t, expr } => {
                            ty = Some(t.clone());
                            b = expr.as_ref();
                        }
                        Expr::Int(0) | Expr::Char(0) => break,
                        _ => return None,
                    }
                }
                let ty = ty?;
                let (struct_name, fields_map) = match ty {
                    Type::Ptr(inner) => match *inner {
                        Type::Struct(n) | Type::Union(n) => {
                            let lay = self.layout_named(&n)?;
                            (n, lay.2)
                        }
                        Type::AnonStruct(fs) => {
                            let lay = self.layout_fields_const(&fs, false, false);
                            (String::new(), lay.2)
                        }
                        _ => return None,
                    },
                    Type::Struct(n) | Type::Union(n) => {
                        let lay = self.layout_named(&n)?;
                        (n, lay.2)
                    }
                    _ => return None,
                };
                let _ = struct_name;
                fields_map.get(field).copied()
            }
            _ => None,
        }
    }

    pub fn int_lit_type(n: i64) -> Type {
        if n < 0 {
            Type::ULong
        } else if n > u32::MAX as i64 {
            Type::Long
        } else if n > i32::MAX as i64 {
            Type::UInt
        } else {
            Type::Int
        }
    }

    fn strip_qualifiers_and_decay(&self, ty: Type) -> Type {
        let unqual = ty.unqual().clone();
        match unqual {
            Type::Array(elem, _) => Type::Ptr(elem),
            other => other,
        }
    }

    fn types_compatible(&self, t1: &Type, t2: &Type) -> bool {
        let t1 = self.strip_qualifiers_and_decay(t1.clone());
        let t2 = self.strip_qualifiers_and_decay(t2.clone());
        match (&t1, &t2) {
            (Type::Int, Type::Int)
            | (Type::UInt, Type::UInt)
            | (Type::Char, Type::Char)
            | (Type::SChar, Type::SChar)
            | (Type::UChar, Type::UChar)
            | (Type::Short, Type::Short)
            | (Type::UShort, Type::UShort)
            | (Type::Long, Type::Long)
            | (Type::ULong, Type::ULong)
            | (Type::Float, Type::Float)
            | (Type::Double, Type::Double)
            | (Type::Void, Type::Void) => true,
            (Type::Ptr(a), Type::Ptr(b)) => self.types_compatible(a, b),
            (Type::Struct(a), Type::Struct(b)) => a == b,
            (Type::Union(a), Type::Union(b)) => a == b,
            _ => false,
        }
    }

    /// Best-effort type of an expression for parse-time sizeof/array bounds.
    /// Uses local_types (params + locals) and struct field layouts.
    fn const_expr_type(&self, e: &Expr) -> Option<Type> {
        match e {
            Expr::Int(n) => Some(Self::int_lit_type(*n)),
            Expr::Char(_) => Some(Type::Int),
            Expr::Float(_) => Some(Type::Double),
            Expr::String(s) => {
                Some(Type::Array(Box::new(Type::Char), (s.len() + 1) as i64))
            }
            Expr::Var(name) => {
                if let Some(t) = self.lookup_local_type(name) {
                    return Some(t.clone());
                }
                if let Some(t) = self.global_types.get(name) {
                    return Some(t.clone());
                }
                if self.enum_values.contains_key(name) {
                    return Some(Type::Int);
                }
                None
            }
            Expr::Member { base, field, arrow } => {
                let mut bt = self.const_expr_type(base)?;
                if *arrow {
                    bt = match bt {
                        Type::Ptr(inner) => *inner,
                        Type::Array(inner, _) => *inner,
                        _ => return None,
                    };
                }
                self.find_field_type(&bt, field)
            }
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
            } => match self.const_expr_type(expr)? {
                Type::Ptr(i) | Type::Array(i, _) => Some(*i),
                _ => None,
            },
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => Some(Type::Ptr(Box::new(self.const_expr_type(expr)?))),
            Expr::Unary {
                op: UnaryOp::Neg | UnaryOp::Not | UnaryOp::BitNot,
                expr,
            } => self.const_expr_type(expr),
            Expr::Index { base, .. } => match self.const_expr_type(base)? {
                Type::Ptr(i) | Type::Array(i, _) => Some(*i),
                _ => None,
            },
            Expr::Cast { ty, .. } => Some(ty.clone()),
            Expr::SizeofType(_) | Expr::SizeofExpr(_) => Some(Type::ULong),
            Expr::Binary { left, right, .. } => {
                // Prefer floating / pointer width when either side has it.
                let lt = self.const_expr_type(left);
                let rt = self.const_expr_type(right);
                let l_unqual = lt.as_ref().map(|t| t.unqual());
                let r_unqual = rt.as_ref().map(|t| t.unqual());
                match (l_unqual, r_unqual) {
                    (Some(Type::Double), _) | (_, Some(Type::Double)) => Some(Type::Double),
                    (Some(Type::Float), _) | (_, Some(Type::Float)) => Some(Type::Float),
                    (Some(Type::Ptr(p)), _) | (_, Some(Type::Ptr(p))) => {
                        Some(Type::Ptr(p.clone()))
                    }
                    (Some(Type::ULong), _) | (_, Some(Type::ULong)) => Some(Type::ULong),
                    (Some(Type::Long), _) | (_, Some(Type::Long)) => Some(Type::Long),
                    (Some(Type::UInt), _) | (_, Some(Type::UInt)) => Some(Type::UInt),
                    _ => Some(Type::Int),
                }
            }
            Expr::Cond {
                then_e, else_e, ..
            } => self
                .const_expr_type(then_e)
                .or_else(|| self.const_expr_type(else_e)),
            Expr::Call { .. } => Some(Type::Int),
            Expr::PreInc(e) | Expr::PreDec(e) | Expr::PostInc(e) | Expr::PostDec(e) => {
                self.const_expr_type(e)
            }
            Expr::Assign { left, .. } | Expr::CompoundAssign { left, .. } => {
                self.const_expr_type(left)
            }
            Expr::StmtExpr(_, final_e) => self.const_expr_type(final_e),
            Expr::InitList { .. } => None,
            Expr::AddrOfLabel(_) => Some(Type::Ptr(Box::new(Type::Void))),
        }
    }

    fn const_array_len(&self, e: &Expr) -> Option<i64> {
        // Try offsetof first (before generic cast peel loses Addr).
        if let Some(o) = self.const_offsetof(e) {
            return Some(o);
        }
        match e {
            Expr::Int(n) | Expr::Char(n) => Some(*n),
            Expr::Var(name) => self.enum_values.get(name).copied(),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(-self.const_array_len(expr)?),
            Expr::Unary {
                op: UnaryOp::Addr,
                expr,
            } => self.const_offsetof(&Expr::Unary {
                op: UnaryOp::Addr,
                expr: expr.clone(),
            }),
            Expr::Binary { op, left, right } => {
                let l = self.const_array_len(left)?;
                let r = self.const_array_len(right)?;
                Some(match op {
                    BinOp::Add => l.wrapping_add(r),
                    BinOp::Sub => l.wrapping_sub(r),
                    BinOp::Mul => l.wrapping_mul(r),
                    BinOp::Div if r != 0 => l / r,
                    BinOp::Mod if r != 0 => l % r,
                    BinOp::Shl => l.wrapping_shl((r as u32) & 63),
                    BinOp::Shr => ((l as u64) >> ((r as u32) & 63)) as i64,
                    BinOp::BitAnd => l & r,
                    BinOp::BitOr => l | r,
                    BinOp::BitXor => l ^ r,
                    BinOp::Eq => (l == r) as i64,
                    BinOp::Ne => (l != r) as i64,
                    BinOp::Lt => (l < r) as i64,
                    BinOp::Gt => (l > r) as i64,
                    BinOp::Le => (l <= r) as i64,
                    BinOp::Ge => (l >= r) as i64,
                    BinOp::And => ((l != 0) && (r != 0)) as i64,
                    BinOp::Or => ((l != 0) || (r != 0)) as i64,
                    _ => return None,
                })
            }
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
            } => Some((self.const_array_len(expr)? == 0) as i64),
            Expr::Unary {
                op: UnaryOp::BitNot,
                expr,
            } => Some(!self.const_array_len(expr)?),
            // GCC builtins used by kernel log2.h / order_base_2 / DEFINE "i"(…).
            Expr::Call { name, args } => match name.as_str() {
                "__builtin_constant_p" => {
                    // 1 if argument is a foldable constant expression.
                    Some(if args.len() == 1 && self.const_array_len(&args[0]).is_some() {
                        1
                    } else {
                        0
                    })
                }
                "__builtin_clzll" | "__builtin_clzl" | "__builtin_clz" => {
                    let n = self.const_array_len(args.first()?)? as u64;
                    if n == 0 {
                        // GCC: undefined for 0; kernel const paths avoid it.
                        return None;
                    }
                    let width = if name == "__builtin_clz" { 32u32 } else { 64u32 };
                    Some((n.leading_zeros() - (64 - width)) as i64)
                }
                "__builtin_ctzll" | "__builtin_ctzl" | "__builtin_ctz" => {
                    let n = self.const_array_len(args.first()?)? as u64;
                    if n == 0 {
                        return None;
                    }
                    Some(n.trailing_zeros() as i64)
                }
                "__builtin_popcount" => {
                    let n = self.const_array_len(args.first()?)? as u32;
                    Some(n.count_ones() as i64)
                }
                "__builtin_popcountl" | "__builtin_popcountll" => {
                    let n = self.const_array_len(args.first()?)? as u64;
                    Some(n.count_ones() as i64)
                }
                _ => None,
            },
            Expr::Cast { expr, .. } => {
                // Cast may wrap offsetof; try offsetof on full expr first already done.
                // Also try offsetof on inner with outer cast re-wrapped.
                if let Some(o) = self.const_offsetof(expr) {
                    return Some(o);
                }
                self.const_array_len(expr)
            }
            Expr::SizeofType(ty) => self.const_type_size(ty),
            Expr::SizeofExpr(ex) => match ex.as_ref() {
                Expr::SizeofType(ty) | Expr::Cast { ty, .. } => self.const_type_size(ty),
                Expr::String(s) => Some((s.len() + 1) as i64),
                other => {
                    // sizeof(expr) must use the *type* of expr, not evaluate expr
                    // as an integer. Critical for array bounds like:
                    //   char dbFileVers[sizeof(pPager->dbFileVers)];
                    // Previously this fell through to const_array_len(Member)
                    // which returned None → unwrap_or(0) → zero-length array →
                    // sqlite3OsRead(..., amt=0) and multi-connection cache
                    // invalidation never saw peer writes.
                    let ty = self.const_expr_type(other)?;
                    match &ty {
                        Type::Array(elem, n) if *n > 0 => {
                            Some(self.const_type_size(elem)? * *n)
                        }
                        Type::Array(_, n) if *n <= 0 => None,
                        other_ty => self.const_type_size(other_ty),
                    }
                }
            },
            Expr::Cond {
                cond,
                then_e,
                else_e,
            } => {
                let c = self.const_array_len(cond)?;
                if c != 0 {
                    self.const_array_len(then_e)
                } else {
                    self.const_array_len(else_e)
                }
            }
            Expr::Member { .. } => self.const_offsetof(e),
            _ => None,
        }
    }

    /// Skip leftover `__attribute__` / `__section__` / similar after a declarator.
    /// Does NOT consume `Weak` — that sticky token is ownership of the caller
    /// (must set `is_weak` on Function for COND_SYSCALL).
    fn skip_trailing_gnu_attrs(&mut self) {
        loop {
            match self.peek_kind().clone() {
                TokenKind::Section(sec) => {
                    self.last_section = Some(sec);
                    self.bump();
                }
                TokenKind::Packed | TokenKind::StringLit(_) => {
                    self.bump();
                }
                TokenKind::Weak => break, // sticky — leave for parse_decl_or_func
                TokenKind::Ident(s) => {
                    if !Self::is_gnu_attr_name(&s) {
                        break;
                    }
                    self.bump();
                    if self.at(&TokenKind::LParen) {
                        let _ = self.skip_balanced_parens();
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_decl_or_func(
        &mut self,
        file_static: bool,
        file_extern: bool,
    ) -> Result<Vec<Item>, String> {
        let mut is_static = file_static;
        let mut is_extern = file_extern;
        let mut saw_inline = false;
        let mut is_register = false;
        let mut is_weak = false;
        loop {
            if self.eat(TokenKind::Static) {
                is_static = true;
                continue;
            }
            if self.eat(TokenKind::Inline) {
                saw_inline = true;
                continue;
            }
            if self.eat(TokenKind::Extern) {
                is_extern = true;
                continue;
            }
            if self.eat(TokenKind::Register) {
                // Plain C `register T x;` is just a stack-local hint — NOT
                // static duration. Treating it as static made
                // `register struct vars *v = &var` emit `.quad var` into .data
                // (U var at link) for Postgres regexec.
                is_register = true;
                continue;
            }
            if self.eat(TokenKind::Auto) {
                continue;
            }
            if self.eat(TokenKind::Weak) {
                is_weak = true;
                continue;
            }
            break;
        }
        let _ = is_register;
        let mut sec_attr: Option<String> = None;
        if let TokenKind::Section(sec) = self.peek_kind().clone() {
            sec_attr = Some(sec);
            self.bump();
        }
        let base = self.parse_type_specifier()?;
        sec_attr = self.last_section.take().or(sec_attr);
        if let TokenKind::Section(sec) = self.peek_kind().clone() {
            sec_attr = Some(sec);
            self.bump();
        }
        // `long __weak foo(...)` — weak sits between type and declarator.
        while self.eat(TokenKind::Weak) {
            is_weak = true;
        }
        while self.eat(TokenKind::Packed) {}
        // type_specifier may still consume residual static/inline interleaved
        // with type keywords; re-check is unnecessary for body-skip heuristics.
        // Could be: type name(...) { }  or type name, name2;
        // Function params are part of the declarator (including multi-suffix forms).
        let (name, ty, func_params) = self.parse_declarator(base.clone())?;
        sec_attr = self.last_section.take().or(sec_attr);
        if let TokenKind::Section(sec) = self.peek_kind().clone() {
            sec_attr = Some(sec);
            self.bump();
        }
        self.skip_trailing_gnu_attrs();
        // Weak may appear after the declarator as sticky token.
        while self.eat(TokenKind::Weak) {
            is_weak = true;
        }
        if name.is_empty() {
            if self.eat(TokenKind::Semicolon) {
                return Ok(Vec::new());
            }
            return Err(format!(
                "expected declarator name at {}:{}",
                self.peek().line,
                self.peek().col
            ));
        }
        if let Some((params, variadic)) = func_params {
            // Skip residual GNU attributes / section macros / kernel function annotations after the declarator
            loop {
                self.skip_trailing_gnu_attrs();
                while self.eat(TokenKind::Weak) {
                    is_weak = true;
                }
                sec_attr = self.last_section.take().or(sec_attr);
                if self.at(&TokenKind::LBrace) || self.at(&TokenKind::Semicolon) || self.at(&TokenKind::Eof) {
                    break;
                }
                if let TokenKind::Ident(_) = self.peek_kind().clone() {
                    self.bump();
                    if self.at(&TokenKind::LParen) {
                        let _ = self.skip_balanced_parens();
                    }
                    continue;
                }
                break;
            }
            // Function prototype or definition
            if self.eat(TokenKind::Semicolon) {
                return Ok(vec![Item::Func(Function {
                    name,
                    ret: ty,
                    params,
                    variadic,
                    body: None,
                    is_static,
                    is_weak,
                    section: sec_attr,
                })]);
            }
            if self.at(&TokenKind::LBrace) {
                let sec_attr = self.last_section.take().or(sec_attr);
                // Params visible to sizeof in array bounds inside the body:
                //   char dbFileVers[sizeof(pPager->dbFileVers)];
                self.push_scope();
                for (pn, pt) in &params {
                    self.insert_local_type(pn, pt.clone());
                }
                let body = Some(self.parse_block()?);
                self.pop_scope();
                return Ok(vec![Item::Func(Function {
                    name,
                    ret: ty,
                    params,
                    variadic,
                    body,
                    is_static: is_static || saw_inline,
                    is_weak,
                    section: sec_attr,
                })]);
            }
            if self.eat(TokenKind::Comma) {
                // int f(int a), g(int a), x;
                let mut items = vec![Item::Func(Function {
                    name,
                    ret: ty.clone(),
                    params,
                    variadic,
                    body: None,
                    is_static,
                    is_weak,
                    section: sec_attr.clone(),
                })];
                loop {
                    let (n2, t2, fp2) = self.parse_declarator(base.clone())?;
                    if let Some((p2, v2)) = fp2 {
                        items.push(Item::Func(Function {
                            name: n2,
                            ret: t2,
                            params: p2,
                            variadic: v2,
                            body: None,
                            is_static,
                            is_weak,
                            section: sec_attr.clone(),
                        }));
                    } else {
                        let init = if self.eat(TokenKind::Assign) {
                            Some(self.parse_initializer()?)
                        } else {
                            None
                        };
                        items.push(Item::Global(VarDecl {
                            name: n2,
                            ty: t2,
                            init,
                            is_static,
                            is_extern,
                            is_weak,
                            section: sec_attr.clone(),
                        }));
                    }
                    if self.eat(TokenKind::Comma) {
                        continue;
                    }
                    break;
                }
                self.expect(TokenKind::Semicolon)?;
                return Ok(items);
            }
            return Err(format!(
                "expected function body or ';' after declarator at {}:{}",
                self.peek().line,
                self.peek().col
            ));
        }
        // variable(s)
        let mut items = Vec::new();
        sec_attr = self.last_section.take().or(sec_attr);
        if let TokenKind::Section(sec) = self.peek_kind().clone() {
            sec_attr = Some(sec);
            self.bump();
        }
        let init = if self.eat(TokenKind::Assign) {
            Some(self.parse_initializer()?)
        } else {
            None
        };
        let ty = self.infer_array_size(ty, &init);
        // Sticky weak: once a name is declared __weak in this TU, later
        // definitions (e.g. version.c #include of version-timestamp.c) stay weak.
        let mut is_weak = is_weak;
        if is_weak {
            self.weak_names.insert(name.clone());
        } else if self.weak_names.contains(&name) {
            is_weak = true;
        }
        self.insert_global_type(&name, ty.clone());
        items.push(Item::Global(VarDecl {
            name: name.clone(),
            ty: ty.clone(),
            init,
            is_static,
            is_extern,
            is_weak,
            section: sec_attr.clone(),
        }));
        while self.eat(TokenKind::Comma) {
            self.skip_trailing_gnu_attrs();
            let (n2, t2, _) = self.parse_declarator(base.clone())?;
            self.skip_trailing_gnu_attrs();
            let mut sec2 = self.last_section.take().or(sec_attr.clone());
            if let TokenKind::Section(sec) = self.peek_kind().clone() {
                sec2 = Some(sec);
                self.bump();
            }
            let init2 = if self.eat(TokenKind::Assign) {
                Some(self.parse_initializer()?)
            } else {
                None
            };
            let t2 = self.infer_array_size(t2, &init2);
            let mut w2 = is_weak;
            if w2 {
                self.weak_names.insert(n2.clone());
            } else if self.weak_names.contains(&n2) {
                w2 = true;
            }
            self.insert_global_type(&n2, t2.clone());
            items.push(Item::Global(VarDecl {
                name: n2,
                ty: t2,
                init: init2,
                is_static,
                is_extern,
                is_weak: w2,
                section: sec2,
            }));
        }
        while !self.at(&TokenKind::Semicolon) && !self.at(&TokenKind::Comma) && !self.at(&TokenKind::Eof) {
            self.skip_trailing_gnu_attrs();
            if self.at(&TokenKind::Semicolon) || self.at(&TokenKind::Comma) || self.at(&TokenKind::Eof) {
                break;
            }
            self.bump();
            if self.at(&TokenKind::LParen) {
                let _ = self.skip_balanced_parens();
            }
        }
        self.eat(TokenKind::Semicolon);
        let _ = &ty;
        Ok(items)
    }

    fn scalar_count(&self, ty: &Type) -> usize {
        match ty {
            Type::Array(elem, len) => ((*len).max(1) as usize) * self.scalar_count(elem),
            Type::Struct(name) => {
                if let Some(fs) = self.struct_fields.get(name) {
                    fs.iter().map(|f| self.scalar_count(&f.ty)).sum()
                } else {
                    1
                }
            }
            Type::Union(name) => {
                if let Some(fs) = self.struct_fields.get(name) {
                    fs.iter().map(|f| self.scalar_count(&f.ty)).max().unwrap_or(1)
                } else {
                    1
                }
            }
            Type::AnonStruct(fs) => fs.iter().map(|f| self.scalar_count(&f.ty)).sum(),
            Type::AnonUnion(fs) => fs.iter().map(|f| self.scalar_count(&f.ty)).max().unwrap_or(1),
            _ => 1,
        }
    }

    fn infer_array_size(&self, ty: Type, init: &Option<Expr>) -> Type {
        match (&ty, init) {
            (Type::Array(elem, 0), Some(Expr::String(s))) => {
                Type::Array(elem.clone(), (s.len() as i64) + 1)
            }
            (Type::Array(elem, 0), Some(Expr::InitList { fields })) => {
                let mut cur = 0i64;
                let mut high = 0i64;
                for (des, _) in fields {
                    if let Some(d) = des {
                        if let Ok(i) = d.parse::<i64>() {
                            cur = i;
                        }
                    }
                    high = high.max(cur + 1);
                    cur += 1;
                }
                let elem_sc = self.scalar_count(elem).max(1) as i64;
                let has_sub_initlists = fields.iter().any(|(_, e)| matches!(e, Expr::InitList { .. }));
                let count = if !has_sub_initlists && elem_sc > 1 {
                    (high + elem_sc - 1) / elem_sc
                } else {
                    high
                };
                Type::Array(elem.clone(), count.max(1))
            }
            _ => ty,
        }
    }

    /// Initializer: expression or brace list { e, e, .field = e }
    fn parse_initializer(&mut self) -> Result<Expr, String> {
        if self.eat(TokenKind::LBrace) {
            let mut fields = Vec::new();
            while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                if self.eat(TokenKind::Dot) {
                    // Nested designators: `.memory.regions = expr` and `.base[0] = expr`
                    // (kernel memblock / timekeeping / page-table types).
                    // Keep the FULL path ("memory.regions") so codegen can apply
                    // nested struct fields — innermost-only dropped regions/cnt/max
                    // and left memblock.memory.regions NULL → panic in double_array.
                    let mut field = if let TokenKind::Ident(s) = self.peek_kind().clone() {
                        self.bump();
                        s
                    } else {
                        return Err("designated init field name".into());
                    };
                    loop {
                        if self.eat(TokenKind::Dot) {
                            if let TokenKind::Ident(s) = self.peek_kind().clone() {
                                self.bump();
                                field = format!("{field}.{s}");
                            } else {
                                return Err("nested designated init field name".into());
                            }
                            continue;
                        }
                        // `.field[n]` / `.field[lo ... hi]` — soft: keep field name,
                        // consume index designator chain.
                        if self.eat(TokenKind::LBracket) {
                            while !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                                self.bump();
                            }
                            self.expect(TokenKind::RBracket)?;
                            continue;
                        }
                        break;
                    }
                    self.expect(TokenKind::Assign)?;
                    fields.push((Some(field), self.parse_initializer()?));
                } else if self.eat(TokenKind::LBracket) {
                    // designated array index: [n], [1+2], [ENUM], GNU [lo ... hi]
                    // Always parse full constant expressions so `1 + 2` is not
                    // left as IntLit then Plus before `]`.
                    let idx_str = if self.at(&TokenKind::RBracket)
                        || self.at(&TokenKind::Ellipsis)
                    {
                        "0".into()
                    } else {
                        match self.parse_assign() {
                            Ok(e) => self
                                .eval_enum_const(&e)
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "0".into()),
                            Err(_) => {
                                while !self.at(&TokenKind::RBracket)
                                    && !self.at(&TokenKind::Ellipsis)
                                    && !self.at(&TokenKind::Eof)
                                {
                                    self.bump();
                                }
                                "0".into()
                            }
                        }
                    };
                    // GNU range designator [lo ... hi] — hi may be `16 - 1` / `n + 1`.
                    if self.eat(TokenKind::Ellipsis) {
                        if !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                            let _ = self.parse_assign();
                        }
                    }
                    self.expect(TokenKind::RBracket)?;
                    // Multi-dimensional: `[i][j] = expr` (cpumask bitmap tables).
                    while self.eat(TokenKind::LBracket) {
                        if !self.at(&TokenKind::RBracket) && !self.at(&TokenKind::Eof) {
                            if self.eat(TokenKind::Ellipsis) {
                                // rare
                            } else {
                                let _ = self.parse_assign();
                            }
                            if self.eat(TokenKind::Ellipsis)
                                && !self.at(&TokenKind::RBracket)
                                && !self.at(&TokenKind::Eof)
                            {
                                let _ = self.parse_assign();
                            }
                        }
                        self.expect(TokenKind::RBracket)?;
                    }
                    self.expect(TokenKind::Assign)?;
                    fields.push((Some(idx_str), self.parse_initializer()?));
                } else {
                    fields.push((None, self.parse_initializer()?));
                }
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                break;
            }
            self.expect(TokenKind::RBrace)?;
            Ok(Expr::InitList { fields })
        } else {
            // Use assign-level expr so comma operator does not swallow
            // brace-list separators: `{1, 2, 3}` must stay three elements.
            self.parse_assign()
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(TokenKind::RBrace)?;
        Ok(stmts)
    }

    fn skip_balanced_braces_offset(&self, start: usize) -> usize {
        if !matches!(self.toks.get(start).map(|t| &t.kind), Some(TokenKind::LBrace)) {
            return start;
        }
        let mut depth = 1i32;
        let mut idx = start + 1;
        while depth > 0 && idx < self.toks.len() {
            match self.toks[idx].kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return idx + 1;
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        idx
    }

    /// Skip a balanced `(...)` starting at `start` index and return index after `)`.
    fn skip_balanced_parens_offset(&self, start: usize) -> usize {
        if !matches!(self.toks.get(start).map(|t| &t.kind), Some(TokenKind::LParen)) {
            return start;
        }
        let mut depth = 1i32;
        let mut idx = start + 1;
        while depth > 0 && idx < self.toks.len() {
            match self.toks[idx].kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return idx + 1;
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        idx
    }

    /// Skip GNU attributes starting at `idx` token offset.
    fn skip_gnu_attrs_offset(&self, mut idx: usize) -> usize {
        loop {
            if idx < self.toks.len() && matches!(self.toks[idx].kind, TokenKind::Packed | TokenKind::Section(_)) {
                idx += 1;
                continue;
            }
            // Require `(` after attr spelling so bare Ident `attribute` (Redis
            // field) is never skipped as a GNU attribute.
            if idx < self.toks.len()
                && matches!(
                    &self.toks[idx].kind,
                    TokenKind::Ident(s) if s.starts_with("__attribute")
                )
            {
                idx += 1;
                if idx < self.toks.len() && self.toks[idx].kind == TokenKind::LParen {
                    idx = self.skip_balanced_parens_offset(idx);
                }
                continue;
            }
            break;
        }
        idx
    }

    /// Skip a balanced `(...)` starting at the current token (must be `(`).
    fn skip_balanced_parens(&mut self) -> Result<(), String> {
        self.expect(TokenKind::LParen)?;
        let mut depth = 1i32;
        while depth > 0 && !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::LBrace) {
                let _ = self.skip_balanced_braces();
                continue;
            }
            if self.at(&TokenKind::LParen) {
                depth += 1;
            } else if self.at(&TokenKind::RParen) {
                depth -= 1;
                if depth == 0 {
                    self.bump();
                    break;
                }
            }
            self.bump();
        }
        Ok(())
    }

    /// Skip a balanced `{...}` starting at the current token (must be `{`).
    /// String/char literals are single tokens so braces inside them do not
    /// affect depth (lexer already tokenized them).
    fn skip_balanced_braces(&mut self) -> Result<(), String> {
        self.expect(TokenKind::LBrace)?;
        let mut depth = 1i32;
        let mut steps = 0u64;
        while depth > 0 && !self.at(&TokenKind::Eof) {
            steps += 1;
            // Safety: never walk the whole file forever on a broken brace match.
            if steps > 50_000_000 {
                return Err("skip_balanced_braces: too many tokens".into());
            }
            if self.at(&TokenKind::LBrace) {
                depth += 1;
                self.bump();
                continue;
            }
            if self.at(&TokenKind::RBrace) {
                depth -= 1;
                self.bump();
                if depth == 0 {
                    break;
                }
                continue;
            }
            self.bump();
        }
        Ok(())
    }

    /// GNU basic/extended asm for kbuild DEFINE / OFFSETS emission.
    fn parse_asm_stmt(&mut self) -> Result<Stmt, String> {
        // consume asm / __asm__ / __asm
        self.bump();
        // optional qualifiers: inline / volatile / goto (any order, may repeat)
        // e.g. `asm __inline volatile (...)` / `asm goto (...)` from kernel.
        // Note: `goto` is TokenKind::Goto, not Ident.
        loop {
            if self.eat(TokenKind::Volatile)
                || self.eat(TokenKind::Inline)
                || self.eat(TokenKind::Goto)
            {
                continue;
            }
            if let TokenKind::Ident(s) = self.peek_kind().clone() {
                if s == "__volatile__" || s == "__inline" || s == "__inline__" {
                    self.bump();
                    continue;
                }
            }
            break;
        }

        // Kernel ALTERNATIVE/asm_inline macros can leave non-string tokens in the
        // template position. Prefer structured parse; on failure, skip balanced
        // `(...)` so headers still parse (DEFINE uses clean string templates).
        //
        // Special case: `asm(ALTERNATIVE("old", "new", CAP) : …)` when the
        // preprocessor did not expand ALTERNATIVE (acc PP gap). Use the first
        // string (oldinstr = default non-cap path) as the template so critical
        // paths like `msr tpidr_el1` / `mrs tpidr_el1` are not dropped to empty
        // (that left __my_cpu_offset == garbage and percpu FAR=0).
        if !self.at(&TokenKind::LParen) {
            // Bare `asm;` / broken macro residue — treat as empty asm.
            let _ = self.eat(TokenKind::Semicolon);
            return Ok(Stmt::Asm {
                lines: Vec::new(),
                in_loads: Vec::new(),
                out_stores: Vec::new(),
                out_store_exprs: Vec::new(),
            });
        }
        let after = self.toks.get(self.i + 1).map(|t| &t.kind);
        let is_alternative = matches!(
            after,
            Some(TokenKind::Ident(s)) if s == "ALTERNATIVE" || s == "_ALTERNATIVE_CFG"
                || s == "__ALTERNATIVE_CFG"
        );
        let clean_template = matches!(
            after,
            Some(TokenKind::StringLit(_) | TokenKind::RParen)
        ) || is_alternative;
        if !clean_template {
            self.skip_balanced_parens()?;
            let _ = self.eat(TokenKind::Semicolon);
            return Ok(Stmt::Asm {
                lines: Vec::new(),
                in_loads: Vec::new(),
                out_stores: Vec::new(),
                out_store_exprs: Vec::new(),
            });
        }

        self.expect(TokenKind::LParen)?;

        // Peel ALTERNATIVE("oldinstr", "newinstr", cap[, cfg…]) → oldinstr only.
        if matches!(
            self.peek_kind(),
            TokenKind::Ident(s) if s == "ALTERNATIVE"
                || s == "_ALTERNATIVE_CFG"
                || s == "__ALTERNATIVE_CFG"
        ) {
            self.bump();
            self.expect(TokenKind::LParen)?;
            // First arg: oldinstr (string or adjacent strings)
            let mut old = String::new();
            loop {
                if let TokenKind::StringLit(s) = self.peek_kind().clone() {
                    self.bump();
                    old.push_str(&s);
                } else {
                    break;
                }
            }
            // Skip remaining ALTERNATIVE args to matching ')'
            let mut depth = 1i32;
            while depth > 0 && !self.at(&TokenKind::Eof) {
                if self.at(&TokenKind::LParen) {
                    depth += 1;
                } else if self.at(&TokenKind::RParen) {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        break;
                    }
                }
                self.bump();
            }
            // Operands follow ALTERNATIVE(...) inside outer asm(...).
            if old.is_empty() {
                while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                    self.bump();
                }
                let _ = self.eat(TokenKind::RParen);
                let _ = self.eat(TokenKind::Semicolon);
                return Ok(Stmt::Asm {
                    lines: Vec::new(),
                    in_loads: Vec::new(),
                    out_stores: Vec::new(),
                    out_store_exprs: Vec::new(),
                });
            }
            return self.finish_asm_stmt_with_template(old);
        }

        // Template: zero or more adjacent string literals (empty "" is a valid
        // memory-barrier asm used throughout the kernel).
        let mut template = String::new();
        let mut saw_template = false;
        loop {
            if let TokenKind::StringLit(s) = self.peek_kind().clone() {
                self.bump();
                template.push_str(&s);
                saw_template = true;
            } else {
                break;
            }
        }
        if !saw_template {
            // Empty template only if we already saw "" — handled by saw_template.
            // If truly no string, skip rest of operands.
            // (Should not reach here when clean_template was true for RParen.)
            while !self.at(&TokenKind::RParen) && !self.at(&TokenKind::Eof) {
                self.bump();
            }
            let _ = self.eat(TokenKind::RParen);
            let _ = self.eat(TokenKind::Semicolon);
            return Ok(Stmt::Asm {
                lines: Vec::new(),
                in_loads: Vec::new(),
                out_stores: Vec::new(),
                out_store_exprs: Vec::new(),
            });
        }

        self.finish_asm_stmt_with_template(template)
    }

    /// Shared tail of `parse_asm_stmt` after the template string is known
    /// (plain string or first arg of unexpanded `ALTERNATIVE(...)`).
    fn finish_asm_stmt_with_template(&mut self, template: String) -> Result<Stmt, String> {
        // Kernel ALTERNATIVE macros often expand to string + bare tokens + more strings.
        // If the template is not followed by `:` / `)`, abandon structured parse and
        // skip to the matching `)` for this asm (depth already open at template level).
        if !self.at(&TokenKind::Colon) && !self.at(&TokenKind::RParen) {
            let mut depth = 1i32;
            while depth > 0 && !self.at(&TokenKind::Eof) {
                if self.at(&TokenKind::LParen) {
                    depth += 1;
                } else if self.at(&TokenKind::RParen) {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        break;
                    }
                }
                self.bump();
            }
            let _ = self.eat(TokenKind::Semicolon);
            return Ok(Stmt::Asm {
                lines: Vec::new(),
                in_loads: Vec::new(),
                out_stores: Vec::new(),
                out_store_exprs: Vec::new(),
            });
        }

        // Optional : outputs : inputs : clobbers [ : goto-labels ]
        // Shared operand index space: outputs first (%0..), then inputs.
        // Kind: 0=imm, 1=reg, 2=drop. reg ops carry (regno, Option<var>, is_out).
        let mut op_kind: Vec<u8> = Vec::new();
        let mut op_imm: Vec<i64> = Vec::new();
        let mut op_reg: Vec<u8> = Vec::new();
        let mut op_var: Vec<Option<String>> = Vec::new();
        let mut op_is_out: Vec<bool> = Vec::new();
        // True when constraint contains '+' (read-write): load before asm too.
        let mut op_plus: Vec<bool> = Vec::new();
        let mut next_reg: u8 = 0;
        let mut alloc_reg = || -> u8 {
            loop {
                let r = next_reg;
                next_reg = next_reg.saturating_add(1);
                if r == 18 || r >= 29 {
                    continue;
                }
                return r;
            }
        };
        // kind: 0=imm, 1=reg, 2=drop, 3=matching (digit constraint → same reg as op N)
        // For kind 3, op_imm holds the matched operand index until resolved.
        let mut op_expr: Vec<Option<Expr>> = Vec::new();
        let mut parse_ops = |p: &mut Parser, is_out: bool| -> Result<(), String> {
            if p.at(&TokenKind::Colon) || p.at(&TokenKind::RParen) {
                return Ok(());
            }
            loop {
                p.skip_asm_operand_name();
                if let TokenKind::StringLit(cstr) = p.peek_kind().clone() {
                    p.bump();
                    p.expect(TokenKind::LParen)?;
                    if p.eat(TokenKind::RParen) {
                        op_kind.push(2);
                        op_imm.push(0);
                        op_reg.push(0);
                        op_var.push(None);
                        op_is_out.push(is_out);
                        op_expr.push(None);
                        op_plus.push(false);
                    } else {
                        let e = p.parse_assign()?;
                        p.expect(TokenKind::RParen)?;
                        // Matching constraint "0"/"1"/… → same register as that operand
                        // (kernel RELOC_HIDE: asm("" : "=r"(p) : "0"(ptr))).
                        let matching = {
                            let t = cstr.trim();
                            if t.len() == 1 && t.as_bytes()[0].is_ascii_digit() {
                                Some((t.as_bytes()[0] - b'0') as i64)
                            } else {
                                None
                            }
                        };
                        let is_imm_c = cstr.contains('i') || cstr.contains('n');
                        // x86 letter classes: r/g general; q = low-byte-capable;
                        // a/b/c/d/S/D fixed regs. Do not treat bare 'm' as reg.
                        let body = cstr.trim_start_matches(['=', '+', '&', '%', ' ']);
                        let is_reg_c = body.contains('r')
                            || body.contains('g')
                            || body.contains('q')
                            || body.contains('Q')
                            || body.starts_with('a')
                            || body.starts_with('b')
                            || body.starts_with('c')
                            || body.starts_with('d')
                            || body.starts_with('S')
                            || body.starts_with('D');
                        let is_mem_c = !is_reg_c
                            && (body.starts_with('m')
                                || body.starts_with('o')
                                || body.contains('m'));
                        let has_plus = cstr.contains('+');
                        let imm = if is_imm_c && matching.is_none() {
                            p.const_array_len(&e)
                                .or_else(|| p.const_offsetof(&e))
                                .or_else(|| p.eval_enum_const(&e))
                        } else {
                            None
                        };
                        if let Some(v) = imm {
                            op_kind.push(0);
                            op_imm.push(v);
                            op_reg.push(0);
                            op_var.push(None);
                            op_is_out.push(is_out);
                            op_expr.push(None);
                            op_plus.push(false);
                        } else if let Some(mi) = matching {
                            op_kind.push(3);
                            op_imm.push(mi);
                            op_reg.push(0); // filled after all ops known
                            op_var.push(None);
                            op_is_out.push(is_out);
                            op_expr.push(Some(e));
                            op_plus.push(has_plus);
                        } else if is_reg_c {
                            // Fixed x86 letter constraints must use the real
                            // physical register (cmpxchg requires %eax for "a").
                            let reg = match body {
                                "a" => 0u8,   // %rax
                                "b" => 19u8,  // %rbx
                                "c" => 11u8,  // %rcx
                                "d" => 3u8,   // %rdx
                                "S" => 17u8,  // %rsi
                                "D" => 1u8,   // %rdi
                                _ => alloc_reg(),
                            };
                            let var = match &e {
                                Expr::Var(n) => Some(n.clone()),
                                _ => None,
                            };
                            op_kind.push(1);
                            op_imm.push(0);
                            op_reg.push(reg);
                            op_var.push(var);
                            op_is_out.push(is_out);
                            op_expr.push(Some(e));
                            op_plus.push(has_plus);
                        } else if is_mem_c {
                            // "+m"(*lock) → address in a GP reg, template uses (xN).
                            let reg = alloc_reg();
                            op_kind.push(4);
                            op_imm.push(0);
                            op_reg.push(reg);
                            op_var.push(None);
                            op_is_out.push(is_out);
                            op_expr.push(Some(e));
                            op_plus.push(has_plus);
                        } else {
                            op_kind.push(2);
                            op_imm.push(0);
                            op_reg.push(0);
                            op_var.push(None);
                            op_is_out.push(is_out);
                            op_expr.push(None);
                            op_plus.push(false);
                        }
                    }
                    if p.eat(TokenKind::Comma) {
                        continue;
                    }
                }
                break;
            }
            Ok(())
        };

        if self.eat(TokenKind::Colon) {
            parse_ops(self, true)?;
            if self.eat(TokenKind::Colon) {
                parse_ops(self, false)?;
                if self.eat(TokenKind::Colon) {
                    while let TokenKind::StringLit(_) = self.peek_kind().clone() {
                        self.bump();
                        if !self.eat(TokenKind::Comma) {
                            break;
                        }
                    }
                    if self.eat(TokenKind::Colon) {
                        while let TokenKind::Ident(_) = self.peek_kind().clone() {
                            self.bump();
                            if !self.eat(TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                }
            }
        }
        // Soft: if still not at `)`, skip to matching close (broken ALTERNATIVE residue).
        if !self.at(&TokenKind::RParen) {
            let mut depth = 1i32;
            while depth > 0 && !self.at(&TokenKind::Eof) {
                if self.at(&TokenKind::LParen) {
                    depth += 1;
                } else if self.at(&TokenKind::RParen) {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        break;
                    }
                }
                self.bump();
            }
        } else {
            self.expect(TokenKind::RParen)?;
        }
        let _ = self.eat(TokenKind::Semicolon);

        // Resolve matching constraints to the target operand's register.
        for i in 0..op_kind.len() {
            if op_kind[i] == 3 {
                let mi = op_imm[i] as usize;
                if mi < op_kind.len() && (op_kind[mi] == 1 || op_kind[mi] == 3) {
                    // If target not yet a reg, allocate.
                    if op_kind[mi] != 1 {
                        let reg = alloc_reg();
                        op_kind[mi] = 1;
                        op_reg[mi] = reg;
                    }
                    op_reg[i] = op_reg[mi];
                    op_kind[i] = 1; // treat as reg for substitution
                } else if mi < op_kind.len() && op_kind[mi] == 1 {
                    op_reg[i] = op_reg[mi];
                    op_kind[i] = 1;
                } else {
                    // Soft: unmatched → fresh reg
                    let reg = alloc_reg();
                    op_reg[i] = reg;
                    op_kind[i] = 1;
                }
            }
        }

        let mut in_loads: Vec<(u8, Expr)> = Vec::new();
        let mut out_stores: Vec<(u8, String)> = Vec::new();
        let mut out_store_exprs: Vec<(u8, Expr)> = Vec::new();
        for i in 0..op_kind.len() {
            if op_kind[i] == 4 {
                // Memory operand: load the *address* of the lvalue into the reg.
                if let Some(e) = op_expr[i].clone() {
                    let addr = match e {
                        Expr::Unary {
                            op: UnaryOp::Deref,
                            expr,
                        } => *expr,
                        other => Expr::Unary {
                            op: UnaryOp::Addr,
                            expr: Box::new(other),
                        },
                    };
                    in_loads.push((op_reg[i], addr));
                }
                continue;
            }
            if op_kind[i] != 1 {
                continue;
            }
            if op_is_out[i] {
                if let Some(ref v) = op_var[i] {
                    out_stores.push((op_reg[i], v.clone()));
                } else if let Some(Expr::Var(n)) = &op_expr[i] {
                    out_stores.push((op_reg[i], n.clone()));
                } else if let Some(e) = op_expr[i].clone() {
                    // "=a"(*expected) — write register back through lvalue.
                    out_store_exprs.push((op_reg[i], e));
                }
                // "+r"(x) / "a"(*p) read-write: preload current value.
                if op_plus[i] || matches!(
                    op_expr[i],
                    Some(Expr::Unary {
                        op: UnaryOp::Deref,
                        ..
                    })
                ) {
                    if let Some(e) = op_expr[i].clone() {
                        in_loads.push((op_reg[i], e));
                    } else if let Some(ref v) = op_var[i] {
                        in_loads.push((op_reg[i], Expr::Var(v.clone())));
                    }
                }
            } else if let Some(e) = op_expr[i].clone() {
                // Input: evaluate expression into the register (Var, Addr, Cast…).
                in_loads.push((op_reg[i], e));
            }
        }

        // Substitute %0, %1, ... with imm or xN; %% → %
        let mut out = String::new();
        let bytes = template.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'%' {
                    out.push('%');
                    i += 2;
                    continue;
                }
                // Named operand %[foo] — leave intact so codegen filter drops the line
                // unless we later map names (not yet).
                if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                    out.push('%');
                    i += 1;
                    continue;
                }
                // %N or %wN / %xN / %cN — optional size/modifier letter then digits.
                // Critical: %w0 must become wN (ldtrb/strh), not xN.
                let mut j = i + 1;
                let mut reg_prefix = 'x'; // AArch64 default for "r"
                if j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                    let c = bytes[j] as char;
                    match c {
                        'w' | 'x' | 'b' | 'h' => {
                            reg_prefix = c;
                            j += 1;
                        }
                        'c' | 'n' | 'a' | 'l' | 'p' | 'P' => {
                            // print/modifier — skip letter, keep default width
                            j += 1;
                        }
                        _ => {
                            // Unknown letter: leave %… for filter
                            out.push('%');
                            i += 1;
                            continue;
                        }
                    }
                }
                let start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if start < j {
                    let idx: usize = std::str::from_utf8(&bytes[start..j])
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(usize::MAX);
                    if idx < op_kind.len() && op_kind[idx] == 0 {
                        out.push_str(&format!("{}", op_imm[idx]));
                    } else if idx < op_kind.len() && op_kind[idx] == 1 {
                        out.push_str(&format!("{reg_prefix}{}", op_reg[idx]));
                    } else if idx < op_kind.len() && op_kind[idx] == 4 {
                        // Memory operand → (xN); soften/rewrite turns into (%reg).
                        out.push_str(&format!("(x{})", op_reg[idx]));
                    } else {
                        // Drop unresolvable %N (codegen filters), TLBI ALL-form exception.
                        let trimmed = out.trim_end();
                        if trimmed.ends_with(',') {
                            let before_comma = trimmed[..trimmed.len() - 1].trim_end();
                            if before_comma.ends_with("vmalle1is")
                                || before_comma.ends_with("vmalle1")
                                || before_comma.ends_with("alle1")
                                || before_comma.ends_with("alle2")
                                || before_comma.ends_with("alle3")
                                || before_comma.ends_with("ialluis")
                                || before_comma.ends_with("iallu")
                            {
                                out.truncate(trimmed.len() - 1);
                            } else {
                                out.push('%');
                                out.push_str(
                                    std::str::from_utf8(&bytes[i + 1..j]).unwrap_or(""),
                                );
                            }
                        } else {
                            out.push('%');
                            out.push_str(
                                std::str::from_utf8(&bytes[i + 1..j]).unwrap_or(""),
                            );
                        }
                    }
                    i = j;
                    continue;
                }
            }
            out.push(bytes[i] as char);
            i += 1;
        }

        let lines: Vec<String> = out
            .split('\n')
            .map(|s| s.trim_end().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(Stmt::Asm {
            lines,
            in_loads,
            out_stores,
            out_store_exprs,
        })
    }

    /// Optional GNU named asm operand: `[name]` before the constraint string.
    fn skip_asm_operand_name(&mut self) {
        if self.eat(TokenKind::LBracket) {
            // [ident] or [ident_with_digits]
            if let TokenKind::Ident(_) = self.peek_kind().clone() {
                self.bump();
            }
            let _ = self.eat(TokenKind::RBracket);
        }
    }

    #[allow(dead_code)]
    fn skip_asm_operand_list(&mut self) -> Result<(), String> {
        // empty or [name] "cstr" (expr) [, ...]  (+ / = constraint prefixes inside string)
        if self.at(&TokenKind::Colon) || self.at(&TokenKind::RParen) {
            return Ok(());
        }
        loop {
            self.skip_asm_operand_name();
            if let TokenKind::StringLit(_) = self.peek_kind().clone() {
                self.bump();
                self.expect(TokenKind::LParen)?;
                if !self.eat(TokenKind::RParen) {
                    let _ = self.parse_assign()?;
                    self.expect(TokenKind::RParen)?;
                }
                if self.eat(TokenKind::Comma) {
                    continue;
                }
            }
            break;
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn parse_asm_input_immediates(&mut self) -> Result<Vec<Option<i64>>, String> {
        let mut vals = Vec::new();
        if self.at(&TokenKind::Colon) || self.at(&TokenKind::RParen) {
            return Ok(vals);
        }
        loop {
            self.skip_asm_operand_name();
            if let TokenKind::StringLit(cstr) = self.peek_kind().clone() {
                self.bump();
                self.expect(TokenKind::LParen)?;
                if self.eat(TokenKind::RParen) {
                    vals.push(None);
                } else {
                    let e = self.parse_assign()?;
                    self.expect(TokenKind::RParen)?;
                    // "i" / "n" / "ri" etc. — evaluate as constant when possible.
                    // Kernel headers also use "i"(var) in non-DEFINE asm; fall back to 0
                    // so we can still parse the TU. kbuild DEFINE values must still fold.
                    if cstr.contains('i') || cstr.contains('n') {
                        if let Some(v) = self.const_array_len(&e) {
                            vals.push(Some(v));
                        } else if let Some(v) = self.const_offsetof(&e) {
                            vals.push(Some(v));
                        } else if let Some(v) = self.eval_enum_const(&e) {
                            vals.push(Some(v));
                        } else {
                            vals.push(None);
                        }
                    } else {
                        // non-immediate (e.g. "r"): push None so %N trims trailing comma
                        vals.push(None);
                    }
                }
                if self.eat(TokenKind::Comma) {
                    continue;
                }
            }
            break;
        }
        Ok(vals)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        // block-scope typedef: typedef enum { e } h;
        if self.at(&TokenKind::Typedef) {
            let item = self.parse_typedef()?;
            // typedef doesn't produce a runtime stmt; register via empty (parser
            // already recorded typedef into self.typedefs via parse_typedef).
            let _ = item;
            return Ok(Stmt::Empty);
        }
        // C11 `_Static_assert` / C23 `static_assert` as statement
        if matches!(
            self.peek_kind(),
            TokenKind::Ident(s) if s == "_Static_assert" || s == "static_assert"
        ) {
            self.bump();
            if self.at(&TokenKind::LParen) {
                self.skip_balanced_parens()?;
            }
            let _ = self.eat(TokenKind::Semicolon);
            return Ok(Stmt::Empty);
        }
        // GCC local labels: `__label__ name [, name2...];`
        if matches!(self.peek_kind(), TokenKind::Ident(s) if s == "__label__") {
            self.bump();
            loop {
                if let TokenKind::Ident(_) = self.peek_kind().clone() {
                    self.bump();
                } else {
                    break;
                }
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            let _ = self.eat(TokenKind::Semicolon);
            return Ok(Stmt::Empty);
        }
        // label: stmt
        if let TokenKind::Ident(name) = self.peek_kind().clone() {
            if self.toks.get(self.i + 1).map(|t| &t.kind) == Some(&TokenKind::Colon) {
                self.bump();
                self.bump();
                let inner = self.parse_stmt()?;
                return Ok(Stmt::Label(name, Box::new(inner)));
            }
        }
        if self.eat(TokenKind::LBrace) {
            self.push_scope();
            let mut stmts = Vec::new();
            while !self.at(&TokenKind::RBrace) {
                stmts.push(self.parse_stmt()?);
            }
            self.expect(TokenKind::RBrace)?;
            self.pop_scope();
            return Ok(Stmt::Block(stmts));
        }
        if self.eat(TokenKind::Semicolon) {
            return Ok(Stmt::Empty);
        }
        // GNU asm / __asm__ / __asm  [volatile] ( "template" [ : outs [ : ins [ : clobbers ] ] ] );
        // Enough for kernel kbuild DEFINE(sym,val) → .ascii "->sym val ..."
        if matches!(
            self.peek_kind(),
            TokenKind::Ident(s) if s == "asm" || s == "__asm__" || s == "__asm"
        ) {
            return self.parse_asm_stmt();
        }
        if self.eat(TokenKind::Return) {
            if self.eat(TokenKind::Semicolon) {
                return Ok(Stmt::Return(None));
            }
            let e = self.parse_expr()?;
            self.expect(TokenKind::Semicolon)?;
            return Ok(Stmt::Return(Some(e)));
        }
        if self.eat(TokenKind::Break) {
            self.expect(TokenKind::Semicolon)?;
            return Ok(Stmt::Break);
        }
        if self.eat(TokenKind::Continue) {
            self.expect(TokenKind::Semicolon)?;
            return Ok(Stmt::Continue);
        }
        if self.eat(TokenKind::Goto) {
            // GCC computed goto: `goto *expr;`
            if self.eat(TokenKind::Star) {
                let e = self.parse_expr()?;
                self.expect(TokenKind::Semicolon)?;
                return Ok(Stmt::GotoIndirect(e));
            }
            let t = self.expect(TokenKind::Ident(String::new()))?;
            let name = match t.kind {
                TokenKind::Ident(s) => s,
                _ => unreachable!(),
            };
            self.expect(TokenKind::Semicolon)?;
            return Ok(Stmt::Goto(name));
        }
        if self.eat(TokenKind::If) {
            self.expect(TokenKind::LParen)?;
            let cond = self.parse_expr()?;
            self.expect(TokenKind::RParen)?;
            let then_b = Box::new(self.parse_stmt()?);
            let else_b = if self.eat(TokenKind::Else) {
                Some(Box::new(self.parse_stmt()?))
            } else {
                None
            };
            return Ok(Stmt::If {
                cond,
                then_b,
                else_b,
            });
        }
        if self.eat(TokenKind::While) {
            self.expect(TokenKind::LParen)?;
            let cond = self.parse_expr()?;
            self.expect(TokenKind::RParen)?;
            let body = Box::new(self.parse_stmt()?);
            return Ok(Stmt::While { cond, body });
        }
        if self.eat(TokenKind::Do) {
            let body = Box::new(self.parse_stmt()?);
            self.expect(TokenKind::While)?;
            self.expect(TokenKind::LParen)?;
            let cond = self.parse_expr()?;
            self.expect(TokenKind::RParen)?;
            self.expect(TokenKind::Semicolon)?;
            return Ok(Stmt::DoWhile { body, cond });
        }
        if self.eat(TokenKind::Switch) {
            self.expect(TokenKind::LParen)?;
            let cond = self.parse_expr()?;
            self.expect(TokenKind::RParen)?;
            let body = Box::new(self.parse_stmt()?);
            return Ok(Stmt::Switch { cond, body });
        }
        if self.eat(TokenKind::Case) {
            let value = self.parse_expr()?;
            // GNU case ranges: `case '0' ... '7':` — parse high end; codegen uses low.
            if self.eat(TokenKind::Ellipsis) {
                let _hi = self.parse_expr()?;
            }
            self.expect(TokenKind::Colon)?;
            // case can have empty body or fallthrough chain
            let body = if self.at(&TokenKind::Case)
                || self.at(&TokenKind::Default)
                || self.at(&TokenKind::RBrace)
            {
                Box::new(Stmt::Empty)
            } else {
                Box::new(self.parse_stmt()?)
            };
            return Ok(Stmt::Case { value, body });
        }
        if self.eat(TokenKind::Default) {
            self.expect(TokenKind::Colon)?;
            let body = if self.at(&TokenKind::RBrace) {
                Box::new(Stmt::Empty)
            } else {
                Box::new(self.parse_stmt()?)
            };
            return Ok(Stmt::Default(body));
        }
        if self.eat(TokenKind::For) {
            self.expect(TokenKind::LParen)?;
            let init = if self.eat(TokenKind::Semicolon) {
                None
            } else if self.is_typename() {
                let d = self.parse_local_decl()?;
                Some(Box::new(d))
            } else {
                let e = self.parse_expr()?;
                self.expect(TokenKind::Semicolon)?;
                Some(Box::new(Stmt::Expr(e)))
            };
            let cond = if self.eat(TokenKind::Semicolon) {
                None
            } else {
                let e = self.parse_expr()?;
                self.expect(TokenKind::Semicolon)?;
                Some(e)
            };
            let step = if self.at(&TokenKind::RParen) {
                None
            } else {
                Some(self.parse_expr()?)
            };
            self.expect(TokenKind::RParen)?;
            let body = Box::new(self.parse_stmt()?);
            return Ok(Stmt::For {
                init,
                cond,
                step,
                body,
            });
        }
        if self.is_typename() {
            return self.parse_local_decl();
        }
        let e = self.parse_expr()?;
        self.expect(TokenKind::Semicolon)?;
        Ok(Stmt::Expr(e))
    }

    fn parse_local_decl(&mut self) -> Result<Stmt, String> {
        self.skip_trailing_gnu_attrs();
        // Detect storage class before type specifier (static/extern are eaten
        // inside parse_type_specifier, so peek first).
        // Block-scope `extern T name;` must NOT become a stack local — otherwise
        // `&sqlite3_search_count` in Tcl_LinkVar points at a frame slot and
        // SQLite index search-count tests always see 0.
        let is_static = self.at(&TokenKind::Static);
        let is_extern = self.at(&TokenKind::Extern);
        let mut sec_attr: Option<String> = None;
        if let TokenKind::Section(sec) = self.peek_kind().clone() {
            sec_attr = Some(sec);
            self.bump();
        }
        self.skip_trailing_gnu_attrs();
        let base = self.parse_type_specifier()?;
        if let TokenKind::Section(sec) = self.peek_kind().clone() {
            sec_attr = Some(sec);
            self.bump();
        }
        // multi-decl: int x, *p, **pp;
        let mut decls = Vec::new();
        loop {
            let (name, ty, func_params) = self.parse_declarator(base.clone())?;
            sec_attr = self.last_section.take().or(sec_attr);
            if let TokenKind::Section(sec) = self.peek_kind().clone() {
                sec_attr = Some(sec);
                self.bump();
            }
            // Block-scope function prototype: `extern const char *f(Tcl_Interp*);`
            // parse_declarator already consumed `(params)` into func_params.
            // Must NOT allocate a stack local (would make calls `blr` a null slot —
            // SQLite tclsqlite main → TCLSH_INIT_PROC SEGV).
            if func_params.is_some() {
                self.expect(TokenKind::Semicolon)?;
                return Ok(Stmt::Empty);
            }
            // Legacy skip: name(...) when declarator did not absorb params.
            if self.eat(TokenKind::LParen) {
                let mut depth = 1;
                while depth > 0 && !self.at(&TokenKind::Eof) {
                    if self.at(&TokenKind::LParen) {
                        depth += 1;
                    } else if self.at(&TokenKind::RParen) {
                        depth -= 1;
                    }
                    if depth == 0 {
                        break;
                    }
                    self.bump();
                }
                self.expect(TokenKind::RParen)?;
                self.expect(TokenKind::Semicolon)?;
                return Ok(Stmt::Empty);
            }
            let init = if self.eat(TokenKind::Assign) {
                Some(self.parse_initializer()?)
            } else {
                None
            };
            let ty = self.infer_array_size(ty, &init);
            // Record type for later sizeof(local) / sizeof(p->field) bounds.
            if !is_extern {
                self.insert_local_type(&name, ty.clone());
            }
            decls.push(VarDecl {
                name,
                ty,
                init,
                is_static,
                is_extern,
                is_weak: false,
                section: sec_attr.clone(),
            });
            if self.eat(TokenKind::Comma) {
                continue;
            }
            break;
        }
        self.expect(TokenKind::Semicolon)?;
        if decls.len() == 1 {
            Ok(Stmt::Decl(decls.remove(0)))
        } else {
            // DeclGroup — NOT Stmt::Block: Block enters/exits a nested scope and
            // would drop multi-decl locals (`u64 cycles, last, ns`) before the
            // rest of the function, turning them into soft globals (vdso ADRP).
            Ok(Stmt::DeclGroup(decls))
        }
    }

    // ---- expressions (Pratt / precedence climbing) ----
    fn parse_expr(&mut self) -> Result<Expr, String> {
        // Comma operator (lowest precedence)
        let mut left = self.parse_assign()?;
        while self.eat(TokenKind::Comma) {
            let right = self.parse_assign()?;
            left = Expr::Binary {
                op: BinOp::Comma,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_assign(&mut self) -> Result<Expr, String> {
        let left = self.parse_cond()?;
        if self.eat(TokenKind::Assign) {
            let right = self.parse_assign()?;
            return Ok(Expr::Assign {
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        let compound = match self.peek_kind() {
            TokenKind::PlusEq => Some(BinOp::Add),
            TokenKind::MinusEq => Some(BinOp::Sub),
            TokenKind::StarEq => Some(BinOp::Mul),
            TokenKind::SlashEq => Some(BinOp::Div),
            TokenKind::PercentEq => Some(BinOp::Mod),
            TokenKind::AndEq => Some(BinOp::BitAnd),
            TokenKind::OrEq => Some(BinOp::BitOr),
            TokenKind::XorEq => Some(BinOp::BitXor),
            TokenKind::ShlEq => Some(BinOp::Shl),
            TokenKind::ShrEq => Some(BinOp::Shr),
            _ => None,
        };
        if let Some(op) = compound {
            self.bump();
            let right = self.parse_assign()?;
            return Ok(Expr::CompoundAssign {
                op,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    fn parse_cond(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_or()?;
        if self.eat(TokenKind::Question) {
            // GNU extension: `x ?: y` ≡ `x ? x : y` (omit middle operand).
            let t = if self.at(&TokenKind::Colon) {
                e.clone()
            } else {
                self.parse_expr()?
            };
            self.expect(TokenKind::Colon)?;
            let f = self.parse_cond()?;
            e = Expr::Cond {
                cond: Box::new(e),
                then_e: Box::new(t),
                else_e: Box::new(f),
            };
        }
        Ok(e)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.eat(TokenKind::OrOr) {
            let right = self.parse_and()?;
            left = Expr::Binary {
                op: BinOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitor()?;
        while self.eat(TokenKind::AndAnd) {
            let right = self.parse_bitor()?;
            left = Expr::Binary {
                op: BinOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bitor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitxor()?;
        while self.eat(TokenKind::Pipe) {
            let right = self.parse_bitxor()?;
            left = Expr::Binary {
                op: BinOp::BitOr,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bitxor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitand()?;
        while self.eat(TokenKind::Caret) {
            let right = self.parse_bitand()?;
            left = Expr::Binary {
                op: BinOp::BitXor,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_bitand(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_eq()?;
        while self.eat(TokenKind::Amp) {
            let right = self.parse_eq()?;
            left = Expr::Binary {
                op: BinOp::BitAnd,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_eq(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_rel()?;
        loop {
            if self.eat(TokenKind::Eq) {
                let right = self.parse_rel()?;
                left = Expr::Binary {
                    op: BinOp::Eq,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(TokenKind::Ne) {
                let right = self.parse_rel()?;
                left = Expr::Binary {
                    op: BinOp::Ne,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_rel(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_shift()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Lt => Some(BinOp::Lt),
                TokenKind::Gt => Some(BinOp::Gt),
                TokenKind::Le => Some(BinOp::Le),
                TokenKind::Ge => Some(BinOp::Ge),
                _ => None,
            };
            if let Some(op) = op {
                self.bump();
                let right = self.parse_shift()?;
                left = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_add()?;
        loop {
            if self.eat(TokenKind::Shl) {
                let right = self.parse_add()?;
                left = Expr::Binary {
                    op: BinOp::Shl,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(TokenKind::Shr) {
                let right = self.parse_add()?;
                left = Expr::Binary {
                    op: BinOp::Shr,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul()?;
        loop {
            if self.eat(TokenKind::Plus) {
                let right = self.parse_mul()?;
                left = Expr::Binary {
                    op: BinOp::Add,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(TokenKind::Minus) {
                let right = self.parse_mul()?;
                left = Expr::Binary {
                    op: BinOp::Sub,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            if self.eat(TokenKind::Star) {
                let right = self.parse_unary()?;
                left = Expr::Binary {
                    op: BinOp::Mul,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(TokenKind::Slash) {
                let right = self.parse_unary()?;
                left = Expr::Binary {
                    op: BinOp::Div,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else if self.eat(TokenKind::Percent) {
                let right = self.parse_unary()?;
                left = Expr::Binary {
                    op: BinOp::Mod,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        // GCC address-of-label: `&&label` (not logical-and). Used as
        // `({ __label__ L; L: (unsigned long)&&L; })` in kernel softirq headers.
        if self.at(&TokenKind::AndAnd) {
            if let Some(TokenKind::Ident(_)) = self.toks.get(self.i + 1).map(|t| &t.kind) {
                self.bump(); // &&
                if let TokenKind::Ident(lab) = self.peek_kind().clone() {
                    self.bump();
                    return Ok(Expr::AddrOfLabel(lab));
                }
            }
        }
        if self.eat(TokenKind::PlusPlus) {
            return Ok(Expr::PreInc(Box::new(self.parse_unary()?)));
        }
        if self.eat(TokenKind::MinusMinus) {
            return Ok(Expr::PreDec(Box::new(self.parse_unary()?)));
        }
        if self.eat(TokenKind::Plus) {
            return self.parse_unary();
        }
        if self.eat(TokenKind::Minus) {
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.eat(TokenKind::Bang) {
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.eat(TokenKind::Tilde) {
            return Ok(Expr::Unary {
                op: UnaryOp::BitNot,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.eat(TokenKind::Star) {
            return Ok(Expr::Unary {
                op: UnaryOp::Deref,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.eat(TokenKind::Amp) {
            return Ok(Expr::Unary {
                op: UnaryOp::Addr,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.eat(TokenKind::Sizeof) {
            if self.eat(TokenKind::LParen) {
                if self.is_typename() {
                    let ty = self.parse_type_name()?;
                    self.expect(TokenKind::RParen)?;
                    return Ok(Expr::SizeofType(ty));
                }
                let e = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                return Ok(Expr::SizeofExpr(Box::new(e)));
            }
            return Ok(Expr::SizeofExpr(Box::new(self.parse_unary()?)));
        }
        // GCC/C11 alignof: `__alignof__(T)` / `__alignof(T)` / `alignof(T)`.
        // Soft: return alignment as int; use type size as a crude stand-in for
        // scalar types (good enough for kernel header comparisons).
        if let TokenKind::Ident(name) = self.peek_kind().clone() {
            if name == "__alignof__" || name == "__alignof" || name == "alignof" {
                self.bump();
                self.expect(TokenKind::LParen)?;
                if self.is_typename() {
                    let ty = self.parse_type_name()?;
                    self.expect(TokenKind::RParen)?;
                    let al = self.const_type_align(&ty);
                    return Ok(Expr::Int(al));
                }
                let _e = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                return Ok(Expr::Int(8));
            }
        }
        // Kernel/Linux: __builtin_offsetof(struct T, field) → constant int.
        // TYPE may be `struct name` / `union name` / typedef name.
        // Also: offsetof(T, field[n]) / nested paths (pt_regs.regs[2], etc.).
        if let TokenKind::Ident(name) = self.peek_kind().clone() {
            if name == "__builtin_offsetof" {
                self.bump();
                self.expect(TokenKind::LParen)?;
                let ty = self.parse_type_name()?;
                self.expect(TokenKind::Comma)?;
                // Support nested paths: `tss.x86_tss.sp1` and `regs[2]`
                // Each segment: (field_name, array_indices).
                let mut path: Vec<(String, Vec<i64>)> = Vec::new();
                loop {
                    if let TokenKind::Ident(f) = self.peek_kind().clone() {
                        self.bump();
                        let mut indices: Vec<i64> = Vec::new();
                        // Optional array indices: field[n] / field[n][m]
                        while self.eat(TokenKind::LBracket) {
                            if !self.at(&TokenKind::RBracket) {
                                let e = self.parse_expr()?;
                                let idx = self.const_array_len(&e).unwrap_or(0);
                                indices.push(idx);
                            } else {
                                indices.push(0);
                            }
                            self.expect(TokenKind::RBracket)?;
                        }
                        path.push((f, indices));
                    } else {
                        return Err(format!(
                            "offsetof member name at {}:{}",
                            self.peek().line,
                            self.peek().col
                        ));
                    }
                    if !self.eat(TokenKind::Dot) && !self.eat(TokenKind::Arrow) {
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                // Soft-fallback 0 when layout is incomplete (incomplete types mid-TU).
                let off = self.const_offsetof_type_path(&ty, &path).unwrap_or(0);
                return Ok(Expr::Int(off));
            }
        }
        // `__builtin_va_arg(ap, type)` — second operand is a type-name, not expr.
        if let TokenKind::Ident(name) = self.peek_kind().clone() {
            if name == "__builtin_va_arg" || name == "va_arg" {
                self.bump();
                self.expect(TokenKind::LParen)?;
                let ap = self.parse_assign()?;
                self.expect(TokenKind::Comma)?;
                let ty = self.parse_type_name()?;
                self.expect(TokenKind::RParen)?;
                // Float/double variadic args on aarch64 Linux live in d0..d7 (AAPCS64),
                // not the next GP slot. System-gcc callers pass them there; our soft
                // GP-only cursor would read 0. Use a dedicated FP walker for those.
                let is_fp = matches!(ty, Type::Float | Type::Double);
                let helper = if is_fp {
                    "__acc_va_arg_fp"
                } else {
                    "__acc_va_arg"
                };
                // Lower to (*(T*)helper(&ap)) soft form.
                return Ok(Expr::Unary {
                    op: UnaryOp::Deref,
                    expr: Box::new(Expr::Cast {
                        ty: Type::Ptr(Box::new(ty)),
                        expr: Box::new(Expr::Call {
                            name: helper.into(),
                            args: vec![Expr::Unary {
                                op: UnaryOp::Addr,
                                expr: Box::new(ap),
                            }],
                        }),
                    }),
                });
            }
        }
        // cast: (type) expr  OR compound literal (type){ init }
        if self.at(&TokenKind::LParen) && self.is_cast_start() {
            self.bump();
            let ty = self.parse_type_name()?;
            self.expect(TokenKind::RParen)?;
            if self.at(&TokenKind::LBrace) {
                let init = self.parse_initializer()?;
                return Ok(Expr::Cast {
                    ty,
                    expr: Box::new(init),
                });
            }
            let e = self.parse_unary()?;
            return Ok(Expr::Cast {
                ty,
                expr: Box::new(e),
            });
        }
        self.parse_postfix()
    }

    /// Look-ahead: skip a cast type-name starting at token index `j`.
    /// Returns the index of the token after the type (usually `)` or `[` of
    /// abstract array), or `None` if tokens do not form a type.
    ///
    /// Critical: `struct Tag` / `union Tag` / `enum Tag` must consume the
    /// optional tag identifier (and optional `{...}` body). The previous
    /// one-token skip left `Tag` unconsumed, so `(struct sdshdr8 *)s` was
    /// not recognized as a cast — Redis `sdslen` / SDS_HDR then silently
    /// failed top-level recovery and never emitted.
    fn skip_cast_type_tokens(&self, mut j: usize) -> Option<usize> {
        // optional leading quals / signedness. Stars belong in the abstract
        // declarator loop below so bare `(unsigned *)` works: consume
        // `unsigned` as the type, then `*` as declarator.
        // Bare `(unsigned)` / `(signed)` must also be accepted — SQLite uses
        // `((unsigned)p[0]<<24)` heavily; missing this soft-skipped whole
        // functions (Get4byte, VdbeExec, …) via top-level recovery.
        let mut saw_sign = false;
        while matches!(
            self.toks.get(j).map(|t| &t.kind),
            Some(
                TokenKind::Const
                    | TokenKind::Volatile
                    | TokenKind::Restrict
                    | TokenKind::Unsigned
                    | TokenKind::Signed
            )
        ) {
            if matches!(
                self.toks.get(j).map(|t| &t.kind),
                Some(TokenKind::Unsigned | TokenKind::Signed)
            ) {
                saw_sign = true;
            }
            j += 1;
        }
        match self.toks.get(j).map(|t| &t.kind) {
            Some(TokenKind::Struct | TokenKind::Union | TokenKind::Enum) => {
                j += 1;
                // optional tag name
                if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Ident(_))) {
                    j += 1;
                }
                // optional inline definition body: struct { ... }
                if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::LBrace)) {
                    j = self.skip_balanced_braces_offset(j);
                }
            }
            Some(
                TokenKind::Int
                    | TokenKind::Void
                    | TokenKind::Char
                    | TokenKind::Float
                    | TokenKind::Double,
            ) => {
                j += 1;
            }
            Some(TokenKind::Long | TokenKind::Short) => {
                // long / short / long long / long int / short int / long double
                j += 1;
                while matches!(
                    self.toks.get(j).map(|t| &t.kind),
                    Some(
                        TokenKind::Long
                            | TokenKind::Short
                            | TokenKind::Int
                            | TokenKind::Unsigned
                            | TokenKind::Signed
                            | TokenKind::Double
                            | TokenKind::Float
                            | TokenKind::Char
                    )
                ) {
                    j += 1;
                }
            }
            Some(TokenKind::Ident(s)) => {
                if Self::is_typeof_kw(s) {
                    j += 1;
                    if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::LParen)) {
                        j = self.skip_balanced_parens_offset(j);
                    }
                } else if self.typedefs.iter().any(|t| t == s) {
                    j += 1;
                } else {
                    return None;
                }
            }
            // bare `unsigned` / `signed` / `const unsigned` → unsigned int / int
            _ if saw_sign => {}
            _ => return None,
        }
        // abstract declarator: * / quals / [] / (*)(params)
        loop {
            match self.toks.get(j).map(|t| &t.kind) {
                Some(
                    TokenKind::Star
                        | TokenKind::Const
                        | TokenKind::Volatile
                        | TokenKind::Restrict,
                ) => j += 1,
                Some(TokenKind::LBracket) => {
                    // skip [ ... ] (balanced by depth of brackets)
                    let mut depth = 1i32;
                    j += 1;
                    while depth > 0 && j < self.toks.len() {
                        match self.toks[j].kind {
                            TokenKind::LBracket => depth += 1,
                            TokenKind::RBracket => {
                                depth -= 1;
                                j += 1;
                                if depth == 0 {
                                    break;
                                }
                                continue;
                            }
                            _ => {}
                        }
                        if depth > 0 {
                            j += 1;
                        }
                    }
                }
                Some(TokenKind::LParen) => {
                    // function-pointer abstract: (*)(int) etc.
                    j = self.skip_balanced_parens_offset(j);
                }
                _ => break,
            }
        }
        Some(j)
    }

    fn is_cast_start(&self) -> bool {
        let Some(j) = self.skip_cast_type_tokens(self.i + 1) else {
            return false;
        };
        // After `(type)`, a cast needs a unary operand. `(quicklist)->x` and
        // `__f((quicklist), y)` are parenthesized expressions (Redis quicklist
        // shadows the typedef name with a parameter of the same name).
        if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::RParen)) {
            if !Self::token_can_start_unary(self.toks.get(j + 1).map(|t| &t.kind)) {
                return false;
            }
            return true;
        }
        // Abstract array form inside cast type ends at `[` already consumed by
        // skip; remaining must be `)`.
        matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::RParen))
    }

    /// Tokens that may start a cast operand / unary expression (or compound lit `{`).
    fn token_can_start_unary(k: Option<&TokenKind>) -> bool {
        matches!(
            k,
            Some(
                TokenKind::Ident(_)
                    | TokenKind::IntLit(_)
                    | TokenKind::FloatLit(_)
                    | TokenKind::CharLit(_)
                    | TokenKind::StringLit(_)
                    | TokenKind::LParen
                    | TokenKind::LBrace // compound literal (type){...}
                    | TokenKind::Star
                    | TokenKind::Amp
                    | TokenKind::Plus
                    | TokenKind::Minus
                    | TokenKind::Bang
                    | TokenKind::Tilde
                    | TokenKind::PlusPlus
                    | TokenKind::MinusMinus
                    | TokenKind::Sizeof
            )
        )
    }

    /// Like `is_cast_start`, but called after `(` was already consumed (primary).
    fn is_cast_after_lparen(&self) -> bool {
        let Some(j) = self.skip_cast_type_tokens(self.i) else {
            return false;
        };
        if !matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::RParen)) {
            return false;
        }
        if !Self::token_can_start_unary(self.toks.get(j + 1).map(|t| &t.kind)) {
            return false;
        }
        true
    }

    fn parse_type_name(&mut self) -> Result<Type, String> {
        let base = self.parse_type_specifier()?;
        // Abstract declarator: * / (*)(params) / [] 
        let (_, ty) = self.parse_abstract_declarator(base)?;
        Ok(ty)
    }

    /// Abstract declarator used in casts: (int *), (void (*)(void)), (int [4])
    fn parse_abstract_declarator(&mut self, base: Type) -> Result<(String, Type), String> {
        let mut ty = base;
        while self.eat(TokenKind::Star) {
            loop {
                match self.peek_kind().clone() {
                    TokenKind::Const | TokenKind::Volatile | TokenKind::Restrict => {
                        self.bump();
                    }
                    TokenKind::Ident(s)
                        if matches!(
                            s.as_str(),
                            "restrict" | "__restrict" | "__restrict__" | "pg_restrict"
                        ) =>
                    {
                        self.bump();
                    }
                    _ => break,
                }
            }
            ty = Type::Ptr(Box::new(ty));
        }
        if self.eat(TokenKind::LParen) {
            // Could be nested abstract (*)... or function params of bare type
            // Distinguish: if next is * or ( or [, nested abstract; else params.
            if matches!(
                self.peek_kind(),
                TokenKind::Star | TokenKind::LParen | TokenKind::LBracket | TokenKind::RParen
            ) && !self.is_typename()
            {
                let (n, inner) = self.parse_abstract_declarator(ty)?;
                self.expect(TokenKind::RParen)?;
                // trailing () or [] on nested
                let mut ty = inner;
                while self.eat(TokenKind::LParen) {
                    // skip params
                    let mut depth = 1;
                    while depth > 0 && !self.at(&TokenKind::Eof) {
                        if self.at(&TokenKind::LParen) {
                            depth += 1;
                        } else if self.at(&TokenKind::RParen) {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        self.bump();
                    }
                    self.expect(TokenKind::RParen)?;
                    ty = Type::Ptr(Box::new(ty)); // function type ≈ pointer
                }
                while self.eat(TokenKind::LBracket) {
                    let nsz = if self.at(&TokenKind::RBracket) {
                        0
                    } else if self.eat(TokenKind::Star) {
                        0
                    } else {
                        let e = self.parse_expr()?;
                        self.const_array_len(&e).unwrap_or(0)
                    };
                    self.expect(TokenKind::RBracket)?;
                    ty = Type::Array(Box::new(ty), nsz);
                }
                return Ok((n, ty));
            } else {
                // function params after type: int (int) — rare; skip to )
                let mut depth = 1;
                while depth > 0 && !self.at(&TokenKind::Eof) {
                    if self.at(&TokenKind::LParen) {
                        depth += 1;
                    } else if self.at(&TokenKind::RParen) {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    self.bump();
                }
                self.expect(TokenKind::RParen)?;
                ty = Type::Ptr(Box::new(ty));
            }
        }
        while self.eat(TokenKind::LBracket) {
            // Abstract array bound may be a const expr: `(T (*)[INIT_FORKNUM + 1])`
            let nsz = if self.at(&TokenKind::RBracket) {
                0
            } else if self.eat(TokenKind::Star) {
                0
            } else {
                let e = self.parse_expr()?;
                self.const_array_len(&e).unwrap_or(0)
            };
            self.expect(TokenKind::RBracket)?;
            ty = Type::Array(Box::new(ty), nsz);
        }
        Ok((String::new(), ty))
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.parse_primary()?;
        loop {
            if self.eat(TokenKind::LBracket) {
                let idx = self.parse_expr()?;
                self.expect(TokenKind::RBracket)?;
                e = Expr::Index {
                    base: Box::new(e),
                    index: Box::new(idx),
                };
            } else if self.eat(TokenKind::LParen) {
                // call: Var name, or call through expression (function pointer)
                // Args use parse_assign (not parse_expr) so commas separate arguments
                // instead of forming the comma operator into a single arg.
                let mut args = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    loop {
                        // C allows trailing commas: `f(a,)` (macro residue in kernel).
                        if self.at(&TokenKind::RParen) {
                            break;
                        }
                        if self.is_typename() {
                            let _ty = self.parse_type_name()?;
                            args.push(Expr::Int(0));
                        } else {
                            args.push(self.parse_assign()?);
                        }
                        if self.eat(TokenKind::Comma) {
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                e = match e {
                    Expr::Var(n) => Expr::Call { name: n, args },
                    other => {
                        // Encode indirect call as Call with empty name + special:
                        // use Call { name: "", args } is bad. Use Unary Deref pattern:
                        // Represent as Call { name: "__indirect__", args: [fnexpr, ...] }
                        let mut a = vec![other];
                        a.extend(args);
                        Expr::Call {
                            name: "__indirect__".into(),
                            args: a,
                        }
                    }
                };
            } else if self.eat(TokenKind::Dot) {
                let field = self.parse_field_name()?;
                e = Expr::Member {
                    base: Box::new(e),
                    field,
                    arrow: false,
                };
            } else if self.eat(TokenKind::Arrow) {
                let field = self.parse_field_name()?;
                e = Expr::Member {
                    base: Box::new(e),
                    field,
                    arrow: true,
                };
            } else if self.eat(TokenKind::PlusPlus) {
                e = Expr::PostInc(Box::new(e));
            } else if self.eat(TokenKind::MinusMinus) {
                e = Expr::PostDec(Box::new(e));
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek_kind().clone() {
            TokenKind::IntLit(n) => {
                self.bump();
                let ty = Self::int_lit_type(n);
                if ty == Type::Int {
                    Ok(Expr::Int(n))
                } else {
                    Ok(Expr::Cast {
                        ty,
                        expr: Box::new(Expr::Int(n)),
                    })
                }
            }
            TokenKind::FloatLit(f) => {
                self.bump();
                Ok(Expr::Float(f))
            }
            TokenKind::CharLit(n) => {
                self.bump();
                Ok(Expr::Char(n))
            }
            TokenKind::StringLit(s) => {
                self.bump();
                Ok(Expr::String(s))
            }
            TokenKind::Ident(name) if name == "__builtin_types_compatible_p" => {
                // GCC: __builtin_types_compatible_p(type1, type2) — both args are types.
                self.bump();
                self.expect(TokenKind::LParen)?;
                let _t1 = self.parse_type_name()?;
                self.expect(TokenKind::Comma)?;
                let _t2 = self.parse_type_name()?;
                self.expect(TokenKind::RParen)?;
                // Soft: always 0 (not compatible) unless we later compare layouts.
                Ok(Expr::Int(0))
            }
            TokenKind::Ident(name) if name == "__builtin_constant_p" => {
                // Keep as Call so const_array_len can return 1 for foldable args
                // (order_base_2 / ilog2 / kbuild DEFINE need the constant branch).
                self.bump();
                self.expect(TokenKind::LParen)?;
                let e = self.parse_assign()?;
                self.expect(TokenKind::RParen)?;
                Ok(Expr::Call {
                    name: "__builtin_constant_p".into(),
                    args: vec![e],
                })
            }
            TokenKind::Ident(name)
                if name == "__builtin_clzll"
                    || name == "__builtin_clzl"
                    || name == "__builtin_clz"
                    || name == "__builtin_ctzll"
                    || name == "__builtin_ctzl"
                    || name == "__builtin_ctz" =>
            {
                let fname = name.clone();
                self.bump();
                self.expect(TokenKind::LParen)?;
                let e = self.parse_assign()?;
                self.expect(TokenKind::RParen)?;
                // Fold immediately when possible so enum/DEFINE "i" see Int.
                if let Some(v) = self.const_array_len(&Expr::Call {
                    name: fname.clone(),
                    args: vec![e.clone()],
                }) {
                    Ok(Expr::Int(v))
                } else {
                    Ok(Expr::Call {
                        name: fname,
                        args: vec![e],
                    })
                }
            }
            TokenKind::Ident(name) if name == "__builtin_expect" => {
                self.bump();
                self.expect(TokenKind::LParen)?;
                let e = self.parse_assign()?;
                if self.eat(TokenKind::Comma) {
                    let _ = self.parse_assign()?;
                }
                self.expect(TokenKind::RParen)?;
                Ok(e)
            }
            TokenKind::Ident(name) if name == "_Generic" => {
                // C11 _Generic(controlling-expr, type: expr, ..., default: expr)
                self.bump();
                self.expect(TokenKind::LParen)?;
                let ctrl = self.parse_assign()?;
                let raw_ty = self.const_expr_type(&ctrl).unwrap_or(Type::Int);
                let ctrl_ty = self.strip_qualifiers_and_decay(raw_ty);
                self.expect(TokenKind::Comma)?;
                let mut matched_res: Option<Expr> = None;
                let mut default_res: Option<Expr> = None;
                loop {
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    if self.eat(TokenKind::Default) {
                        self.expect(TokenKind::Colon)?;
                        let val = self.parse_assign()?;
                        if default_res.is_none() {
                            default_res = Some(val);
                        }
                    } else if self.is_typename() {
                        let parsed_assoc = self.parse_type_name()?;
                        let assoc_ty = self.strip_qualifiers_and_decay(parsed_assoc);
                        self.expect(TokenKind::Colon)?;
                        let val = self.parse_assign()?;
                        if self.types_compatible(&ctrl_ty, &assoc_ty) && matched_res.is_none() {
                            matched_res = Some(val);
                        }
                    } else {
                        return Err(format!(
                            "expected type or default in _Generic at {}:{}",
                            self.peek().line,
                            self.peek().col
                        ));
                    }
                    if !self.eat(TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                Ok(matched_res.or(default_res).unwrap_or(Expr::Int(0)))
            }
            TokenKind::Ident(name) => {
                self.bump();
                Ok(Expr::Var(name))
            }
            TokenKind::LParen => {
                self.bump();
                // GNU statement expression: ({ stmts; expr; })
                // Kernel headers use this heavily (READ_ONCE, test_bit, etc.).
                if self.eat(TokenKind::LBrace) {
                    self.push_scope();
                    let mut stmts = Vec::new();
                    while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
                        stmts.push(self.parse_stmt()?);
                    }
                    self.expect(TokenKind::RBrace)?;
                    self.expect(TokenKind::RParen)?;
                    self.pop_scope();

                    while stmts.last().map_or(false, |s| matches!(s, Stmt::Empty)) {
                        stmts.pop();
                    }

                    let final_expr = if let Some(Stmt::Expr(_)) = stmts.last() {
                        if let Some(Stmt::Expr(e)) = stmts.pop() {
                            Box::new(e)
                        } else {
                            Box::new(Expr::Int(0))
                        }
                    } else {
                        Box::new(Expr::Int(0))
                    };

                    return Ok(Expr::StmtExpr(stmts, final_expr));
                }
                // compound literal: (type){ init }  OR cast (type)expr.
                // After consuming `(`, `is_cast_start` is no longer valid (it
                // looks from before `(`).  Re-check from current type token and
                // reject `(typedef_name)->field` — a parenthesized expression
                // when a parameter shadows the typedef (Redis quicklist).
                if self.is_typename() && self.is_cast_after_lparen() {
                    let ty = self.parse_type_name()?;
                    // more stars already in parse_type_name; allow abstract declarator *
                    while self.eat(TokenKind::Star) {
                        // already handled in parse_type_name partially
                    }
                    self.expect(TokenKind::RParen)?;
                    if self.at(&TokenKind::LBrace) {
                        let init = self.parse_initializer()?;
                        // treat as cast of a synthetic global — use Cast wrapping InitList
                        return Ok(Expr::Cast {
                            ty,
                            expr: Box::new(init),
                        });
                    }
                    // Soft: kernel macros may expand missing args to empty
                    // `(unsigned long)()` — treat empty operand as 0.
                    if self.eat(TokenKind::LParen) {
                        if self.eat(TokenKind::RParen) {
                            return Ok(Expr::Cast {
                                ty,
                                expr: Box::new(Expr::Int(0)),
                            });
                        }
                        // `(type)(expr)` — parenthesized operand
                        let e = self.parse_expr()?;
                        self.expect(TokenKind::RParen)?;
                        return Ok(Expr::Cast {
                            ty,
                            expr: Box::new(e),
                        });
                    }
                    // normal cast (type)expr
                    let e = self.parse_unary()?;
                    return Ok(Expr::Cast {
                        ty,
                        expr: Box::new(e),
                    });
                }
                // Soft: empty `()` as rvalue 0 (macro-expanded missing args).
                if self.eat(TokenKind::RParen) {
                    return Ok(Expr::Int(0));
                }
                let e = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                Ok(e)
            }
            _ => Err(format!(
                "unexpected token in expression: {:?} at {}:{}",
                self.peek_kind(),
                self.peek().line,
                self.peek().col
            )),
        }
    }
}

#[allow(dead_code)]
enum Postfix {
    Array(i64),
    Func,
}



pub fn parse(src: &str) -> Result<Program, String> {
    let toks = crate::lexer::Lexer::tokenize(src)?;
    let mut p = Parser::new(toks);
    p.parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gnu_attributes_in_pointer_and_func_declarator() {
        let src = "
            void * __no_caller_saved_registers__ delay_fn(unsigned long loops);
            static void __no_sanitize_coverage delay_loop(unsigned long loops) {}
            noinstr unsigned long spec_ctrl_current(void) { return 0; }
        ";
        let prog = parse(src);
        assert!(prog.is_ok(), "failed to parse gnu attributes in declarators: {:?}", prog.err());
    }

    #[test]
    fn test_parse_typeof_auto_type_and_leading_attributes() {
        let src = "
            void test_fn(void) {
                __auto_type x = 10;
                __attribute__((__section__(\".modinfo\"))) static const char info[] = \"test\";
                __no_sanitize_coverage typeof(x) y = 20;
            }
        ";
        let prog = parse(src);
        assert!(prog.is_ok(), "failed to parse typeof/auto_type/attrs: {:?}", prog.err());
    }

    #[test]
    fn test_parse_array_declarator_trailing_section_attribute() {
        let src = r#"
            char var[] __attribute__((__section__(".modinfo"))) = "test";
            static const char var2[] __section__(".modinfo2") = "test2";
        "#;
        let prog = parse(src);
        assert!(prog.is_ok(), "failed to parse array declarator section attribute: {:?}", prog.err());
        let items = prog.unwrap().items;
        assert_eq!(items.len(), 2);
        if let Item::Global(ref v) = items[0] {
            assert_eq!(v.name, "var");
            assert_eq!(v.section.as_deref(), Some(".modinfo"));
        } else {
            panic!("expected Item::Global");
        }
        if let Item::Global(ref v) = items[1] {
            assert_eq!(v.name, "var2");
            assert_eq!(v.section.as_deref(), Some(".modinfo2"));
        } else {
            panic!("expected Item::Global");
        }
    }

    #[test]
    fn test_parse_arrow_expression_contexts() {
        let src = r#"
            typedef struct { int flags; void *lock; } class_irqsave_t;
            struct task_struct { int flags; class_irqsave_t signal; };

            static inline void test_fn(class_irqsave_t *_T, struct task_struct *p) {
                _T->flags = 1;
                (void)(_T->lock);
                unsigned long x = __builtin_offsetof(struct task_struct, signal.lock);
                unsigned long y = __builtin_offsetof(struct task_struct, signal->lock);
                list_next_or_null_rcu(&p->signal.lock, &p->flags, struct task_struct, flags);
            }
        "#;
        let prog = parse(src);
        assert!(prog.is_ok(), "failed to parse arrow expression contexts: {:?}", prog.err());
    }

    /// `(struct Tag *)expr` must parse as cast (Redis SDS_HDR / sdslen).
    #[test]
    fn test_struct_tag_pointer_cast() {
        let src = r#"
            typedef char *sds;
            struct sdshdr8 {
                unsigned char len;
                unsigned char alloc;
                unsigned char flags;
                char buf[];
            };
            static inline unsigned long sdslen(const sds s) {
                unsigned char flags = s[-1];
                switch (flags & 7) {
                    case 0:
                        return flags >> 3;
                    case 1:
                        return ((struct sdshdr8 *)((s) - (sizeof(struct sdshdr8))))->len;
                }
                return 0;
            }
            unsigned long f(sds s) { return sdslen(s); }
        "#;
        let prog = parse(src).expect("parse struct-tag cast / sdslen");
        let names: Vec<_> = prog
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Func(f) if f.body.is_some() => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            names.contains(&"sdslen"),
            "sdslen body must parse, got funcs {:?}",
            names
        );
        assert!(names.contains(&"f"), "f must parse, got {:?}", names);
    }

    /// Forward `struct T;` must not wipe a later full definition's layout.
    #[test]
    fn test_forward_decl_does_not_zero_layout() {
        let src = r#"
            struct task_struct;
            struct task_struct {
                int __state;
                void *stack;
                long flags;
            };
            struct task_struct init_task = {
                .__state = 0,
                .stack = 0,
                .flags = 1,
            };
            unsigned long tsz = sizeof(struct task_struct);
        "#;
        let prog = parse(src).expect("parse forward+full");
        // type_layouts must have non-empty fields for task_struct
        let fields = prog
            .type_layouts
            .iter()
            .find(|(n, _, _, _)| n == "task_struct")
            .map(|(_, _, _, f)| f.len())
            .unwrap_or(0);
        assert!(fields >= 3, "task_struct fields in type_layouts, got {fields}");
        // Also ensure empty StructDef items exist (forward) without erasing fields map
        let empty_defs = prog.items.iter().filter(|i| matches!(i, Item::StructDef { name, fields } if name == "task_struct" && fields.is_empty())).count();
        let full_defs = prog.items.iter().filter(|i| matches!(i, Item::StructDef { name, fields } if name == "task_struct" && !fields.is_empty())).count();
        assert!(empty_defs + full_defs >= 1, "expected StructDef items");
    }

    /// Linux asm-offsets requires anonymous-union flatten + array-index offsetof.
    #[test]
    fn test_offsetof_anonymous_union_and_array_index() {
        let src = r#"
            struct thread_info {
                unsigned long flags;
                union {
                    unsigned long preempt_count;
                    struct {
                        unsigned int count;
                        unsigned int need_resched;
                    } preempt;
                };
                unsigned int cpu;
            };
            struct pt_regs {
                union {
                    struct {
                        unsigned long regs[31];
                        unsigned long sp;
                        unsigned long pc;
                        unsigned long pstate;
                    };
                };
                unsigned long orig_x0;
                int syscallno;
                unsigned int unused2;
                unsigned long sdei_ttbr1;
                unsigned long pmr_save;
                unsigned long stackframe[2];
                unsigned long lockdep_hardirqs;
                unsigned long exit_rcu;
            };
            struct task_struct {
                struct thread_info thread_info;
                unsigned int __state;
                void *stack;
            };
            unsigned long offs[] = {
                __builtin_offsetof(struct thread_info, flags),
                __builtin_offsetof(struct thread_info, preempt_count),
                __builtin_offsetof(struct thread_info, cpu),
                __builtin_offsetof(struct pt_regs, regs[0]),
                __builtin_offsetof(struct pt_regs, regs[2]),
                __builtin_offsetof(struct pt_regs, sp),
                __builtin_offsetof(struct pt_regs, pc),
                __builtin_offsetof(struct pt_regs, pstate),
                __builtin_offsetof(struct pt_regs, syscallno),
                sizeof(struct pt_regs),
                __builtin_offsetof(struct task_struct, thread_info.preempt_count),
                __builtin_offsetof(struct task_struct, stack),
                sizeof(struct thread_info),
            };
        "#;
        let prog = parse(src).expect("parse offsetof test");
        // Extract Int constants; sizeof may remain SizeofType until codegen.
        let mut vals: Vec<Option<i64>> = Vec::new();
        for item in &prog.items {
            if let Item::Global(v) = item {
                if v.name == "offs" {
                    if let Some(Expr::InitList { fields }) = &v.init {
                        for (_, e) in fields {
                            match e {
                                Expr::Int(n) => vals.push(Some(*n)),
                                Expr::SizeofType(_) => vals.push(None),
                                other => panic!("unexpected init expr {:?}", other),
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(vals.len(), 13, "offs array length");
        // GCC reference: ti flags/preempt/cpu; pt_regs regs[0]/regs[2]/sp/pc/pstate/syscallno;
        // sizeof(pt_regs); tsk.preempt; tsk.stack; sizeof(thread_info)
        assert_eq!(vals[0], Some(0));
        assert_eq!(vals[1], Some(8));
        assert_eq!(vals[2], Some(16));
        assert_eq!(vals[3], Some(0));
        assert_eq!(vals[4], Some(16));
        assert_eq!(vals[5], Some(248));
        assert_eq!(vals[6], Some(256));
        assert_eq!(vals[7], Some(264));
        assert_eq!(vals[8], Some(280));
        // vals[9] = sizeof(pt_regs) — folded at codegen
        assert_eq!(vals[10], Some(8));
        assert_eq!(vals[11], Some(32));
        // vals[12] = sizeof(thread_info)
    }

    #[test]
    fn test_parse_kernel_gnu_attributes_in_declarators() {
        let src = r#"
            int * __read_mostly ptr_var;
            int arr_var[10] __ro_after_init = {0};
            void * __no_caller_saved_registers delay_fn_1(unsigned long loops);
            static void __no_sanitize_coverage __no_kasan_or_inline delay_loop_1(unsigned long loops) {}
            void __no_profile __no_stack_protector test_fn_attr(void) {}
        "#;
        let prog = parse(src);
        assert!(prog.is_ok(), "failed to parse kernel gnu attributes in declarators: {:?}", prog.err());
    }

    #[test]
    fn test_decl_attr_hang() {
        let src = r#"
            typedef struct { void *lock; } class_preempt_t;
            void foo(void) {
                class_preempt_t _t, *_T __attribute__((__unused__)) = &_t;
            }
        "#;
        let prog = parse(src);
        assert!(prog.is_ok(), "failed to parse local decl with attr: {:?}", prog.err());
    }
}


#[cfg(test)]
mod weak_tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::ast::Item;

    #[test]
    fn weak_function_is_weak() {
        let src = "long __attribute__((weak)) foo(void) { return 1; }";
        let toks = Lexer::tokenize(src).unwrap();
        assert!(toks.iter().any(|t| matches!(t.kind, crate::token::TokenKind::Weak)));
        let mut p = Parser::new(toks);
        let prog = p.parse_program().unwrap();
        let f = prog.items.iter().find_map(|i| match i {
            Item::Func(f) if f.name == "foo" => Some(f),
            _ => None,
        }).expect("foo fn");
        assert!(f.is_weak, "foo should be weak, got is_weak={}", f.is_weak);
    }

    #[test]
    fn enum_const_containing_attribute_is_not_gnu_attr() {
        // Regression: is_gnu_attr_name used to match any *ATTRIBUTE* substring,
        // soft-skipping postgres attribute_reloptions (RELOPT_KIND_ATTRIBUTE).
        let src = r#"
            enum { RELOPT_KIND_ATTRIBUTE = 2 };
            typedef unsigned char bytea;
            typedef struct AttributeOpts { float n_distinct; } AttributeOpts;
            void *build_reloptions(int kind, unsigned long sz);
            bytea *attribute_reloptions(void) {
                return (bytea *) build_reloptions(RELOPT_KIND_ATTRIBUTE, sizeof(AttributeOpts));
            }
        "#;
        let toks = Lexer::tokenize(src).unwrap();
        let mut p = Parser::new(toks);
        let prog = p.parse_program().unwrap();
        assert!(
            prog.items.iter().any(|i| matches!(i, Item::Func(f) if f.name == "attribute_reloptions" && f.body.is_some())),
            "attribute_reloptions must parse with body"
        );
    }

    #[test]
    fn goto_indirect_parses() {
        let src = "void f(void *p) { goto *p; }";
        let toks = Lexer::tokenize(src).unwrap();
        let mut p = Parser::new(toks);
        let prog = p.parse_program().unwrap();
        let f = prog.items.iter().find_map(|i| match i {
            Item::Func(f) if f.name == "f" => Some(f),
            _ => None,
        }).expect("f");
        let body = f.body.as_ref().expect("body");
        assert!(
            body.iter().any(|s| matches!(s, crate::ast::Stmt::GotoIndirect(_))),
            "expected GotoIndirect, got {body:?}"
        );
    }

    #[test]
    fn abstract_array_bound_const_expr() {
        let src = r#"
            enum { INIT_FORKNUM = 3 };
            typedef int BlockNumber;
            void *palloc(unsigned long);
            void f(void) {
                BlockNumber (*block)[INIT_FORKNUM + 1];
                block = (BlockNumber (*)[INIT_FORKNUM + 1]) palloc(16);
                (void)block;
            }
        "#;
        let toks = Lexer::tokenize(src).unwrap();
        let mut p = Parser::new(toks);
        let prog = p.parse_program().unwrap();
        assert!(
            prog.items.iter().any(|i| matches!(i, Item::Func(f) if f.name == "f" && f.body.is_some())),
            "pointer-to-array cast with const expr bound must parse"
        );
    }

    #[test]
    fn test_forward_decl_does_not_zero_layout() {
        let src = r#"
            struct Plan;
            struct Plan {
                int type;
                int a;
                int b;
                int c;
            };
            struct Container {
                struct Plan plan;
                int extra;
            };
            int get_extra_offset(void) {
                return __builtin_offsetof(struct Container, extra);
            }
        "#;
        let toks = Lexer::tokenize(src).unwrap();
        let mut p = Parser::new(toks);
        let prog = p.parse_program().unwrap();
        assert!(
            prog.items.iter().any(|i| matches!(i, Item::Func(f) if f.name == "get_extra_offset")),
            "get_extra_offset must parse successfully"
        );
    }
}
