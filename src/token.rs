#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // punctuation / operators
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Colon,
    Dot,
    Arrow, // ->
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,      // &
    Pipe,     // |
    Caret,    // ^
    Tilde,    // ~
    Bang,     // !
    Question, // ?
    Assign,   // =
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AndEq,   // &=
    OrEq,    // |=
    XorEq,   // ^=
    ShlEq,   // <<=
    ShrEq,   // >>=
    Eq,    // ==
    Ne,    // !=
    Lt,
    Gt,
    Le,
    Ge,
    AndAnd, // &&
    OrOr,   // ||
    Shl,    // <<
    Shr,    // >>
    PlusPlus,
    MinusMinus,
    Ellipsis, // ...
    // keywords
    Int,
    Void,
    Char,
    Long,
    Short,
    Float,
    Double,
    Struct,
    Union,
    Typedef,
    Enum,
    Unsigned,
    Signed,
    Static,
    Extern,
    Register,
    Inline,
    Restrict,
    Auto,
    Const,
    Volatile,
    Return,
    If,
    Else,
    While,
    For,
    Do,
    Break,
    Continue,
    Goto,
    Switch,
    Case,
    Default,
    Sizeof,
    /// Sticky GNU `__attribute__((packed))` — fields pack with align 1.
    Packed,
    // literals / idents
    Ident(String),
    IntLit(i64),
    FloatLit(f64),
    CharLit(i64),
    StringLit(String),
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}
