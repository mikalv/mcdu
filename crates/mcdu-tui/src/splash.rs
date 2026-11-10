//! Animated splash screen shown during the initial directory scan.
//!
//! Uses tachyonfx for post-render effects and tui-big-text for a large "MCDU" logo.
//! This module is only compiled when the `splash` feature is enabled.

use ratatui::prelude::*;
use ratatui::widgets::{Clear, Paragraph};
use std::time::Instant;
use tachyonfx::{fx, Effect, Interpolation, Motion};
use tui_big_text::{BigText, PixelSize};

/// Tracks splash animation state across frames.
pub struct SplashState {
    start: Instant,
    last_frame: Instant,
    logo_effect: Effect,
    tagline_effect: Effect,
    fadeout_effect: Option<Effect>,
    fadeout_started: bool,
}

impl Default for SplashState {
    fn default() -> Self {
        Self::new()
    }
}

impl SplashState {
    pub fn new() -> Self {
        let now = Instant::now();

        // Logo: sweep in from left over 800ms with a smooth ease-out
        let logo_effect = fx::sweep_in(
            Motion::LeftToRight,
            10,
            2,
            Color::Black,
            (800, Interpolation::CubicOut),
        );

        // Tagline: fade from black starting after a delay (handled via timing)
        let tagline_effect =
            fx::fade_from(Color::Black, Color::Black, (800, Interpolation::SineOut));

        Self {
            start: now,
            last_frame: now,
            logo_effect,
            tagline_effect,
            fadeout_effect: None,
            fadeout_started: false,
        }
    }

    /// Begin the exit fade-out animation (called when the scan completes).
    pub fn start_fadeout(&mut self) {
        if !self.fadeout_started {
            self.fadeout_started = true;
            self.fadeout_effect = Some(fx::fade_to(
                Color::Black,
                Color::Black,
                (300, Interpolation::CubicIn),
            ));
        }
    }

    /// Returns true when the fadeout animation has finished.
    pub fn is_done(&self) -> bool {
        self.fadeout_effect
            .as_ref()
            .map(|e| e.done())
            .unwrap_or(false)
    }
}

/// Draw the splash screen and process animations.
///
/// Returns `true` if the splash animation is completely finished
/// (fadeout done) and the caller should switch to the normal view.
pub fn draw_splash(
    f: &mut Frame,
    state: &mut SplashState,
    files_scanned: usize,
    scanning_path: Option<&str>,
) -> bool {
    let elapsed_ms = state.start.elapsed().as_millis();
    let frame_delta: tachyonfx::Duration = state.last_frame.elapsed().into();
    state.last_frame = Instant::now();

    let area = f.area();

    // Clear the entire screen for the splash
    f.render_widget(Clear, area);

    // Calculate centered layout: logo area + tagline + progress
    let logo_height: u16 = 5; // BigText with HalfHeight pixel size
    let tagline_height: u16 = 1;
    let progress_height: u16 = 3;
    let spacing: u16 = 1;
    let total_content_height = logo_height + spacing + tagline_height + spacing + progress_height;

    let vertical_padding = area.height.saturating_sub(total_content_height) / 2;
    let horizontal_padding = area.width.saturating_sub(40) / 2;

    let content_area = Rect {
        x: area.x + horizontal_padding.min(area.width),
        y: area.y + vertical_padding.min(area.height),
        width: area.width.saturating_sub(horizontal_padding * 2),
        height: total_content_height.min(area.height.saturating_sub(vertical_padding)),
    };

    // Split content area into logo / tagline / progress
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(logo_height),
            Constraint::Length(spacing),
            Constraint::Length(tagline_height),
            Constraint::Length(spacing),
            Constraint::Length(progress_height),
        ])
        .split(content_area);

    let logo_area = chunks[0];
    let tagline_area = chunks[2];
    let progress_area = chunks[4];

    // --- Render the BigText logo ---
    let big_text = BigText::builder()
        .pixel_size(PixelSize::HalfHeight)
        .style(Style::new().cyan().bold())
        .lines(vec!["MCDU".into()])
        .centered()
        .build();
    f.render_widget(big_text, logo_area);

    // Apply sweep_in effect to logo area
    state
        .logo_effect
        .process(frame_delta, f.buffer_mut(), logo_area);

    // --- Render the tagline (only after 400ms) ---
    if elapsed_ms >= 400 {
        let tagline = Paragraph::new(Line::from(vec![Span::styled(
            "Modern Disk Usage Analyzer",
            Style::default().fg(Color::DarkGray).italic(),
        )]))
        .alignment(Alignment::Center);
        f.render_widget(tagline, tagline_area);

        // Apply fade effect to tagline
        state
            .tagline_effect
            .process(frame_delta, f.buffer_mut(), tagline_area);
    }

    // --- Render live progress (no effects, always readable) ---
    let file_count_text = format!("  Scanning... {:>7} files", files_scanned);
    let max_width = progress_area.width.saturating_sub(4) as usize;
    let path_text = match scanning_path {
        // Char-safe truncation via `truncate_path_end`; the two-space indent
        // shrinks the path budget so the whole line fits `max_width`.
        Some(path) => format!(
            "  {}",
            crate::ui::truncate_path_end(path, max_width.saturating_sub(2))
        ),
        None => "  Initializing...".to_string(),
    };

    let progress_lines = vec![
        Line::from(Span::styled(
            file_count_text,
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            path_text,
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(progress_lines), progress_area);

    // --- Apply full-screen fadeout if active ---
    if let Some(ref mut fadeout) = state.fadeout_effect {
        fadeout.process(frame_delta, f.buffer_mut(), area);
    }

    state.is_done()
}
