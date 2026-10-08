#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "windows")]
    {
        let is_com_activation = std::env::args_os()
            .skip(1)
            .any(|argument| argument.eq_ignore_ascii_case("-Embedding"));
        if !is_com_activation {
            std::process::exit(1);
        }
        std::process::exit(mtt_explorer_command::run_local_server());
    }
    #[cfg(not(target_os = "windows"))]
    std::process::exit(1);
}
