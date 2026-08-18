use crate::app::{App, AppMode, CleanupRow};
use crate::cleanup_ui::CleanupTab;
use crate::modal::Modal;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Tabs},
    Frame,
};
use tui_piechart::{PieChart, PieSlice};

pub fn draw(f: &mut Frame, app: &mut App) {
    // Splash only until the first directory listing; then drop it so navigation works
    #[cfg(feature = "splash")]
    {
        if app.mode != AppMode::Cleanup {
            if let Some(ref mut splash_state) = app.splash_state {
                if app.tree.is_none() {
                    let _ = crate::splash::draw_splash(
                        f,
                        splash_state,
                        app.scan_files_count,
                        app.scanning_path.as_deref(),
                    );
                    return;
                }
                app.splash_state = None;
            }
        }
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(f.area());

    // Title bar
    draw_title(f, app, chunks[0]);

    // Main content area
    match app.mode {
        AppMode::Cleanup => draw_cleanup(f, app, chunks[1]),
        _ => draw_browser(f, app, chunks[1]),
    }

    // Help/status bar
    draw_footer(f, app, chunks[2]);

    // Notification if present
    if let Some(notif) = &app.notification {
        draw_notification(f, notif);
    }

    // Modal overlay if present
    if let Some(modal) = &app.modal {
        draw_modal(f, modal);
    }

    // Progress bar if deleting
    if let Some(progress) = &app.delete_progress {
        draw_progress(f, progress);
    }

    // Cleanup delete progress
    if let Some(progress) = &app.cleanup_delete_progress {
        draw_cleanup_progress(f, progress);
    }

    // Fullscreen loading only when we have nothing to browse yet
    if app.is_scanning && app.tree.is_none() {
        draw_loading(f, app.scan_files_count, app.scanning_path.as_deref());
    }

    // Cleanup scanning overlay
    if app.cleanup_scanning {
        let count = app
            .cleanup_scan_progress
            .as_ref()
            .map(|p| p.found_count as usize);
        draw_cleanup_loading(f, count);
    }

    // Help screen if shown
    if app.show_help {
        draw_help(f);
    }
}

fn draw_title(f: &mut Frame, app: &App, area: Rect) {
    let current_path = app.get_current_path();
    let title_text = format!(
        " 📊 mcdu v{} | {} ",
        env!("CARGO_PKG_VERSION"),
        current_path.display()
    );

    let right_text = if app.is_scanning {
        format!("  ⟳ Scanning... {} files ", app.scan_files_count)
    } else {
        let mut info = format!("  {} items", app.entries_count());

        // Add disk space if available
        if let Some(ref disk) = app.disk_space {
            let avail = format_size(disk.available_bytes);
            let total = format_size(disk.total_bytes);
            let percent_used = if disk.total_bytes == 0 {
                0
            } else {
                (disk.used_bytes as f64 / disk.total_bytes as f64 * 100.0) as u8
            };
            info.push_str(&format!(" | 💾 {}/{} ({}%)", avail, total, percent_used));
        }

        format!("{} ", info)
    };

    // Layout for title bar
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(30),
            Constraint::Length(right_text.len() as u16 + 2),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(title_text)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Left),
        chunks[0],
    );

    let right_style = if app.is_scanning {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    f.render_widget(
        Paragraph::new(right_text)
            .style(right_style)
            .alignment(Alignment::Right),
        chunks[1],
    );
}

