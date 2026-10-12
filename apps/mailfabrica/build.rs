//! Windows PE 资源只在目标为 Windows 时生成；不在仓库保存 ICO 位图。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
/// 使用共享图标，并写入 Windows 可执行文件的产品和版本信息。
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return Ok(());
    }
    let output = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let icon = output.join("MailFabrica.ico");
    std::fs::write(&icon, mailfabrica_brand::ico()?)?;
    let path = icon.to_str().ok_or("icon path is not Unicode")?;
    winresource::WindowsResource::new()
        .set_icon(path)
        .set("ProductName", "MailFabrica")
        .set("FileDescription", "MailFabrica desktop mail client")
        .set("OriginalFilename", "mailfabrica.exe")
        .compile()?;
    Ok(())
}
