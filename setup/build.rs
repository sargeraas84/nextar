// Embeds the nextar application icon into the installer exe itself.
fn main() {
    println!("cargo:rerun-if-changed=nextar.rc");
    println!("cargo:rerun-if-changed=../resources/nextar.ico");
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        embed_resource::compile("nextar.rc", embed_resource::NONE);
    }
}