fn draw_browser(f: &mut Frame, app: &mut App, area: Rect) {
    let mut lines = Vec::new();

    // Path display
    let current_path = app.get_current_path();
    lines.push(Line::from(format!("Path: {}", current_path.display())));
    lines.push(Line::from("".to_string()));

    let entries = app.get_display_entries();

    let viewport_height = area.height.saturating_sub(4) as usize;
    app.adjust_scroll(viewport_height);
    let start_idx = app.scroll_offset;
    let end_idx = (start_idx + viewport_height).min(entries.len());

    let total_size: u64 = entries
        .iter()
        .filter(|entry| entry.name != "..")
        .map(|entry| entry.size)
        .sum();

    let max_size = entries
        .iter()
        .filter(|entry| entry.name != "..")
        .map(|entry| entry.size)
        .max()
        .unwrap_or(1)
        .max(1);

    // Directory entries - only render visible items
    for (idx, entry) in entries
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(end_idx - start_idx)
    {
        let is_selected = idx == app.selected_index;
        let raw_size = format_size(entry.size);
        let size_str = if entry.is_dir && !entry.complete && entry.name != ".." {
            format!("~{}", raw_size)
        } else {
            raw_size
        };
        let percent_bar = if entry.size > 0 && entry.name != ".." {
            create_bar(entry.size, max_size)
        } else {
            String::new()
        };
        let percent_of_total = if total_size > 0 && entry.name != ".." {
            (entry.size as f64 / total_size as f64) * 100.0
        } else {
            0.0
        };
        let percent_str = format!("{:>4.0}%", percent_of_total.round());

        let size_color = get_color_by_size(entry.size);
        let name_prefix = if entry.is_dir { "📁 " } else { "📄 " };

        let name_style = if is_selected {
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let size_style = if entry.is_dir && !entry.complete && entry.name != ".." {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(size_color).add_modifier(Modifier::BOLD)
        };

        let line_spans = vec![
            Span::styled(
                format!("{}{:<25}", name_prefix, truncate_chars(&entry.name, 25)),
                name_style,
            ),
            Span::styled(format!("{:>10}", size_str), size_style),
            Span::styled(
                format!("{:>6}", percent_str),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(format!("  {} ", percent_bar)),
        ];

        lines.push(Line::from(line_spans));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Directory Contents");

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(35),
            Constraint::Percentage(25),
        ])
        .split(area);

    match app.mode {
        AppMode::Cleanup => {
            f.render_widget(
                Paragraph::new("[Tab] Tabs  [Space] Select  [d] Delete  [D] Dry-run")
                    .style(Style::default().fg(Color::Gray))
                    .alignment(Alignment::Left),
                chunks[0],
            );
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        "[q/Esc]",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" Back to browser", Style::default().fg(Color::Gray)),
                ]))
                .alignment(Alignment::Center),
                chunks[1],
            );
            f.render_widget(
                Paragraph::new("[?] Help")
                    .style(Style::default().fg(Color::Gray))
                    .alignment(Alignment::Right),
                chunks[2],
            );
        }
        _ => {
            f.render_widget(
                Paragraph::new("[↑↓] Navigate  [Enter] Open  [d] Delete  [?] Help")
                    .style(Style::default().fg(Color::Gray))
                    .alignment(Alignment::Left),
                chunks[0],
            );
            // Make cleanup discoverable — highlighted call-to-action
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        "[C]",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " Cleanup mode",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  (Shift+C)", Style::default().fg(Color::DarkGray)),
                ]))
                .alignment(Alignment::Center),
                chunks[1],
            );
            f.render_widget(
                Paragraph::new("[q] Quit")
                    .style(Style::default().fg(Color::Gray))
                    .alignment(Alignment::Right),
                chunks[2],
            );
        }
    }
}

fn draw_modal(f: &mut Frame, modal: &Modal) {
    let centered = centered_rect(60, 30, f.area());

    f.render_widget(Clear, centered);

    let title = modal.get_title();
    let message = modal.get_message();

    let mut button_spans = Vec::new();
    for (idx, (label, _)) in modal.buttons.iter().enumerate() {
        let button_style = if idx == modal.selected_button {
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(Color::DarkGray).fg(Color::White)
        };

        button_spans.push(Span::styled(format!(" {} ", label), button_style));

        if idx < modal.buttons.len() - 1 {
            button_spans.push(Span::raw("  "));
        }
    }

    let content = vec![
        Line::from(message),
        Line::from(""),
        Line::from(button_spans),
    ];

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    f.render_widget(
        Paragraph::new(content)
            .block(block)
            .style(Style::default().bg(Color::Black))
            .alignment(Alignment::Center),
        centered,
    );
}

