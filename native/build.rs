fn main() {
    // Dark widget style so std-widgets (SpinBox/CheckBox/ComboBox/ScrollView)
    // match the app's dark theme instead of the light default.
    let cfg = slint_build::CompilerConfiguration::new().with_style("fluent-dark".to_string());
    slint_build::compile_with_config("ui/app.slint", cfg).unwrap();
}
