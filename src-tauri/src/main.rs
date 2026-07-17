#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Run Cortana App
    cortana_lib::run();
}
