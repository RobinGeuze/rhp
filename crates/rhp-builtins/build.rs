#[path = "build/native.rs"]
mod native;

fn main() {
    println!("cargo:rerun-if-changed=build/native.rs");
    let linked = native::link_all();
    let libxml2 = linked
        .iter()
        .find(|l| l.name == "LIBXML2")
        .expect("native.rs links libxml2");
    // Macros and struct access the Rust side cannot do (see the file).
    println!("cargo:rerun-if-changed=src/ext/libxml/shim.c");
    cc::Build::new()
        .file("src/ext/libxml/shim.c")
        .include(&libxml2.include)
        .warnings(true)
        .extra_warnings(true)
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("rhp_libxml_shim");
}
