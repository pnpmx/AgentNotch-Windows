// Release builds use the GUI subsystem so no console window appears. The
// Claude bridge still works: Claude Code pipes stdin/stdout explicitly.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == agentnotch_lib::BRIDGE_FLAG) {
        std::process::exit(agentnotch_lib::run_bridge());
    }
    // Agent hooks run this binary briefly; never start the widget for them.
    if args.iter().any(|a| a == agentnotch_lib::EVENT_FLAG) {
        std::process::exit(agentnotch_lib::run_agent_event(&args));
    }
    #[cfg(target_os = "linux")]
    agentnotch_lib::prepare_environment();
    agentnotch_lib::run();
}
