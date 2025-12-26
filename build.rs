use std::env;
use std::path::Path;

fn main() {
    // No external assembly build required when using `global_asm!`.
    println!("cargo:rerun-if-changed=build.rs");

    // embed_test_module feature가 활성화되면 모듈 파일을 OUT_DIR로 복사
    #[cfg(feature = "embed_test_module")]
    {
        let out_dir = env::var("OUT_DIR").unwrap();
        let target = env::var("TARGET").unwrap();

        // 아키텍처 결정
        let arch = if target.contains("aarch64") {
            "aarch64"
        } else if target.contains("riscv64") {
            "riscv64"
        } else {
            panic!("Unsupported target: {}", target);
        };

        // 모듈 소스 경로
        let module_src = format!("target/modules/{}/hello_module.ko", arch);
        let module_dst = Path::new(&out_dir).join("hello_module.ko");

        if Path::new(&module_src).exists() {
            std::fs::copy(&module_src, &module_dst).expect("Failed to copy module");
            println!("cargo:rerun-if-changed={}", module_src);
            println!("cargo:warning=Embedded test module from {}", module_src);
        } else {
            panic!(
                "Test module not found: {}. Build it first with: ./modules/hello/build.sh {}",
                module_src, arch
            );
        }
    }
}
