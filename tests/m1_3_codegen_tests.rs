use acc::codegen::{Target, TargetOs};
use acc::driver::{compile, CompileOptions};
use std::process::Command;

fn compile_and_run(c_code: &str, test_name: &str) -> (i32, String, String) {
    let tmp_dir = std::env::temp_dir();
    let src_path = tmp_dir.join(format!("{test_name}.c"));
    let bin_path = tmp_dir.join(test_name);

    std::fs::write(&src_path, c_code).expect("write src");

    let opts = CompileOptions {
        input: src_path,
        output: bin_path.clone(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::default(),
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };

    compile(&opts).expect("in-process compilation failed");

    let output = Command::new(&bin_path)
        .output()
        .expect("execution failed");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.code().unwrap_or(-1), stdout, stderr)
}

#[test]
fn test_large_struct_return_direct_member_access() {
    let src = r#"
#include <stdio.h>

struct Big {
    long a;
    long b;
    long c;
};

struct Big get_big(void) {
    struct Big s;
    s.a = 100;
    s.b = 200;
    s.c = 300;
    return s;
}

int main(void) {
    if (get_big().a != 100) return 1;
    if (get_big().b != 200) return 2;
    if (get_big().c != 300) return 3;
    return 0;
}
"#;
    let (code, _, _) = compile_and_run(src, "test_large_struct_ret_member");
    assert_eq!(code, 0, "Large struct return member access failed");
}

#[test]
fn test_relocatable_pointer_alignment_struct_initializer() {
    let src = r#"
#include <stdio.h>

typedef void (*fptr)(void);
static void dummy(void) {}

struct Wrap {
    char pad;
    fptr fn_ptr;
};

static struct Wrap w[] = {
    { 'a', dummy },
    { 'b', dummy },
};

int main(void) {
    if (w[0].pad != 'a') return 1;
    if (w[1].pad != 'b') return 2;
    if (w[0].fn_ptr != dummy) return 3;
    return 0;
}
"#;
    let (code, _, _) = compile_and_run(src, "test_reloc_ptr_align");
    assert_eq!(code, 0, "Relocatable pointer alignment test failed");
}

#[test]
fn test_mutable_global_array_placement() {
    let src = r#"
#include <stdio.h>

char t[] = "012345678";

int main(void) {
    char *data = t;
    data[4] = 'X';
    if (data[4] != 'X') return 1;
    return 0;
}
"#;
    let (code, _, _) = compile_and_run(src, "test_mutable_global_arr");
    assert_eq!(code, 0, "Mutable global array modification failed");
}

#[test]
fn test_single_exec_00204() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "third_party/c-testsuite/tests/single-exec/00204.c".into(),
        output: "target/worker_test/test_00204_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::default(),
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile 00204.c");

    let output = Command::new("target/worker_test/test_00204_bin")
        .output()
        .expect("run 00204.c");
    assert_eq!(output.status.code(), Some(0), "00204.c execution failed");
}

#[test]
fn test_single_exec_00216() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "third_party/c-testsuite/tests/single-exec/00216.c".into(),
        output: "target/worker_test/test_00216_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::default(),
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile 00216.c");

    let output = Command::new("target/worker_test/test_00216_bin")
        .output()
        .expect("run 00216.c");
    assert_eq!(output.status.code(), Some(0), "00216.c execution failed");
}

#[test]
fn test_single_exec_00217() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "third_party/c-testsuite/tests/single-exec/00217.c".into(),
        output: "target/worker_test/test_00217_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::default(),
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile 00217.c");

    let output = Command::new("target/worker_test/test_00217_bin")
        .output()
        .expect("run 00217.c");
    assert_eq!(output.status.code(), Some(0), "00217.c execution failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("data = \"0123-5678\""), "00217.c output mismatch: {}", stdout);
}
