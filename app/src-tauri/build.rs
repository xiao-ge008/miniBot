fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_prebuilt_mac_recorder)");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    println!("cargo:rerun-if-changed=src/recorder_macos.swift");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        if let Ok(out_dir) = std::env::var("OUT_DIR") {
            let out_bin = std::path::Path::new(&out_dir).join("minibot-mac-recorder");
            let status = std::process::Command::new("swiftc")
                .args([
                    "-O",
                    "src/recorder_macos.swift",
                    "-framework",
                    "ScreenCaptureKit",
                    "-framework",
                    "AVFoundation",
                    "-framework",
                    "CoreGraphics",
                    "-framework",
                    "CoreImage",
                    "-framework",
                    "CoreMedia",
                    "-framework",
                    "Foundation",
                    "-o",
                ])
                .arg(&out_bin)
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("cargo:rustc-cfg=has_prebuilt_mac_recorder");
                }
                _ => {
                    // 若构建环境暂缺 swiftc，运行时会自动按需编译缓存
                    let _ = std::fs::write(&out_bin, b"");
                }
            }
        }
    }

    tauri_build::build()
}
