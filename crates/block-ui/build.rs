fn main() {
    // rust-i18n reads these files during macro expansion.
    println!("cargo:rerun-if-changed=locales");
}
