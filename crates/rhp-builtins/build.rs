#[path = "build/native.rs"]
mod native;

fn main() {
    println!("cargo:rerun-if-changed=build/native.rs");
    native::link_all();
}
