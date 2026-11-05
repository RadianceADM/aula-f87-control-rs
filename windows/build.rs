fn main() {
    // Stage the Windows App Runtime next to the exe (self-contained, no install).
    windows_reactor_setup::as_self_contained();
    // The exe's icon and version info.
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/aula-f87-control.ico");
    embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional().unwrap();
}
