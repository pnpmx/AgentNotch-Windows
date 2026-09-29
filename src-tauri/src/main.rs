// Release builds use the GUI subsystem so no console window appears. The
// Claude bridge still works: Claude Code pipes stdin/stdout explicitly.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|a| a == agentnotch_lib::BRIDGE_FLAG) {
        std::process::exit(agentnotch_lib::run_bridge());
    }
    agentnotch_lib::run();
}
