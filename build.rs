fn main() {
    // Fluent is Slint's Windows-like widget style.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint build failed");
}
