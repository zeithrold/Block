fn main() -> std::io::Result<()> {
    const ICON: &str = "../../packaging/windows/Block.ico";
    println!("cargo:rerun-if-changed={ICON}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon(ICON)
            .set("ProductName", "Block")
            .set("FileDescription", "Block Minecraft launcher")
            .set("OriginalFilename", "block.exe")
            .compile()?;
    }
    Ok(())
}
