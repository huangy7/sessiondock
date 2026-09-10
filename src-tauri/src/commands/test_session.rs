use crate::error::{AppError, AppResult};
#[cfg(test)]
mod tests {
    use super::super::mod::*;
    use sessiondock_lib::cli::CliKind;

    #[test]
    fn test_scan_codex() {
        let res = sessiondock_lib::commands::scan_projects_paged(Some("codex".to_string()), 0, 50);
        println!("scan result: {:?}", res);
    }
}
