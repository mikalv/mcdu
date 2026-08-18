use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use mcdu_core::devclean::{devclean, load_settings as load_devclean_settings};
use mcdu_tui::{app::App, app::AppMode, modal, ui};
use ratatui::prelude::*;
use ratatui::Terminal;
use std::error::Error;
use std::io::{self, Write};
use std::panic;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    /// Optional path to start in the TUI
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Developer cleanup utilities
    Cleanup(CleanupCommand),
    /// Quick non-interactive cleanup of build artifacts (alias: dc)
    #[command(alias = "dc")]
    Devclean(DevcleanCommand),
    /// Detect orphaned macOS app data (macOS only)
    Orphans(OrphansCommand),
}

#[derive(Parser)]
pub struct CleanupCommand {
    /// Path to scan for cleanup candidates
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,
}

#[derive(Parser)]
pub struct OrphansCommand {}

#[derive(Parser)]
pub struct DevcleanCommand {
    /// Path to clean (default: current directory)
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,
    /// List what would be removed, then exit (no deletion)
    #[arg(short = 'n', long)]
    dry_run: bool,
    /// Delete without the confirmation prompt
    #[arg(short = 'y', long)]
    yes: bool,
    /// Remove age-gated dirs (node_modules, deps, ...) regardless of age
    #[arg(long)]
    force_age: bool,
    /// Also remove release builds (target/release, _build/prod)
    #[arg(long)]
    all: bool,
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = stdout.flush();
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    // Non-interactive subcommand: no TUI involved
    if let Some(Commands::Devclean(cmd)) = &cli.command {
        run_devclean(cmd)?;
        return Ok(());
    }

    let orphan_mode = matches!(cli.command, Some(Commands::Orphans(_)));
    let (cleanup_mode, start_path) = match cli.command {
        Some(Commands::Cleanup(ref cmd)) => (true, validate_start_path(cmd.path.clone())?),
        Some(Commands::Orphans(_)) => (false, None),
        _ => (false, validate_start_path(cli.path)?),
    };

    // Orphans subcommand is macOS-only
    #[cfg(not(target_os = "macos"))]
    if orphan_mode {
        eprintln!("The 'orphans' subcommand is only available on macOS");
        std::process::exit(1);
    }

    // Restore terminal on panic
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    // Setup terminal with alternate screen (preserves scrollback)
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    // Run app
    let app = match (orphan_mode, &start_path, cleanup_mode) {
        #[cfg(target_os = "macos")]
        (true, _, _) => {
            let mut a = App::new_cleanup_mode();
            let _ = a.start_orphan_scan();
            a
        }
        (_, _, true) => {
            let mut a = App::new_cleanup_mode();
            let _ = a.start_cleanup_scan_with_path(start_path.clone());
            a
        }
        (_, Some(path), false) => App::new_with_root(path.clone()),
        (_, None, false) => App::new(),
    };

    let result = run_app(&mut terminal, app);

