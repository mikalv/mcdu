mod app;
mod ui;
mod scan;
mod delete;
mod platform;
mod modal;
mod logger;
mod changes;
mod cache;

use app::App;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use ratatui::Terminal;
use std::error::Error;
use std::io;

fn main() -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.hide_cursor()?;

    // Run app
    let app = App::new();
    let result = run_app(&mut terminal, app);

    // Cleanup terminal - always restore state even on error
    let _ = terminal.show_cursor();
    let _ = terminal.clear();
    disable_raw_mode()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> Result<(), Box<dyn Error>> {
    loop {
        let viewport_height = terminal.size()?.height as usize;

        terminal.draw(|f| {
            ui::draw(f, &app);
        })?;

        // Adjust scroll to keep selected item visible
        app.adjust_scroll(viewport_height);

        if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if handle_input(&mut app, key)? {
                    break;
                }
            }
        }

        // Update scan progress if thread is running
        app.update_scan_progress();

        // Update delete progress if thread is running
        app.update_delete_progress();

        // Clear notification after 3 seconds
        if let Some(notif_time) = app.notification_time {
            if notif_time.elapsed().as_secs() > 3 {
                app.notification = None;
                app.notification_time = None;
            }
        }
    }

    Ok(())
}

fn handle_input(app: &mut App, key: KeyEvent) -> Result<bool, Box<dyn Error>> {
    // If help is shown, any key closes it
    if app.show_help {
        app.show_help = false;
        return Ok(false);
    }

    // If modal is open, handle modal input
    if app.modal.is_some() {
        return handle_modal_input(app, key);
    }

    // Normal file browser input
    match key.code {
        KeyCode::Char('q') => return Ok(true), // 'q' to quit
        KeyCode::Esc => {
            // Esc key closes modals if open, otherwise quits
            if app.modal.is_some() {
                app.modal = None;
            } else {
                return Ok(true);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => app.select_previous(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app.enter_directory(),
        KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => app.go_parent(),
        KeyCode::Char('d') => app.open_delete_modal(),
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Char('r') => app.refresh(),
        KeyCode::Char('c') => app.hard_refresh(), // 'c' to clear cache and refresh
        _ => {}
    }

    Ok(false)
}

fn handle_modal_input(app: &mut App, key: KeyEvent) -> Result<bool, Box<dyn Error>> {
    if let Some(modal) = &mut app.modal {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                if modal.selected_button > 0 {
                    modal.selected_button -= 1;
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if modal.selected_button < modal.buttons.len() - 1 {
                    modal.selected_button += 1;
                }
            }
            KeyCode::Tab => {
                modal.selected_button = (modal.selected_button + 1) % modal.buttons.len();
            }
            KeyCode::BackTab => {
                if modal.selected_button > 0 {
                    modal.selected_button -= 1;
                } else {
                    modal.selected_button = modal.buttons.len() - 1;
                }
            }
            KeyCode::Enter => {
                let action = modal.buttons[modal.selected_button].1.clone();
                return handle_modal_action(app, action);
            }
            KeyCode::Esc => {
                app.modal = None;
            }
            KeyCode::Char('y') if modal.has_button("Yes") => {
                return handle_modal_action(app, modal::ModalAction::Confirm);
            }
            KeyCode::Char('n') if modal.has_button("No") => {
                return handle_modal_action(app, modal::ModalAction::Cancel);
            }
            KeyCode::Char('d') if modal.has_button("Dry-run") => {
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
            if let Some(modal) = app.modal.take() {
                match modal.modal_type {
                    modal::ModalType::ConfirmDelete { path, size } => {
                        // Move to final confirmation
                        app.modal = Some(modal::Modal::final_confirm(&path, size));
                    }
                    modal::ModalType::FinalConfirm { path, size: _ } => {
                        // Start deletion
                        app.modal = None;
                        app.start_delete(&path)?;
                    }
                    #[allow(unreachable_patterns)]
                    _ => {}
                }
            }
        }
        modal::ModalAction::DryRun => {
            if let Some(modal) = app.modal.take() {
                if let modal::ModalType::ConfirmDelete { path, size: _ } = modal.modal_type {
                    app.start_dry_run(&path)?;
                }
            }
        }
        modal::ModalAction::Cancel => {
            app.modal = None;
        }
    }

    Ok(false)
}
