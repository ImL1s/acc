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
fn test_shift_operator_result_type_00200() {
    let src = r#"
#include <stdio.h>

int main(void) {
    short s = 1;
    long long shift = 1;
    if (sizeof(s << shift) != sizeof(int)) return 1;
    if (sizeof((unsigned char)1 << (long long)1) != sizeof(int)) return 2;
    if (sizeof((long long)1 << (short)1) != sizeof(long long)) return 3;
    return 0;
}
"#;
    let (code, _, _) = compile_and_run(src, "test_shift_type");
    assert_eq!(code, 0, "Shift result type check failed");
}

#[test]
fn test_single_exec_00200() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "third_party/c-testsuite/tests/single-exec/00200.c".into(),
        output: "target/worker_test/test_00200_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::default(),
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile 00200.c");

    let output = Command::new("target/worker_test/test_00200_bin")
        .output()
        .expect("run 00200.c");
    assert_eq!(output.status.code(), Some(0), "00200.c execution failed");
}

#[test]
fn test_subaggregate_brace_elision_00205() {
    let src = r#"
#include <stdio.h>

struct Data {
    int arr[3];
    int val1;
    int val2;
};

struct Data items[] = {
    { { 10, 20, 30 }, 40, 50 }
};

int main(void) {
    if (items[0].arr[0] != 10) return 1;
    if (items[0].arr[1] != 20) return 2;
    if (items[0].arr[2] != 30) return 3;
    if (items[0].val1 != 40) return 4;
    if (items[0].val2 != 50) return 5;
    return 0;
}
"#;
    let (code, _, _) = compile_and_run(src, "test_subagg_elision");
    assert_eq!(code, 0, "Subaggregate brace elision check failed");
}

#[test]
fn test_single_exec_00205() {
    std::fs::create_dir_all("target/worker_test").ok();
    let opts = CompileOptions {
        input: "third_party/c-testsuite/tests/single-exec/00205.c".into(),
        output: "target/worker_test/test_00205_bin".into(),
        keep_asm: false,
        emit_asm_only: false,
        target: Target::default(),
        target_os: TargetOs::host(),
        linker: None,
        include_dirs: vec![],
        defines: vec![],
        force_includes: vec![],
    };
    compile(&opts).expect("compile 00205.c");

    let output = Command::new("target/worker_test/test_00205_bin")
        .output()
        .expect("run 00205.c");
    assert_eq!(output.status.code(), Some(0), "00205.c execution failed");
}
