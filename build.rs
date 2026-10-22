fn main() {
    println!("cargo:rerun-if-changed=native/WindowMaterial.m");
    println!("cargo:rerun-if-changed=native/CompletionSound.m");
    println!("cargo:rerun-if-changed=native/Updater.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/WindowMaterial.m")
            .file("native/CompletionSound.m")
            .flag("-fobjc-arc")
            .compile("four_quadrants_material");
        cc::Build::new()
            .file("native/Updater.m")
            .flag("-fobjc-arc")
            .compile("four_quadrants_updater");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=AVFAudio");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}