    // Cleanup terminal
    let _ = terminal.show_cursor();
    restore_terminal();

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn fmt_bytes(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

fn run_devclean(cmd: &DevcleanCommand) -> Result<(), Box<dyn Error>> {
    let root = cmd
        .path
        .clone()
        .map(|p| -> Result<PathBuf, Box<dyn Error>> {
            if !p.exists() {
                return Err(format!("Path does not exist: {}", p.display()).into());
            }
            if !p.is_dir() {
                return Err(format!("Path is not a directory: {}", p.display()).into());
            }
            Ok(p)
        })
        .transpose()?
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut settings = load_devclean_settings().map_err(|e| format!("{e}"))?;
    if cmd.force_age {
        settings.age_gate_node_modules = false;
    }
    if cmd.all {
        settings.keep_release = false;
    }

    let result = devclean(&root, &settings, true).map_err(|e| format!("{e}"))?;

    if result.removed.is_empty() {
        println!("Nothing to clean under {}", root.display());
        return Ok(());
    }

    for item in &result.removed {
        println!(
            "  {:>10}  {}",
            fmt_bytes(item.size_bytes),
            item.path.display()
        );
    }
    println!(
        "\n{} item(s), {} would be freed",
        result.removed.len(),
        fmt_bytes(result.freed_bytes)
    );
    for item in &result.kept {
        println!("  kept: {} ({})", item.path.display(), item.reason);
    }

    if cmd.dry_run {
        return Ok(());
    }

    if !cmd.yes {
        print!("\nDelete these directories? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("Aborted.");
            return Ok(());
        }
    }

    let result = devclean(&root, &settings, false).map_err(|e| format!("{e}"))?;
    for (path, err) in &result.errors {
        eprintln!("error: {}: {}", path.display(), err);
    }
    println!(
        "Removed {} item(s), freed {}",
        result.removed.len(),
        fmt_bytes(result.freed_bytes)
    );
    if !result.errors.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn validate_start_path(path: Option<PathBuf>) -> Result<Option<PathBuf>, Box<dyn Error>> {
    if let Some(path) = path {
        if !path.exists() {
            return Err(format!("Path does not exist: {}", path.display()).into());
        }
        if !path.is_dir() {
            return Err(format!("Path is not a directory: {}", path.display()).into());
        }
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> Result<(), Box<dyn Error>>
where
    B::Error: 'static,
{
    let mut needs_redraw = true;
    let mut last_activity = Instant::now();

    loop {
        if needs_redraw {
            terminal.draw(|f| {
                ui::draw(f, &mut app);
            })?;
            needs_redraw = false;
        }

        // Idle longer when quiet; poll faster while scanning/deleting
        let busy = app.is_scanning
            || app.cleanup_scanning
            || app.delete_thread.is_some()
            || app.cleanup_delete_thread.is_some()
            || app.notification_time.is_some();
        let poll_ms = if busy { 16 } else { 200 };

        // Wait for input, then drain the whole key queue so navigation stays snappy
        if crossterm::event::poll(Duration::from_millis(poll_ms))? {
            let mut should_quit = false;
            loop {
                match event::read()? {
                    Event::Key(key) if is_actionable_key(&key) => {
                        if handle_input(&mut app, key)? {
                            should_quit = true;
                            break;
                        }
                        needs_redraw = true;
                        last_activity = Instant::now();
                    }
                    _ => {}
                }
                if !crossterm::event::poll(Duration::ZERO)? {
                    break;
                }
            }
            if should_quit {
                break;
            }
        }

        if app.update_scan_progress() {
            needs_redraw = true;
        }
        app.update_delete_progress();
        app.update_cleanup_scan();
        app.update_cleanup_delete();
        if app.delete_progress.is_some() || app.cleanup_delete_progress.is_some() {
            needs_redraw = true;
        }
        if app.cleanup_scanning || app.cleanup_scan_progress.is_some() {
            needs_redraw = true;
        }

        // Clear notification after 3 seconds
        if let Some(notif_time) = app.notification_time {
            if notif_time.elapsed() > Duration::from_secs(3) {
                app.notification = None;
                app.notification_time = None;
                needs_redraw = true;
            }
        }

        // Splash animation needs continuous redraw
        #[cfg(feature = "splash")]
        if app.splash_state.is_some() {
            needs_redraw = true;
        }

        let _ = last_activity;
    }

    Ok(())
}

fn is_actionable_key(key: &KeyEvent) -> bool {
    match key.kind {
        KeyEventKind::Press => true,
        // Allow held arrow/hjkl navigation (kitty/Windows emit Repeat)
        KeyEventKind::Repeat => matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Char('j')
                | KeyCode::Char('k')
                | KeyCode::Char('h')
                | KeyCode::Char('l')
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
        ),
        KeyEventKind::Release => false,
    }
}

fn handle_input(app: &mut App, key: KeyEvent) -> Result<bool, Box<dyn Error>> {
    if app.modal.is_some() {
        return handle_modal_input(app, key);
    }

    match app.mode {
        AppMode::Cleanup => handle_cleanup_input(app, key),
        AppMode::Browsing | AppMode::DryRun => handle_browse_input(app, key),
        AppMode::Deleting => Ok(false),
    }
}

fn handle_browse_input(app: &mut App, key: KeyEvent) -> Result<bool, Box<dyn Error>> {
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        // Esc no longer quits from browser (easy mispress) — require q
        KeyCode::Esc => {
            if app.show_help {
                app.show_help = false;
            }
        }
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Up | KeyCode::Char('k') => app.select_previous(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app.enter_directory(),
        KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => app.go_parent(),
        KeyCode::Char('d') => app.open_delete_modal(),
        KeyCode::Char('r') => app.rescan_selected(),
        KeyCode::Char('R') | KeyCode::Char('c') => app.refresh(),
        KeyCode::Char('C') => {
            let _ = app.start_cleanup_scan();
        }
        _ => {}
    }

    Ok(false)
}

fn handle_cleanup_input(app: &mut App, key: KeyEvent) -> Result<bool, Box<dyn Error>> {
    use mcdu_tui::cleanup_ui::CleanupTab;

    match key.code {
        KeyCode::Char('q') => {
            app.mode = AppMode::Browsing;
        }
        KeyCode::Esc => {
            // Esc leaves cleanup back to browser, does not quit app
            app.mode = AppMode::Browsing;
        }
        KeyCode::Tab => {
            app.next_cleanup_tab();
        }
        KeyCode::BackTab => {
            app.prev_cleanup_tab();
        }
        KeyCode::Char('1') => app.set_cleanup_tab(CleanupTab::Overview),
        KeyCode::Char('2') => app.set_cleanup_tab(CleanupTab::Categories),
        KeyCode::Char('3') => app.set_cleanup_tab(CleanupTab::Files),
        KeyCode::Char('4') => app.set_cleanup_tab(CleanupTab::Quarantine),
        KeyCode::Up | KeyCode::Char('k') => {
            if app.cleanup_active_tab == CleanupTab::Quarantine {
                if app.cleanup_quarantine_selected > 0 {
                    app.cleanup_quarantine_selected -= 1;
                }
            } else if app.cleanup_active_tab == CleanupTab::Files {
                if app.cleanup_files_selected > 0 {
                    app.cleanup_files_selected -= 1;
                }
            } else {
                app.select_previous_cleanup();
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.cleanup_active_tab == CleanupTab::Quarantine {
                let max = app.cleanup_quarantine_list.len().saturating_sub(1);
                if app.cleanup_quarantine_selected < max {
                    app.cleanup_quarantine_selected += 1;
                }
            } else if app.cleanup_active_tab == CleanupTab::Files {
                let max = app.cleanup_candidates.len().saturating_sub(1);
                if app.cleanup_files_selected < max {
                    app.cleanup_files_selected += 1;
                }
            } else {
                app.select_next_cleanup();
            }
        }
        KeyCode::Char('s') if app.cleanup_active_tab == CleanupTab::Files => {
            app.cycle_files_sort();
        }
        KeyCode::Char('r') if app.cleanup_active_tab == CleanupTab::Quarantine => {
            app.restore_quarantine_selected();
        }
        KeyCode::Char('p') if app.cleanup_active_tab == CleanupTab::Quarantine => {
            app.purge_quarantine_selected();
        }
        KeyCode::Char(' ') => {
            if app.cleanup_active_tab == CleanupTab::Files {
                app.toggle_files_selection();
            } else {
                app.toggle_cleanup_selection();
            }
        }
        KeyCode::Enter => app.toggle_cleanup_expand(),
        KeyCode::Char('a') => app.select_all_cleanup(),
        KeyCode::Char('n') => app.select_none_cleanup(),
        KeyCode::Char('d') => app.start_cleanup_delete(),
        KeyCode::Char('D') => app.start_cleanup_dry_run(),
        KeyCode::Char('C') => {
            let _ = app.start_cleanup_scan();
        }
        _ => {}
    }

    Ok(false)
}

fn handle_modal_input(app: &mut App, key: KeyEvent) -> Result<bool, Box<dyn Error>> {
    if let Some(modal_ref) = &mut app.modal {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                if modal_ref.selected_button > 0 {
                    modal_ref.selected_button -= 1;
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if modal_ref.selected_button < modal_ref.buttons.len() - 1 {
                    modal_ref.selected_button += 1;
                }
            }
            KeyCode::Tab => {
                modal_ref.selected_button =
                    (modal_ref.selected_button + 1) % modal_ref.buttons.len();
            }
            KeyCode::BackTab => {
                if modal_ref.selected_button > 0 {
                    modal_ref.selected_button -= 1;
                } else {
                    modal_ref.selected_button = modal_ref.buttons.len() - 1;
                }
            }
            KeyCode::Enter => {
                let action = modal_ref.buttons[modal_ref.selected_button].1.clone();
                return handle_modal_action(app, action);
            }
            KeyCode::Esc => {
                app.modal = None;
                app.cleanup_pending = None;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                return handle_modal_action(app, modal::ModalAction::Confirm);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                return handle_modal_action(app, modal::ModalAction::Cancel);
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                return handle_modal_action(app, modal::ModalAction::DryRun);
            }
            _ => {}
        }
    }

    Ok(false)
}

fn handle_modal_action(app: &mut App, action: modal::ModalAction) -> Result<bool, Box<dyn Error>> {
    match action {
        modal::ModalAction::Confirm => {
            if let Some(modal_instance) = app.modal.take() {
                match modal_instance.modal_type {
                    modal::ModalType::ConfirmDelete { path, size } => {
                        app.modal = Some(modal::Modal::final_confirm(&path, size));
                    }
                    modal::ModalType::FinalConfirm { path, size: _ } => {
                        app.modal = None;
                        app.start_delete(&path)?;
                    }
                    modal::ModalType::CleanupConfirm { dry_run, .. } => {
                        app.handle_cleanup_modal_confirm(true);
                        if dry_run {
                            return Ok(false);
                        }
                    }
                    modal::ModalType::CleanupFinal { .. } => {
                        app.handle_cleanup_final_confirm_with_git(true, false);
                    }
                    #[allow(unreachable_patterns)]
                    _ => {}
                }
            }
        }
        modal::ModalAction::ConfirmWithGit => {
            if let Some(modal_instance) = app.modal.take() {
                if matches!(
                    modal_instance.modal_type,
                    modal::ModalType::CleanupFinal { .. }
                ) {
                    app.handle_cleanup_final_confirm_with_git(true, true);
                }
            }
        }
        modal::ModalAction::DryRun => {
            if let Some(modal_instance) = app.modal.take() {
                match modal_instance.modal_type {
                    modal::ModalType::ConfirmDelete { path, size: _ } => {
                        app.start_dry_run(&path)?;
                    }
                    modal::ModalType::CleanupConfirm { .. } => {
                        app.handle_cleanup_modal_confirm(true);
                    }
                    _ => {}
                }
            }
        }
        modal::ModalAction::Cancel => {
            app.modal = None;
            app.cleanup_pending = None;
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validate_start_path_accepts_existing_dir() {
        let tmp = tempdir().unwrap();
        let result = validate_start_path(Some(tmp.path().to_path_buf()));
        assert!(result.is_ok());
    }

    #[test]
    fn validate_start_path_rejects_missing() {
        let missing = PathBuf::from("/path/does/not/exist");
        let result = validate_start_path(Some(missing));
        assert!(result.is_err());
    }
}
