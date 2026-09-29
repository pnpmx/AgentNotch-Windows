// Release builds use the GUI subsystem so no console window appears. The
// Claude bridge still works: Claude Code pipes stdin/stdout explicitly.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|a| a == agentnotch_lib::BRIDGE_FLAG) {
        std::process::exit(agentnotch_lib::run_bridge());
    }
    #[cfg(target_os = "linux")]
    agentnotch_lib::prepare_environment();
    agentnotch_lib::run();
}
