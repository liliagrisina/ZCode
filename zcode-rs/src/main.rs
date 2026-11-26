fn main() {
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("zcode-rs {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    zcode_rs::ui::run();
}
