// Windowsのexeへ版情報(製品名・版)を埋め込む。コード署名(SignPath)の照合で
// 本体Hashi.exeと同じ製品名・版を求めるため。
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = tauri_winres::WindowsResource::new();
    res.set("ProductName", "Hashi")
        .set("FileDescription", "Hashi MCP helper")
        .set("CompanyName", "hashi")
        .set("InternalName", "HashiMCP-RustPreview")
        .set("OriginalFilename", "HashiMCP-RustPreview.exe");
    res.compile().expect("版情報リソースを埋め込めませんでした");
}
