//! Run a command, archive its output, and open it in the editor.

use anyhow::Result;
use sivtr_core::capture::subprocess;
use sivtr_core::config::SivtrConfig;
use sivtr_core::export::editor;
/// Execute a command, capture its output, archive it, and open the editor.
pub fn execute(command: &str, args: &[String]) -> Result<()> {
    let command_line = render_command_line(command, args);
    eprintln!("sivtr: running `{command_line}`");

    let result = subprocess::run_and_capture(command, args)?;

    match result.exit_code {
        Some(0) => eprintln!("sivtr: command exited successfully"),
        Some(code) => eprintln!("sivtr: command exited with code {code}"),
        None => eprintln!("sivtr: command was terminated by signal"),
    }

    if result.combined.is_empty() {
        eprintln!("sivtr: no output captured");
        return Ok(());
    }

    let config = SivtrConfig::load()?;
    sivtr_core::archive::store::insert_terminal_capture(
        Some(command_line.as_str()),
        &result.combined,
        &std::env::current_dir()?,
        result.exit_code,
    )?;

    let ed = editor::resolve_editor_with_config(&config)?;
    eprintln!("sivtr: opening in {ed}");
    editor::open_in_editor(&result.combined)?;
    Ok(())
}

fn render_command_line(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(quote_shell_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_shell_token(value: &str) -> String {
    if !value.is_empty() && value.chars().all(is_shell_safe_char) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\''"))
}

fn is_shell_safe_char(ch: char) -> bool {
    matches!(
        ch,
        'A'..='Z'
            | 'a'..='z'
            | '0'..='9'
            | '_'
            | '.'
            | '/'
            | ':'
            | '@'
            | '%'
            | '+'
            | '='
            | ','
            | '-'
    )
}

#[cfg(test)]
mod tests {
    use super::render_command_line;

    #[test]
    fn quotes_arguments_with_spaces() {
        let rendered =
            render_command_line("echo", &["hello world".to_string(), "plain".to_string()]);
        assert_eq!(rendered, "echo 'hello world' plain");
    }

    #[test]
    fn escapes_single_quotes_inside_arguments() {
        let rendered = render_command_line("printf", &["it's fine".to_string()]);
        assert_eq!(rendered, "printf 'it'\''s fine'");
    }
}
