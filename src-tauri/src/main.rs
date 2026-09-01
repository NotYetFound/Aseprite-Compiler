#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(pos) = args.iter().position(|a| a == "--run-aseprite") {
        // Launcher shim mode: start Aseprite + background update check,
        // without bringing up the GUI.
        aseprite_compiler_lib::run_shim(args[pos + 1..].to_vec());
        return;
    }
    aseprite_compiler_lib::run()
}
