use acc::codegen::{Target, TargetOs};
use acc::driver::{compile, CompileOptions};
use std::process::Command;

fn compile_and_run(c_code: &str, test_name: &str) -> (i32, String, String) {
    let tmp_dir = std::env::temp_dir();
    let src_path = tmp_dir.join(format!("{test_name}.c"));
    let bin_path = tmp_dir.join(test_name);
    let _ = std::fs::remove_file(&bin_path);

    std::fs::write(&src_path, c_code).expect("write src");

    let opts = CompileOptions {
        input: src_path.clone(),
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
        target: Target::host(),
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
        target: Target::host(),
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

#[test]
fn test_shift_unary_stress_test() {
    let src = r#"
#include <stdio.h>

int main(void) {
    // 1. Shift by 0
    if ((15 << 0) != 15) return 101;
    if ((15 >> 0) != 15) return 102;
    if ((15u << 0) != 15u) return 103;
    if ((15u >> 0) != 15u) return 104;

    // 2. Shift 32-bit by 31
    if (((unsigned int)1 << 31) != 0x80000000u) return 201;
    if (((unsigned int)0x80000000u >> 31) != 1u) return 202;
    if ((-1 >> 31) != -1) return 203;

    // 3. Shift 64-bit types (0, 31, 32, 63)
    if (((unsigned long long)1 << 0) != 1ull) return 301;
    if (((unsigned long long)1 << 31) != 0x80000000ull) return 302;
    if (((unsigned long long)1 << 32) != 4294967296ull) return 303;
    if (((unsigned long long)1 << 63) != 0x8000000000000000ull) return 304;
    if (((unsigned long long)0x8000000000000000ull >> 63) != 1ull) return 305;
    if (((unsigned long long)4294967296ull >> 32) != 1ull) return 306;
    if (((long long)-1ll >> 63) != -1ll) return 307;

    // 4. Shift result types (sizeof check)
    if (sizeof((char)1 << 1) != sizeof(int)) return 401;
    if (sizeof((short)1 << (long long)1) != sizeof(int)) return 402;
    if (sizeof((unsigned int)1 << (short)1) != sizeof(unsigned int)) return 403;
    if (sizeof((long long)1 << (int)1) != sizeof(long long)) return 404;
    if (sizeof((unsigned long long)1 << (int)1) != sizeof(unsigned long long)) return 405;

    // 5. Compound shift assignments
    unsigned int u = 1u;
    u <<= 31;
    if (u != 0x80000000u) return 501;
    u >>= 31;
    if (u != 1u) return 502;
    int s = -1;
    s >>= 31;
    if (s != -1) return 503;

    unsigned long long u64 = 1ull;
    u64 <<= 63;
    if (u64 != 0x8000000000000000ull) return 504;
    u64 >>= 63;
    if (u64 != 1ull) return 505;
    long long s64 = -1ll;
    s64 >>= 63;
    if (s64 != -1ll) return 506;

    // 6. Unary Negation
    unsigned int u_val = 1u;
    unsigned int neg_u = -u_val;
    if (neg_u != 0xFFFFFFFFu) return 601;
    unsigned long v602 = (unsigned long)(-(unsigned int)1);
    if (v602 != 0xFFFFFFFFUL) return 602;

    unsigned long long u64_val = 1ull;
    unsigned long long neg_u64 = -u64_val;
    if (neg_u64 != 0xFFFFFFFFFFFFFFFFull) return 603;

    // 7. Unary Bitwise NOT
    unsigned int zero_u = 0u;
    unsigned int not_u = ~zero_u;
    if (not_u != 0xFFFFFFFFu) return 701;
    if ((unsigned long)(~(unsigned int)0) != 0xFFFFFFFFUL) return 702;

    unsigned long long zero_u64 = 0ull;
    unsigned long long not_u64 = ~zero_u64;
    if (not_u64 != 0xFFFFFFFFFFFFFFFFull) return 703;
    if (~0ull != 0xFFFFFFFFFFFFFFFFull) return 704;

    // 8. Unary Logical NOT
    if (!0 != 1) return 801;
    if (!1 != 0) return 802;
    if (!12345 != 0) return 803;
    if (!(-1) != 0) return 804;
    if (!0ull != 1) return 805;
    if (!0x8000000000000000ull != 0) return 806;

    return 0;
}
"#;
    let (code, _stdout, stderr) = compile_and_run(src, "test_shift_unary_stress");
    assert_eq!(code, 0, "Shift and unary stress test failed with exit code {code}, stdout: {_stdout}, stderr: {stderr}");
}