fn draw_cleanup_loading(f: &mut Frame, count: Option<usize>) {
    let area = centered_rect(50, 7, f.area());
    f.render_widget(Clear, area);

    let loading_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("⟳ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                "Scanning for cleanup candidates...",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            match count {
                Some(c) => format!("{} candidates found", c),
                None => "Initializing...".to_string(),
            },
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Black));

    f.render_widget(
        Paragraph::new(loading_text)
            .block(block)
            .style(Style::default().bg(Color::Black))
            .alignment(Alignment::Center),
        area,
    );
}

fn draw_cleanup(f: &mut Frame, app: &App, area: Rect) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    draw_cleanup_tabs(f, app, layout[0]);

    match app.cleanup_active_tab {
        CleanupTab::Overview => draw_cleanup_overview(f, app, layout[1]),
        CleanupTab::Categories => draw_cleanup_categories(f, app, layout[1]),
        CleanupTab::Files => draw_cleanup_files(f, app, layout[1]),
        CleanupTab::Quarantine => draw_cleanup_quarantine(f, app, layout[1]),
    }

    let hints = Line::from(vec![
        Span::styled(
            " q",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Browse  "),
        Span::styled(
            "Tab",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Switch tab  "),
        Span::styled(
            "Space",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Select  "),
        Span::styled(
            "d",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Delete  "),
        Span::styled(
            "a/n",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" All/None  "),
        Span::styled(
            "C",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Rescan"),
    ]);
    f.render_widget(
        Paragraph::new(hints).style(Style::default().bg(Color::DarkGray)),
        layout[2],
    );
}

fn draw_cleanup_tabs(f: &mut Frame, app: &App, area: Rect) {
    let tab_titles = vec!["1 Overview", "2 Categories", "3 Files", "4 Quarantine"];
    let selected_idx = match app.cleanup_active_tab {
        CleanupTab::Overview => 0,
        CleanupTab::Categories => 1,
        CleanupTab::Files => 2,
        CleanupTab::Quarantine => 3,
    };

    let tabs = Tabs::new(tab_titles)
        .select(selected_idx)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" | ");

    f.render_widget(tabs, area);
}

fn category_color(idx: usize) -> Color {
    const COLORS: [Color; 12] = [
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
    ];
    COLORS[idx % COLORS.len()]
}

fn draw_cleanup_overview(f: &mut Frame, app: &App, area: Rect) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let total_size: u64 = app
        .cleanup_categories
        .iter()
        .flat_map(|c| c.candidates.iter())
        .map(|c| c.size_bytes)
        .sum();

    let selected_size: u64 = app
        .cleanup_categories
        .iter()
        .flat_map(|c| c.candidates.iter())
        .filter(|c| app.cleanup_selected.contains(&c.path))
        .map(|c| c.size_bytes)
        .sum();

    let total_items: usize = app
        .cleanup_categories
        .iter()
        .map(|c| c.candidates.len())
        .sum();

    let selected_items = app.cleanup_selected.len();

    let slices: Vec<PieSlice> = app
        .cleanup_categories
        .iter()
        .enumerate()
        .filter_map(|(idx, cat)| {
            let size: u64 = cat.candidates.iter().map(|c| c.size_bytes).sum();
            if size > 0 {
                Some(PieSlice::new(&cat.name, size as f64, category_color(idx)))
            } else {
                None
            }
        })
        .collect();

    if !slices.is_empty() {
        let chart = PieChart::new(slices)
            .show_legend(true)
            .show_percentages(true)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Disk Usage by Category"),
            );
        f.render_widget(chart, layout[0]);
    } else {
        let empty = Paragraph::new("No items found")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Disk Usage by Category"),
            )
            .alignment(Alignment::Center);
        f.render_widget(empty, layout[0]);
    }

    let stats_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("Total size:     "),
            Span::styled(format_size(total_size), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Selected:       "),
            Span::styled(
                format_size(selected_size),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("Categories:     "),
            Span::styled(
                format!("{}", app.cleanup_categories.len()),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::raw("Total items:    "),
            Span::styled(
                format!("{}", total_items),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::raw("Selected items: "),
            Span::styled(
                format!("{}", selected_items),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(""),
        Line::from(""),
        if total_items > 0 {
            let pct = (selected_items as f64 / total_items as f64 * 100.0) as u16;
            let bar_width = 20;
            let filled = (pct as usize * bar_width / 100).min(bar_width);
            Line::from(vec![
                Span::styled("▓".repeat(filled), Style::default().fg(Color::Green)),
                Span::styled(
                    "░".repeat(bar_width - filled),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(format!(" {}% selected", pct)),
            ])
        } else {
            Line::from("")
        },
    ];

    let stats = Paragraph::new(stats_lines)
        .block(Block::default().borders(Borders::ALL).title("Statistics"));
    f.render_widget(stats, layout[1]);
}

fn draw_cleanup_categories(f: &mut Frame, app: &App, area: Rect) {
    let rows = app.cleanup_rows();
    let mut lines = Vec::new();
    let viewport_height = area.height.saturating_sub(2) as usize;

    // Build all display lines with their source row index for cursor tracking
    let mut all_lines: Vec<(usize, Line)> = Vec::new();
    for (idx, row) in rows.iter().enumerate() {
        let cursor = idx == app.cleanup_selected_index;
        match row {
            CleanupRow::Category { name } => {
                let cat = app.cleanup_categories.iter().find(|c| &c.name == name);
                let (selected_count, total_count, total_size) = match cat {
                    Some(c) => (
                        c.candidates
                            .iter()
                            .filter(|cand| app.cleanup_selected.contains(&cand.path))
                            .count(),
                        c.candidates.len(),
                        c.candidates.iter().map(|c| c.size_bytes).sum::<u64>(),
                    ),
                    None => (0, 0, 0),
                };
                let checkbox = if total_count == 0 {
                    "[ ]"
                } else if selected_count == total_count {
                    "[x]"
                } else if selected_count > 0 {
                    "[-]"
                } else {
                    "[ ]"
                };
                let expanded = app.cleanup_expanded.contains(name);
                let arrow = if expanded { "▾" } else { "▸" };
                let size_str = format_size(total_size);
                let mut spans = vec![Span::raw(format!("{} {} {}", arrow, checkbox, name))];
                spans.push(Span::styled(
                    format!(" {:>8}", size_str),
                    Style::default().fg(Color::Green),
                ));
                let line = Line::from(spans).style(if cursor {
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                });
                all_lines.push((idx, line));
            }
            CleanupRow::Candidate {
                path,
                rule,
                pattern,
                size,
            } => {
                let selected = app.cleanup_selected.contains(path);
                let mark = if selected { "[x]" } else { "[ ]" };
                let size_str = format_size(*size);
                let display_path = path.display().to_string();
                let mut spans = vec![Span::raw(format!("  {} {}", mark, display_path))];
                spans.push(Span::styled(
                    format!(" {:>8}", size_str),
                    Style::default().fg(Color::Green),
                ));
                let line = Line::from(spans).style(if cursor {
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                });
                all_lines.push((idx, line));
                all_lines.push((
                    idx,
                    Line::from(Span::styled(
                        format!("      {} ({})", rule, pattern),
                        Style::default().fg(Color::DarkGray),
                    )),
                ));
            }
        }
    }

    // Scroll so the selected row's first line is visible (accounts for 2-line candidates)
    let cursor_line = all_lines
        .iter()
        .position(|(idx, _)| *idx == app.cleanup_selected_index)
        .unwrap_or(0);
    let start_line = cursor_line.saturating_sub(viewport_height / 2);
    let end_line = (start_line + viewport_height).min(all_lines.len());
    for (_, line) in all_lines
        .into_iter()
        .skip(start_line)
        .take(end_line - start_line)
    {
        lines.push(line);
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Categories (Space toggle, Enter expand, a all, n none, d delete, q back)");

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_cleanup_files(f: &mut Frame, app: &App, area: Rect) {
    let mut all_candidates: Vec<_> = app
        .cleanup_categories
        .iter()
        .flat_map(|cat| cat.candidates.iter().map(|c| (&cat.name, c)))
        .collect();

    match app.cleanup_files_sort {
        crate::cleanup_ui::FilesSortColumn::Size => {
            all_candidates.sort_by(|a, b| b.1.size_bytes.cmp(&a.1.size_bytes));
        }
        crate::cleanup_ui::FilesSortColumn::Name => {
            all_candidates.sort_by(|a, b| a.1.path.cmp(&b.1.path));
        }
        crate::cleanup_ui::FilesSortColumn::Category => {
            all_candidates.sort_by(|a, b| a.0.cmp(b.0));
        }
        crate::cleanup_ui::FilesSortColumn::Age => {
            all_candidates.sort_by(|a, b| a.1.last_accessed.cmp(&b.1.last_accessed));
        }
    }

    if app.cleanup_files_sort_desc {
        all_candidates.reverse();
    }

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Path", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("                                        "),
        Span::styled("Size", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("      "),
        Span::styled("Category", Style::default().add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(
        "─".repeat(area.width.saturating_sub(2) as usize),
    ));

    let viewport_height = area.height.saturating_sub(4) as usize;
    let start_idx = app.cleanup_files_scroll;

    for (idx, (cat_name, candidate)) in all_candidates
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(viewport_height)
    {
        let cursor = idx == app.cleanup_files_selected;
        let selected = app.cleanup_selected.contains(&candidate.path);
        let mark = if selected { "[x]" } else { "[ ]" };

        let path_str = candidate.path.display().to_string();

        let line = Line::from(vec![
            Span::raw(format!("{} ", mark)),
            Span::raw(format!("{} ", path_str)),
            Span::styled(
                format!("{:>8}", format_size(candidate.size_bytes)),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(cat_name.to_string(), Style::default().fg(Color::Yellow)),
        ])
        .style(if cursor {
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        });

        lines.push(line);
    }

    let sort_indicator = match app.cleanup_files_sort {
        crate::cleanup_ui::FilesSortColumn::Size => "size",
        crate::cleanup_ui::FilesSortColumn::Name => "name",
        crate::cleanup_ui::FilesSortColumn::Category => "category",
        crate::cleanup_ui::FilesSortColumn::Age => "age",
    };
    let sort_dir = if app.cleanup_files_sort_desc {
        "↓"
    } else {
        "↑"
    };

    let block = Block::default().borders(Borders::ALL).title(format!(
        "Files (s: sort by {}{}, Space: toggle, q: back)",
        sort_indicator, sort_dir
    ));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_cleanup_quarantine(f: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();

    if app.cleanup_quarantine_list.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "No quarantined items.",
            Style::default().fg(Color::Gray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Cleanup deletes are moved here and can be restored (r) or purged (p).",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (idx, manifest) in app.cleanup_quarantine_list.iter().enumerate() {
            let selected = idx == app.cleanup_quarantine_selected;
            let style = if selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let id_short: String = manifest.id.chars().take(8).collect();
            let size_mb = manifest.total_size_bytes as f64 / 1_048_576.0;
            lines.push(Line::from(Span::styled(
                format!(
                    " {}  {} item(s)  {:.1} MB  id:{}",
                    if selected { ">" } else { " " },
                    manifest.items.len(),
                    size_mb,
                    id_short
                ),
                style,
            )));
            if let Some(first) = manifest.items.first() {
                lines.push(Line::from(Span::styled(
                    format!("     {}", first.original_path.display()),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Quarantine (j/k: select, r: restore, p: purge)");

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_progress(f: &mut Frame, progress: &crate::app::DeleteProgress) {
    let centered = centered_rect(70, 40, f.area());

    f.render_widget(Clear, centered);

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(centered);

    f.render_widget(
        Paragraph::new(format!("🗑️  {}", progress.status)).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .bg(Color::Black),
        ),
        inner_layout[0],
    );

    let ratio = if progress.total_bytes > 0 {
        progress.deleted_bytes as f64 / progress.total_bytes as f64
    } else {
        0.0
    };

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Green))
        .ratio(ratio)
        .label(format!("{:.1}%", ratio * 100.0));

    f.render_widget(gauge, inner_layout[1]);

    let stats = format!(
        "Deleted: {} / {} ({} files)",
        format_size(progress.deleted_bytes),
        format_size(progress.total_bytes),
        progress.deleted_files
    );
    f.render_widget(
        Paragraph::new(stats).style(Style::default().bg(Color::Black)),
        inner_layout[2],
    );
}

fn draw_cleanup_progress(f: &mut Frame, progress: &mcdu_core::executor::CleanupProgress) {
    let centered = centered_rect(70, 40, f.area());

    f.render_widget(Clear, centered);

    let inner_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(centered);

    let stage_label = match progress.stage {
        mcdu_core::executor::CleanupStage::Files => "Files",
        mcdu_core::executor::CleanupStage::Git => "Git",
        mcdu_core::executor::CleanupStage::Command => "Command",
    };
    f.render_widget(
        Paragraph::new(format!(
            "Cleanup [{}]: {}",
            stage_label,
            progress.path.display()
        ))
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
                .bg(Color::Black),
        ),
        inner_layout[0],
    );

    let ratio = if progress.total > 0 {
        progress.current as f64 / progress.total as f64
    } else {
        0.0
    };
    f.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Cleanup Progress"),
            )
            .gauge_style(Style::default().fg(Color::Green))
            .ratio(ratio),
        inner_layout[1],
    );

    f.render_widget(
        Paragraph::new(format!("Freed {} bytes", progress.freed_bytes))
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center),
        inner_layout[2],
    );
}

fn draw_loading(f: &mut Frame, files_scanned: usize, scanning_path: Option<&str>) {
    let centered = centered_rect(70, 25, f.area());

    f.render_widget(Clear, centered);

    let mut loading_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("⟳ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                "Scanning directory tree...",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];

    // Show file count
    loading_text.push(Line::from(vec![Span::styled(
        format!("{} files scanned", files_scanned),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )]));

    loading_text.push(Line::from(""));

    // Show current path being scanned
    if let Some(path) = scanning_path {
        let max_width = (f.area().width as usize).saturating_sub(10).max(4);
        let truncated = truncate_path_end(path, max_width);

        loading_text.push(Line::from(vec![Span::styled(
            truncated,
            Style::default().fg(Color::Cyan),
        )]));
    } else {
        loading_text.push(Line::from(vec![Span::styled(
            "Initializing...",
            Style::default().fg(Color::Gray),
        )]));
    }

    loading_text.push(Line::from(""));
    loading_text.push(Line::from(vec![Span::styled(
        "Please wait",
        Style::default().fg(Color::Gray),
    )]));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Black));

    f.render_widget(
        Paragraph::new(loading_text)
            .block(block)
            .style(Style::default().bg(Color::Black))
            .alignment(Alignment::Center),
        centered,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Truncate a string to at most `max_chars` Unicode scalar values (not bytes).
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

/// Truncate from the start, keeping the end of a path (char-safe).
fn truncate_path_end(path: &str, max_width: usize) -> String {
    if max_width <= 3 {
        return "...".chars().take(max_width).collect();
    }
    let char_count = path.chars().count();
    if char_count <= max_width {
        return path.to_string();
    }
    let keep = max_width - 3;
    let skipped = char_count.saturating_sub(keep);
    format!("...{}", path.chars().skip(skipped).collect::<String>())
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.1} {}", size, UNITS[unit_idx])
}

fn get_color_by_size(size: u64) -> Color {
    match size {
        s if s > 100_000_000_000 => Color::Red,   // >100GB
        s if s > 10_000_000_000 => Color::Yellow, // >10GB
        s if s > 1_000_000_000 => Color::Cyan,    // >1GB
        _ => Color::Green,                        // <1GB
    }
}

fn create_bar(current: u64, max: u64) -> String {
    let max = max.max(1);
    let ratio = (current as f64 / max as f64).clamp(0.0, 1.0);
    let filled = (ratio * 10.0).round() as usize;
    let filled = filled.min(10);
    let empty = 10 - filled;
    format!("{}{}", "▓".repeat(filled), "░".repeat(empty))
}

fn draw_notification(f: &mut Frame, notif: &str) {
    let centered = centered_rect(60, 10, f.area());

    f.render_widget(Clear, centered);

    let notification_widget = Paragraph::new(notif)
        .block(Block::default().borders(Borders::ALL).title("Notification"))
        .alignment(Alignment::Center)
        .style(if notif.contains('✓') {
            Style::default().fg(Color::Green).bg(Color::Black)
        } else if notif.contains('✗') {
            Style::default().fg(Color::Red).bg(Color::Black)
        } else {
            Style::default().fg(Color::Cyan).bg(Color::Black)
        });

    f.render_widget(notification_widget, centered);
}

pub fn draw_help(f: &mut Frame) {
    let centered = centered_rect(80, 90, f.area());

    f.render_widget(Clear, centered);

    let help_text = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            "NAVIGATION",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  ↑ / k               Move cursor up"),
        Line::from("  ↓ / j               Move cursor down"),
        Line::from("  Enter / → / l       Enter directory"),
        Line::from("  Backspace / ← / h   Go to parent directory"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "DELETION",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]),
        Line::from("  d                   Delete selected file/directory"),
        Line::from("  y / n / d           Quick confirm in modals (yes/no/dry-run)"),
        Line::from("  ← / →               Navigate modal buttons (arrow keys)"),
        Line::from("  Enter               Confirm selected button"),
        Line::from("  Esc                 Close modal / leave cleanup (does not quit)"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "CLEANUP MODE",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  C (Shift+C)         Enter cleanup mode (developer cleanup)"),
        Line::from("  Tab / 1-4           Switch Overview/Categories/Files/Quarantine"),
        Line::from("  Space               Toggle selection"),
        Line::from("  a / n               Select all / none"),
        Line::from("  d / D               Delete selected / dry-run"),
        Line::from("  s                   Cycle Files-tab sort"),
        Line::from("  r / p               Restore / purge quarantine batch"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "GENERAL",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  r                   Rescan selected directory"),
        Line::from("  R / c               Rescan entire tree"),
        Line::from("  ?                   Show this help screen"),
        Line::from("  q                   Quit application"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "COLOR LEGEND",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled("  ", Style::default().bg(Color::Red)),
            Span::raw("  Red: >100 GB"),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default().bg(Color::Yellow)),
            Span::raw("  Yellow: 10-100 GB"),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default().bg(Color::Cyan)),
            Span::raw("  Cyan: 1-10 GB"),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default().bg(Color::Green)),
            Span::raw("  Green: <1 GB"),
        ]),
        Line::from(""),
        Line::from("Logs are saved to: ~/.mcdu/logs/"),
        Line::from(""),
        Line::from("Press any key to close this help screen..."),
    ];

    let help_widget = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(format!(" 🎯 HELP - mcdu v{} ", env!("CARGO_PKG_VERSION")))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .style(Style::default().bg(Color::Black))
        .alignment(Alignment::Left);

    f.render_widget(help_widget, centered);
}
