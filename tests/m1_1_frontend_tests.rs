// Unit tests for Sub-milestone M1.1 Frontend & Preprocessor Fixes
// Fixes: 00104.c (Hex Literal Typing), 00124.c (Func Ret Func Pointer Declarator),
// 00206.c (Macro Push/Pop Pragma Stack), 00219.c (_Generic Type Selection), 00220.c (Wide String Literal UTF-8 Tokenization)

use acc::lexer::Lexer;
use acc::parser::parse;
use acc::preprocess::preprocess_with_options;
use acc::token::TokenKind;

#[test]
fn test_00104_hex_literal_typing() {
    let code = "int f() { return 0xffffffff > 0; }";
    let prog = parse(code).unwrap();
    assert!(!prog.items.is_empty());
}

#[test]
fn test_00124_func_ret_func_ptr_declarator() {
    let code = "int (* f1(int a, int b))(int c, int d);";
    let prog = parse(code).unwrap();
    assert_eq!(prog.items.len(), 1);
}

#[test]
fn test_00206_pragma_push_pop_macro() {
    let input = r#"
#define FOO 1
#pragma push_macro("FOO")
#undef FOO
#define FOO 2
int a = FOO;
#pragma pop_macro("FOO")
int b = FOO;
"#;
    let output = preprocess_with_options(input, None, &[], true, "test.c").unwrap();
    assert!(output.contains("int a = 2;"));
    assert!(output.contains("int b = 1;"));
}

#[test]
fn test_00219_generic_type_selection() {
    let code = r#"
int main() {
    const int x = 1;
    int a = _Generic(x, int: 10, default: 20);
    int b = _Generic((const int)1, int: 30, default: 40);
    return a + b;
}
"#;
    let prog = parse(code).unwrap();
    assert!(!prog.items.is_empty());
}

#[test]
fn test_00220_wide_string_utf8_tokenization() {
    let code = r#"wchar_t *s = L"Hello 世界 \u0041 \U00000042";"#;
    let mut lexer = Lexer::new(code);
    let _ = lexer.next_token().unwrap(); // wchar_t
    let _ = lexer.next_token().unwrap(); // *
    let _ = lexer.next_token().unwrap(); // s
    let _ = lexer.next_token().unwrap(); // =
    let tok = lexer.next_token().unwrap(); // Wide String Literal
    if let TokenKind::StringLit(s) = tok.kind {
        // UTF-8 decoded characters check
        assert!(s.contains("世界"));
        assert!(s.contains('A'));
        assert!(s.contains('B'));
    } else {
        panic!("expected wide string literal token");
    }
}
