#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
    /// Named struct/union layouts discovered during parsing (including inline defs).
    pub type_layouts: Vec<(String, bool, Vec<Field>)>, // name, is_union, fields
}

#[derive(Debug, Clone)]
pub enum Item {
    Func(Function),
    Global(VarDecl),
    Typedef { name: String, ty: Type },
    StructDef { name: String, fields: Vec<Field> },
    UnionDef { name: String, fields: Vec<Field> },
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    /// Bit-field width when declared as `T name : N;`. `None` = ordinary field.
    /// Width 0 is a zero-width bit-field (alignment marker only).
    pub bit_width: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub ret: Type,
    pub params: Vec<(String, Type)>,
    /// True if the prototype ends with `...` (variadic).
    pub variadic: bool,
    pub body: Option<Vec<Stmt>>, // None = declaration only
    /// File-scope `static` function (internal linkage). Kernel headers pull in
    /// thousands of static inlines; codegen may skip their bodies for speed.
    pub is_static: bool,
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub name: String,
    pub ty: Type,
    pub init: Option<Expr>,
    /// Function-scope static → emit as hidden global with static duration.
    pub is_static: bool,
}

#[derive(Debug, Clone)]
pub enum Type {
    Void,
    /// Plain / unsigned char (zero-extended on load).
    Char,
    /// `signed char` (sign-extended on load). Required for lemon `yyRuleInfoNRhs[]`.
    SChar,
    Short,
    /// `unsigned short` (zero-extended on load).
    UShort,
    Int,
    /// `unsigned` / `unsigned int` (zero-extended on load). Critical for SQLite
    /// `Pgno` / `u32` page numbers (mxPgno = 0xfffffffe must not become -2).
    UInt,
    Long,
    /// `unsigned long` / `unsigned long long` / `u64`.
    ULong,
    Float,
    Double,
    Ptr(Box<Type>),
    Array(Box<Type>, i64),
    Struct(String),
    Union(String),
    AnonStruct(Vec<Field>),
    AnonUnion(Vec<Field>),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Block(Vec<Stmt>),
    Decl(VarDecl),
    Expr(Expr),
    Return(Option<Expr>),
    If {
        cond: Expr,
        then_b: Box<Stmt>,
        else_b: Option<Box<Stmt>>,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
    },
    DoWhile {
        body: Box<Stmt>,
        cond: Expr,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Box<Stmt>,
    },
    Break,
    Continue,
    Goto(String),
    Label(String, Box<Stmt>),
    Switch {
        cond: Expr,
        body: Box<Stmt>,
    },
    Case {
        value: Expr,
        body: Box<Stmt>,
    },
    Default(Box<Stmt>),
    Empty,
    /// Fully-resolved GNU basic asm lines (after "i" constraint substitution).
    /// Used by kernel kbuild `DEFINE(sym, val)` → `.ascii "->sym val ..."`.
    Asm { lines: Vec<String> },
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Char(i64),
    String(String),
    Var(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Assign {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    CompoundAssign {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// Brace initializer: optional field names for designated init.
    InitList {
        fields: Vec<(Option<String>, Expr)>,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Member {
        base: Box<Expr>,
        field: String,
        arrow: bool,
    },
    Cast {
        ty: Type,
        expr: Box<Expr>,
    },
    SizeofType(Type),
    SizeofExpr(Box<Expr>),
    Cond {
        cond: Box<Expr>,
        then_e: Box<Expr>,
        else_e: Box<Expr>,
    },
    PreInc(Box<Expr>),
    PreDec(Box<Expr>),
    PostInc(Box<Expr>),
    PostDec(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
    Addr,
    Deref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    /// Comma operator: evaluate left for side effects, result is right.
    Comma,
}
