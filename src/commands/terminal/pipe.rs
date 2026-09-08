//! Read stdin, archive it, and open it in the editor.

use anyhow::Result;
use sivtr_core::archive::store::insert_terminal_capture;
use sivtr_core::capture::pipe::read_stdin;
use sivtr_core::config::SivtrConfig;
use sivtr_core::export::editor;

/// Pipe mode: read stdin, archive it, and open the external editor.
pub fn execute() -> Result<()> {
    let raw = read_stdin()?;

    if raw.is_empty() {
        eprintln!("sivtr: no input received from stdin");
        return Ok(());
    }

    let config = SivtrConfig::load()?;
    insert_terminal_capture(None, &raw, &std::env::current_dir()?, None)?;

    let ed = editor::resolve_editor_with_config(&config)?;
    eprintln!("sivtr: opening in {ed}");
    editor::open_in_editor(&raw)?;
    Ok(())
}
