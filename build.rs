// Embeds the Windows application icon (resources/nextar.ico) into the exe.
fn main() {
    println!("cargo:rerun-if-changed=nextar.rc");
    println!("cargo:rerun-if-changed=resources/nextar.ico");
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        embed_resource::compile("nextar.rc", embed_resource::NONE);
    }
}
