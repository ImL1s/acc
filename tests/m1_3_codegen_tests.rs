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
        target: Target::host(),
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
    let (code, stdout, stderr) = compile_and_run(src, "test_large_struct_ret_member");
    println!("code={}, stdout={}, stderr={}", code, stdout, stderr);
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
    if Target::host() != Target::Aarch64 {
        return;
    }
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "third_party/c-testsuite/tests/single-exec/00204.c".into(),
        output: "target/worker_test/test_00204_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::host(),
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
        target: Target::host(),
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
        target: Target::host(),
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

#[test]
fn test_pg_plan_struct_layout() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "tests/oracle/pg_plan_struct_layout.c".into(),
        output: "target/worker_test/test_pg_plan_struct_layout_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::host(),
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile pg_plan_struct_layout.c");

    let output = Command::new("target/worker_test/test_pg_plan_struct_layout_bin")
        .output()
        .expect("run pg_plan_struct_layout_bin");
    assert_eq!(output.status.code(), Some(0), "pg_plan_struct_layout execution failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PG_PLAN_STRUCT_LAYOUT_OK"), "output mismatch: {}", stdout);
}

#[test]
fn test_pg_switch_flex_bison() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "tests/oracle/pg_switch_flex_bison.c".into(),
        output: "target/worker_test/test_pg_switch_flex_bison_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::host(),
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile pg_switch_flex_bison.c");

    let output = Command::new("target/worker_test/test_pg_switch_flex_bison_bin")
        .output()
        .expect("run pg_switch_flex_bison_bin");
    assert_eq!(output.status.code(), Some(0), "pg_switch_flex_bison execution failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PG_SWITCH_FLEX_BISON_OK"), "output mismatch: {}", stdout);
}

#[test]
fn test_sysv_valist_vsnprintf() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "tests/oracle/sysv_valist_vsnprintf.c".into(),
        output: "target/worker_test/test_sysv_valist_vsnprintf_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::X86_64,
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile sysv_valist_vsnprintf.c");

    let output = Command::new("target/worker_test/test_sysv_valist_vsnprintf_bin")
        .output()
        .expect("run sysv_valist_vsnprintf_bin");
    assert_eq!(output.status.code(), Some(0), "sysv_valist_vsnprintf execution failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SYSV_VALIST_VSNPRINTF_OK"), "output mismatch: {}", stdout);
}

#[test]
fn test_indirect_call_7args() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "tests/oracle/indirect_call_7args.c".into(),
        output: "target/worker_test/test_indirect_call_7args_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::X86_64,
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile indirect_call_7args.c");

    let output = Command::new("target/worker_test/test_indirect_call_7args_bin")
        .output()
        .expect("run indirect_call_7args_bin");
    assert_eq!(output.status.code(), Some(0), "indirect_call_7args execution failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("INDIRECT_CALL_7ARGS_OK"), "output mismatch: {}", stdout);
}

#[test]
fn test_indirect_call_alignment() {
    let src = r#"
#include <stdio.h>
#include <stdint.h>

typedef void (*fn_t)(int, int, int, int, int, int, int);

void target(int a1, int a2, int a3, int a4, int a5, int a6, int a7) {
    uintptr_t frame = (uintptr_t)__builtin_frame_address(0);
    if ((frame & 0xF) != 0) {
        printf("MISALIGNED %p\n", (void*)frame);
        return;
    }
    printf("INDIRECT_ALIGN_OK %d\n", a1 + a2 + a3 + a4 + a5 + a6 + a7);
}

int main(void) {
    fn_t f = target;
    f(1, 2, 3, 4, 5, 6, 7);
    return 0;
}
"#;
    let (code, stdout, stderr) = compile_and_run(src, "test_indirect_call_alignment");
    println!("code={}, stdout={}, stderr={}", code, stdout, stderr);
    assert_eq!(code, 0, "indirect call alignment failed");
    assert!(stdout.contains("INDIRECT_ALIGN_OK 28"));
}

#[test]
fn test_compound_literal_bare_func_ptr() {
    let src = r#"
#include <stdio.h>

typedef int (*fn_t)(int);
static int my_fn(int x) { return x * 2; }

struct Routine {
    fn_t f;
};

#define ROUTINE(fn) &(struct Routine){ .f = fn }

int main(void) {
    struct Routine *r = ROUTINE(my_fn);
    if (!r->f) return 1;
    if (r->f(21) != 42) return 2;
    printf("COMPOUND_BARE_FUNC_OK\n");
    return 0;
}
"#;
    let (code, stdout, stderr) = compile_and_run(src, "test_compound_literal_bare_func_ptr");
    println!("code={}, stdout={}, stderr={}", code, stdout, stderr);
    assert_eq!(code, 0, "compound literal bare func ptr failed");
    assert!(stdout.contains("COMPOUND_BARE_FUNC_OK"));
}

#[test]
fn test_unsigned_indexing_zero_extension() {
    let src = r#"
#include <stdio.h>
#include <stdint.h>

typedef uint32_t WalSegNo;

static char buf[256];

char get_elem(char *base, WalSegNo idx) {
    return base[idx];
}

int main(void) {
    for (int i = 0; i < 256; i++) buf[i] = (char)i;
    WalSegNo seg = 10;
    if (get_elem(buf, seg) != 10) return 1;
    printf("UNSIGNED_INDEXING_OK\n");
    return 0;
}
"#;
    let (code, stdout, stderr) = compile_and_run(src, "test_unsigned_indexing_zero_extension");
    println!("code={}, stdout={}, stderr={}", code, stdout, stderr);
    assert_eq!(code, 0, "unsigned indexing failed");
    assert!(stdout.contains("UNSIGNED_INDEXING_OK"));
}




