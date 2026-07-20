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
    /// scope-unique tag renames: (scope_id, tag) -> unique_name
    tag_scope: Vec<std::collections::HashMap<String, String>>,
    tag_serial: usize,
    pending_enum_globals: Vec<VarDecl>,
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
            tag_scope: vec![std::collections::HashMap::new()],
            tag_serial: 0,
            pending_enum_globals: Vec::new(),
        }
    }

    fn push_scope(&mut self) {
        self.tag_scope.push(std::collections::HashMap::new());
    }
    fn pop_scope(&mut self) {
        if self.tag_scope.len() > 1 {
            self.tag_scope.pop();
        }
    }
    fn resolve_tag(&mut self, tag: &str, define: bool) -> String {
        if define {
            self.tag_serial += 1;
            let uniq = format!("{tag}__s{}", self.tag_serial);
            if let Some(scope) = self.tag_scope.last_mut() {
                scope.insert(tag.to_string(), uniq.clone());
            }
            return uniq;
        }
        for scope in self.tag_scope.iter().rev() {
            if let Some(u) = scope.get(tag) {
                return u.clone();
            }
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
            TokenKind::Ident(s) => self.typedefs.iter().any(|t| t == s),
            _ => false,
        }
    }

    /// After seeing '(', decide if this is nested `(declarator)` vs function params.
    fn lparen_starts_nested_declarator(&self) -> bool {
        // Look at token after the '('.
        let j = self.i + 1;
        match self.toks.get(j).map(|t| &t.kind) {
            // (*...)  ((...))  ([...]) — nested
            Some(TokenKind::Star | TokenKind::LParen | TokenKind::LBracket) => true,
            // () empty → abstract function type (params), not nested
            Some(TokenKind::RParen) => false,
            Some(TokenKind::Ellipsis) => false,
            // type keywords / typedef → function parameter list
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
                // typedef name as type in params; ordinary ident is nested name `(foo)`
                !self.typedefs.iter().any(|t| t == s)
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
            while self.eat(TokenKind::Static)
                || self.eat(TokenKind::Extern)
                || self.eat(TokenKind::Register)
                || self.eat(TokenKind::Inline)
                || self.eat(TokenKind::Restrict)
                || self.eat(TokenKind::Auto)
                || self.eat(TokenKind::Const)
                || self.eat(TokenKind::Volatile)
            {}
            if self.at(&TokenKind::Typedef) {
                items.push(self.parse_typedef()?);
                continue;
            }
            if self.at(&TokenKind::Enum) && self.is_enum_tag_decl() {
                items.extend(self.parse_enum_item()?);
                continue;
            }
            // struct/union definition or forward decl at file scope: struct S { ... }; / struct S;
            if matches!(self.peek_kind(), TokenKind::Struct | TokenKind::Union)
                && self.is_struct_tag_decl()
            {
                items.push(self.parse_struct_or_union_item()?);
                continue;
            }
            items.extend(self.parse_decl_or_func()?);
        }
        // Flush enum constants discovered inside type specs (e.g. struct { enum { X } x; })
        for g in self.pending_enum_globals.drain(..) {
            items.insert(0, Item::Global(g));
        }
        let mut type_layouts = Vec::new();
        for (name, fields) in &self.struct_fields {
            let is_union = self.unions.iter().any(|u| u == name);
            type_layouts.push((name.clone(), is_union, fields.clone()));
        }
        Ok(Program {
            items,
            type_layouts,
        })
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
        matches!(
            self.toks.get(j).map(|t| &t.kind),
            Some(TokenKind::LBrace | TokenKind::Semicolon)
        )
    }

    fn is_enum_tag_decl(&self) -> bool {
        // enum E;  OR  enum E { ... }  OR  enum { ... }
        // NOT: enum E foo(...); / enum E x;
        let mut j = self.i;
        if !matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Enum)) {
            return false;
        }
        j += 1;
        if matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Ident(_))) {
            j += 1;
        }
        matches!(
            self.toks.get(j).map(|t| &t.kind),
            Some(TokenKind::LBrace | TokenKind::Semicolon)
        )
    }

    fn parse_struct_or_union_item(&mut self) -> Result<Item, String> {
        let is_union = self.at(&TokenKind::Union);
        self.bump();
        let name = if let TokenKind::Ident(s) = &self.peek_kind() {
            let s = s.clone();
            self.bump();
            s
        } else {
            return Err("anonymous struct at file scope needs a name".into());
        };
        if self.eat(TokenKind::Semicolon) {
            // forward declaration
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
                Ok(Item::Global(VarDecl {
                    name: vname,
                    ty: vty,
                    init,
                is_static: false,
            }))
            }
        }
    }

    fn parse_fields(&mut self) -> Result<Vec<Field>, String> {
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
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
                let w = match self.parse_expr()? {
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
                let (name, ty, _) = self.parse_declarator(base.clone())?;
                // Bit-field: `unsigned flags : 1;`
                let bit_width = if self.eat(TokenKind::Colon) {
                    let w = match self.parse_expr()? {
                        Expr::Int(n) => n.max(0) as u32,
                        _ => 1,
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
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                break;
            }
            self.expect(TokenKind::Semicolon)?;
        }
        Ok(fields)
    }

    fn parse_typedef(&mut self) -> Result<Item, String> {
        self.expect(TokenKind::Typedef)?;
        let base = self.parse_type_specifier()?;
        let (name, ty, _) = self.parse_declarator(base)?;
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
        // skip storage / signedness prefixes (may interleave with long/int)
        let mut saw_unsigned = false;
        let mut saw_signed = false;
        loop {
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
            if self.eat(TokenKind::Signed) {
                saw_signed = true;
                continue;
            }
            if self.eat(TokenKind::Unsigned) {
                saw_unsigned = true;
                continue;
            }
            break;
        }
        let _ = (saw_unsigned, saw_signed);
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
                Type::Int
            }
            TokenKind::Long => {
                self.bump();
                // long long / long int / long double
                if self.eat(TokenKind::Double) {
                    Type::Double
                } else {
                    let _ = self.eat(TokenKind::Long);
                    let _ = self.eat(TokenKind::Int);
                    Type::Long
                }
            }
            TokenKind::Short => {
                self.bump();
                let _ = self.eat(TokenKind::Int);
                Type::Short
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
            _ if saw_unsigned || saw_signed => Type::Int,
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
                        if let TokenKind::Ident(id) = self.peek_kind().clone() {
                            self.bump();
                            if self.eat(TokenKind::Assign) {
                                if let TokenKind::IntLit(n) = self.peek_kind().clone() {
                                    self.bump();
                                    next_val = n;
                                }
                            }
                            // store as int global for expression use
                            // (parser side-effect via temporary list on self)
                            self.pending_enum_globals.push(VarDecl {
                                name: id,
                                ty: Type::Int,
                                init: Some(Expr::Int(next_val)),
                                is_static: false,
                            });
                            next_val += 1;
                        }
                        if self.eat(TokenKind::Comma) {
                            continue;
                        }
                        break;
                    }
                    self.expect(TokenKind::RBrace)?;
                }
                Type::Int
            }
            TokenKind::Ident(s) if self.typedefs.iter().any(|t| t == &s) => {
                self.bump();
                self.typedef_map.get(&s).cloned().unwrap_or(Type::Int)
            }
            _ => {
                return Err(format!(
                    "expected type at {}:{}",
                    self.peek().line,
                    self.peek().col
                ));
            }
        };
        // Trailing type qualifiers: `char const`, `int volatile`, etc.
        while self.eat(TokenKind::Const) || self.eat(TokenKind::Volatile) {}
        Ok(ty)
    }

    fn parse_enum_item(&mut self) -> Result<Vec<Item>, String> {
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
            let id = if let TokenKind::Ident(s) = self.peek_kind().clone() {
                self.bump();
                s
            } else {
                return Err("enum enumerator name expected".into());
            };
            if self.eat(TokenKind::Assign) {
                if let TokenKind::IntLit(n) = self.peek_kind().clone() {
                    self.bump();
                    next_val = n;
                } else if let TokenKind::CharLit(n) = self.peek_kind().clone() {
                    self.bump();
                    next_val = n;
                }
            }
            // Emit as global const int
            items.push(Item::Global(VarDecl {
                name: id,
                ty: Type::Int,
                init: Some(Expr::Int(next_val)),
            is_static: false,
        }));
            next_val += 1;
            if self.eat(TokenKind::Comma) {
                continue;
            }
            break;
        }
        self.expect(TokenKind::RBrace)?;
        // enum fred { ... } optional trailing name/var
        if let TokenKind::Ident(_) = self.peek_kind().clone() {
            self.bump();
        }
        self.eat(TokenKind::Semicolon);
        Ok(items)
    }

    fn parse_struct_type(&mut self, is_union: bool) -> Result<Type, String> {
        self.bump(); // struct/union
        let mut name: Option<String> = None;
        if let TokenKind::Ident(s) = self.peek_kind().clone() {
            name = Some(s);
            self.bump();
        }
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
            if let Some(uniq) = uniq_opt {
                self.struct_fields.insert(uniq.clone(), fields.clone());
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
            if is_union {
                Ok(Type::Union(uniq))
            } else {
                Ok(Type::Struct(uniq))
            }
        } else {
            Err("struct/union without name or body".into())
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
            let (pn, pt, _) = self.parse_declarator(pb)?;
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
    fn parse_declarator(
        &mut self,
        base: Type,
    ) -> Result<(String, Type, Option<(Vec<(String, Type)>, bool)>), String> {
        let mut ty = base;
        while self.eat(TokenKind::Star) {
            // pointer qualifiers: *const / *volatile / *restrict
            loop {
                match self.peek_kind().clone() {
                    TokenKind::Const | TokenKind::Volatile => {
                        self.bump();
                    }
                    TokenKind::Ident(s) if s == "restrict" => {
                        self.bump();
                    }
                    _ => break,
                }
            }
            ty = Type::Ptr(Box::new(ty));
        }
        // '(' starts a nested declarator (*name)/(*name()) only when the next
        // token looks like a declarator, not a type (function-parameter list).
        // So `int (int x)` / `int ()` are abstract function types, while
        // `int (*f)(void)` / `int (foo)` are nested declarators.
        let (name, mut ty, nested, mut bubbled_fp) = if self.at(&TokenKind::LParen)
            && self.lparen_starts_nested_declarator()
        {
            self.bump(); // (
            let (n, inner, inner_fp) = self.parse_declarator(ty)?;
            self.expect(TokenKind::RParen)?;
            // Bubble function params from `(*name(params))` so definitions work:
            // `void (*f(T))(void) { ... }` is a function named f.
            (n, inner, true, inner_fp)
        } else if let TokenKind::Ident(s) = self.peek_kind().clone() {
            // Don't consume typedef names as declarator identifiers when they
            // appear where a nested type would be unexpected — still OK as name.
            self.bump();
            (s, ty, false, None)
        } else {
            (String::new(), ty, false, None)
        };
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
                    TokenKind::Ident(s) if s == "restrict" || s == "__restrict" || s == "__restrict__" => {
                        self.bump();
                    }
                    _ => break,
                }
            }
            let n = if self.eat(TokenKind::Star) {
                0
            } else if self.at(&TokenKind::RBracket) {
                0
            } else {
                // Constant expression: 11, (11+1), sizeof...
                let e = self.parse_expr()?;
                Self::const_array_len(&e).unwrap_or(0)
            };
            self.expect(TokenKind::RBracket)?;
            dims.push(n);
        }
        for n in dims.into_iter().rev() {
            if nested {
                ty = Self::array_under_ptrs(ty, n);
            } else {
                ty = Type::Array(Box::new(ty), n);
            }
        }
        // Function suffixes:
        // - bare `name(params)` → function prototype/definition (return params)
        // - `(*name)(params)` → pointer-to-function variable (no func params)
        // - `(*name(params))(params2)` → function returning function pointer
        //   (params bubbled from inner; params2 is return type sugar)
        let mut func_params: Option<(Vec<(String, Type)>, bool)> = None;
        if !nested && self.at(&TokenKind::LParen) {
            self.bump();
            let params = self.parse_param_list_body()?;
            self.expect(TokenKind::RParen)?;
            func_params = Some(params);
            while self.at(&TokenKind::LParen) {
                self.bump();
                let _ = self.parse_param_list_body()?;
                self.expect(TokenKind::RParen)?;
            }
        } else {
            // nested / abstract: absorb trailing (params) as type sugar
            while self.at(&TokenKind::LParen) {
                self.bump();
                let _ = self.parse_param_list_body()?;
                self.expect(TokenKind::RParen)?;
            }
            // Promote bubbled params from (*name(params)) form
            if bubbled_fp.is_some() {
                func_params = bubbled_fp.take();
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

    fn const_array_len(e: &Expr) -> Option<i64> {
        match e {
            Expr::Int(n) | Expr::Char(n) => Some(*n),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(-Self::const_array_len(expr)?),
            Expr::Binary { op, left, right } => {
                let l = Self::const_array_len(left)?;
                let r = Self::const_array_len(right)?;
                Some(match op {
                    BinOp::Add => l.wrapping_add(r),
                    BinOp::Sub => l.wrapping_sub(r),
                    BinOp::Mul => l.wrapping_mul(r),
                    BinOp::Div if r != 0 => l / r,
                    _ => return None,
                })
            }
            Expr::Cast { expr, .. } => Self::const_array_len(expr),
            _ => None,
        }
    }

    fn parse_decl_or_func(&mut self) -> Result<Vec<Item>, String> {
        let base = self.parse_type_specifier()?;
        // Could be: type name(...) { }  or type name, name2;
        // Function params are part of the declarator (including multi-suffix forms).
        let (name, mut ty, func_params) = self.parse_declarator(base.clone())?;
        if name.is_empty() {
            return Err(format!(
                "expected declarator name at {}:{}",
                self.peek().line,
                self.peek().col
            ));
        }
        if let Some((params, variadic)) = func_params {
            // Function prototype or definition
            if self.eat(TokenKind::Semicolon) {
                return Ok(vec![Item::Func(Function {
                    name,
                    ret: ty,
                    params,
                    variadic,
                    body: None,
                })]);
            }
            if self.at(&TokenKind::LBrace) {
                let body = self.parse_block()?;
                return Ok(vec![Item::Func(Function {
                    name,
                    ret: ty,
                    params,
                    variadic,
                    body: Some(body),
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
                })];
                loop {
                    let (n2, t2, fp2) = self.parse_declarator(base.clone())?;
                    if let Some((p2, v2)) = fp2 {
                        items.push(Item::Func(Function {
                            name: n2,
                            ret: base.clone(),
                            params: p2,
                            variadic: v2,
                            body: None,
                        }));
                        let _ = t2;
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
                            is_static: false,
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
        let init = if self.eat(TokenKind::Assign) {
            Some(self.parse_initializer()?)
        } else {
            None
        };
        let ty = Self::infer_array_size(ty, &init);
        items.push(Item::Global(VarDecl {
            name,
            ty: ty.clone(),
            init,
        is_static: false,
    }));
        while self.eat(TokenKind::Comma) {
            let (n2, t2, _) = self.parse_declarator(base.clone())?;
            let init2 = if self.eat(TokenKind::Assign) {
                Some(self.parse_initializer()?)
            } else {
                None
            };
            let t2 = Self::infer_array_size(t2, &init2);
            items.push(Item::Global(VarDecl {
                name: n2,
                ty: t2,
                init: init2,
            is_static: false,
        }));
        }
        self.expect(TokenKind::Semicolon)?;
        let _ = &ty;
        Ok(items)
    }

    fn infer_array_size(ty: Type, init: &Option<Expr>) -> Type {
        match (&ty, init) {
            (Type::Array(elem, 0), Some(Expr::String(s))) => {
                Type::Array(elem.clone(), (s.len() as i64) + 1)
            }
            (Type::Array(elem, 0), Some(Expr::InitList { fields })) => {
                // max of len or designated indices
                let mut max_i = fields.len() as i64;
                let mut idx = 0i64;
                for (name, _) in fields {
                    if let Some(n) = name {
                        // designated [n] encoded? we use name as "0","1" for index later
                        if let Ok(i) = n.parse::<i64>() {
                            idx = i + 1;
                            if idx > max_i {
                                max_i = idx;
                            }
                            continue;
                        }
                    }
                    idx += 1;
                    if idx > max_i {
                        max_i = idx;
                    }
                }
                // simpler: track while parsing — use fields len and [n] designators stored as Some(index)
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
                Type::Array(elem.clone(), high.max(1))
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
                    let field = if let TokenKind::Ident(s) = self.peek_kind().clone() {
                        self.bump();
                        s
                    } else {
                        return Err("designated init field name".into());
                    };
                    self.expect(TokenKind::Assign)?;
                    fields.push((Some(field), self.parse_initializer()?));
                } else if self.eat(TokenKind::LBracket) {
                    // designated array index [n] = expr
                    let idx = if let TokenKind::IntLit(n) = self.peek_kind().clone() {
                        self.bump();
                        n
                    } else {
                        return Err("array designator index".into());
                    };
                    self.expect(TokenKind::RBracket)?;
                    self.expect(TokenKind::Assign)?;
                    fields.push((Some(idx.to_string()), self.parse_initializer()?));
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

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        // block-scope typedef: typedef enum { e } h;
        if self.at(&TokenKind::Typedef) {
            let item = self.parse_typedef()?;
            // typedef doesn't produce a runtime stmt; register via empty (parser
            // already recorded typedef into self.typedefs via parse_typedef).
            let _ = item;
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
        // Detect storage class before type specifier (static is eaten inside
        // parse_type_specifier too, so peek first).
        let is_static = self.at(&TokenKind::Static) || self.at(&TokenKind::Register);
        let base = self.parse_type_specifier()?;
        // multi-decl: int x, *p, **pp;
        // For simplicity emit a block of decls; first returned as Decl, rest as Block if multiple
        let mut decls = Vec::new();
        loop {
            let (name, ty, _) = self.parse_declarator(base.clone())?;
            // Skip function prototype: name(...);
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
            let ty = Self::infer_array_size(ty, &init);
            decls.push(VarDecl {
                name,
                ty,
                init,
                is_static,
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
            Ok(Stmt::Block(
                decls.into_iter().map(Stmt::Decl).collect(),
            ))
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
            let t = self.parse_expr()?;
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

    fn is_cast_start(&self) -> bool {
        let mut j = self.i + 1; // after (
        // optional stars then type keyword or typedef
        while matches!(self.toks.get(j).map(|t| &t.kind), Some(TokenKind::Star)) {
            j += 1;
        }
        match self.toks.get(j).map(|t| &t.kind) {
            Some(
                TokenKind::Int
                | TokenKind::Void
                | TokenKind::Char
                | TokenKind::Long
                | TokenKind::Short
                | TokenKind::Struct
                | TokenKind::Union,
            ) => true,
            Some(TokenKind::Ident(s)) => self.typedefs.iter().any(|t| t == s),
            _ => false,
        }
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
                    TokenKind::Const | TokenKind::Volatile => {
                        self.bump();
                    }
                    TokenKind::Ident(s) if s == "restrict" => {
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
                    let nsz = if let TokenKind::IntLit(v) = self.peek_kind().clone() {
                        self.bump();
                        v
                    } else {
                        0
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
            let nsz = if let TokenKind::IntLit(v) = self.peek_kind().clone() {
                self.bump();
                v
            } else {
                0
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
                        args.push(self.parse_assign()?);
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
                let t = self.expect(TokenKind::Ident(String::new()))?;
                let field = match t.kind {
                    TokenKind::Ident(s) => s,
                    _ => unreachable!(),
                };
                e = Expr::Member {
                    base: Box::new(e),
                    field,
                    arrow: false,
                };
            } else if self.eat(TokenKind::Arrow) {
                let t = self.expect(TokenKind::Ident(String::new()))?;
                let field = match t.kind {
                    TokenKind::Ident(s) => s,
                    _ => unreachable!(),
                };
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
                Ok(Expr::Int(n))
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
            TokenKind::Ident(name) => {
                self.bump();
                Ok(Expr::Var(name))
            }
            TokenKind::LParen => {
                self.bump();
                // compound literal: (type){ init }
                if self.is_typename() {
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
                    // normal cast (type)expr
                    let e = self.parse_unary()?;
                    return Ok(Expr::Cast {
                        ty,
                        expr: Box::new(e),
                    });
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

enum Postfix {
    Array(i64),
    Func,
}

pub fn parse(src: &str) -> Result<Program, String> {
    let toks = crate::lexer::Lexer::tokenize(src)?;
    let mut p = Parser::new(toks);
    p.parse_program()
}
