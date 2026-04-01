use crate::app::{App, AppMode, Focus};
use crate::color::center_color;
use crate::renderer::ArcShape;
use crate::tree::{format_size, TreeItem};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// Main render function
pub fn render(f: &mut Frame, app: &App) {
    match app.mode {
        AppMode::Scanning => render_scanning(f, app),
        AppMode::Viewing => render_viewing(f, app),
        AppMode::Help => render_help(f, app),
    }
}

/// Render scanning mode
fn render_scanning(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(f.area());

    // Progress message
    let progress_text = if let Some(ref progress) = app.scan_progress {
        format!(
            "Scanning {}...\n{} files ({})",
            app.current_path.display(),
            progress.files_scanned,
            format_size(progress.total_size)
        )
    } else {
        format!("Scanning {}...", app.current_path.display())
    };

    let progress = Paragraph::new(progress_text)
        .block(Block::default().borders(Borders::ALL).title("Radisk"))
        .style(Style::default().fg(Color::Cyan))
        .wrap(Wrap { trim: true });
    f.render_widget(progress, chunks[0]);

    // Status bar
    let status = Paragraph::new("Press ESC or 'q' to quit")
        .block(Block::default().borders(Borders::TOP))
        .style(Style::default().fg(Color::Gray));
    f.render_widget(status, chunks[1]);
}

/// Render viewing mode
fn render_viewing(f: &mut Frame, app: &App) {
    let area = f.area();

    // Main layout: sidebar | map
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    // Sidebar
    render_sidebar(f, app, main_chunks[0]);

    // Map area with status bar
    let map_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(main_chunks[1]);

    // Radial map
    render_radial_map(f, app, map_chunks[0]);

    // Status bar
    render_status_bar(f, app, map_chunks[1]);

    // Tooltip if hovered
    if let Some(tooltip) = app.tooltip_text() {
        render_tooltip(f, &tooltip);
    }
}

/// Render sidebar with file list
fn render_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .sidebar_items()
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let (icon, name, size_str, style) = match item {
                TreeItem::File(id, s) => {
                    let name = app
                        .arena
                        .as_ref()
                        .map(|a| a.file(*id).name.clone())
                        .unwrap_or_else(|| "?".to_string());
                    let mut style = Style::default().fg(Color::White);
                    if i == app.sidebar_index && app.focus == Focus::Sidebar {
                        style = style.bg(Color::DarkGray);
                    }
                    if app.sidebar_hover_index == Some(i) {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    (" ", name, format_size(*s), style)
                }
                TreeItem::Folder(id, s) => {
                    let name = app
                        .arena
                        .as_ref()
                        .map(|a| a.folder(*id).file.name.clone())
                        .unwrap_or_else(|| "?".to_string());
                    let mut style = Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD);
                    if i == app.sidebar_index && app.focus == Focus::Sidebar {
                        style = style.bg(Color::DarkGray);
                    }
                    if app.sidebar_hover_index == Some(i) {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }
                    ("[D]", name, format_size(*s), style)
                }
            };
            let content = format!("{} {} ({})", icon, name, size_str);
            ListItem::new(content).style(style)
        })
        .collect();

    let title = app
        .current_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());

    let sidebar = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", title))
                .border_style(if app.focus == Focus::Sidebar {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        )
        .style(Style::default().fg(Color::White));

    f.render_widget(sidebar, area);
}

/// Render radial map using canvas
fn render_radial_map(f: &mut Frame, app: &App, area: Rect) {
    use ratatui::symbols::Marker;
    use ratatui::widgets::canvas::Canvas;

    if app.radial_map.is_none() {
        let placeholder =
            Paragraph::new("No data").block(Block::default().borders(Borders::ALL).title("Map"));
        f.render_widget(placeholder, area);
        return;
    }

    let map = app.radial_map.as_ref().unwrap();

    // Calculate max radius from map
    let max_radius = map
        .rings
        .last()
        .map(|r| r.outer_radius)
        .unwrap_or(map.center_radius);

    // Set bounds with some padding
    let bounds = max_radius * 1.2;

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", app.current_path.display()))
                .border_style(if app.focus == Focus::Map {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        )
        .marker(Marker::Braille)
        .x_bounds([-bounds, bounds])
        .y_bounds([-bounds, bounds])
        .paint(|ctx| {
            // Draw center circle first (background)
            let center_clr = center_color(&app.renderer.config);
            ctx.draw(&crate::renderer::CenterShape {
                radius: map.center_radius,
                color: center_clr.to_ratatui(),
                center_x: 0.0,
                center_y: 0.0,
            });

            // Draw segments from innermost to outermost
            for ring in &map.rings {
                for segment in &ring.segments {
                    let colors = app.renderer.get_segment_colors(segment, ring.depth);

                    ctx.draw(&ArcShape {
                        start_angle: segment.start_degrees(),
                        sweep_angle: segment.sweep_degrees(),
                        inner_radius: ring.inner_radius,
                        outer_radius: ring.outer_radius,
                        color: colors.fill.to_ratatui(),
                        center_x: 0.0,
                        center_y: 0.0,
                    });
                }
            }
        });

    f.render_widget(canvas, area);
}

/// Render status bar
fn render_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let help_text = if app.hovered_uuid.is_some() {
        "[u/Backspace] Up  [Enter] Open  [+/-] Zoom  [r] Rescan  [?] Help  [q] Quit"
    } else {
        "[u/Backspace] Up  [+/-] Zoom  [r] Rescan  [Tab] Focus  [?] Help  [q] Quit"
    };

    let status_line = Line::from(vec![
        Span::styled(app.status_text(), Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled(help_text, Style::default().fg(Color::DarkGray)),
    ]);

    let status = Paragraph::new(status_line).block(Block::default().borders(Borders::TOP));
    f.render_widget(status, area);
}

/// Render tooltip near cursor
fn render_tooltip(f: &mut Frame, text: &str) {
    let area = f.area();

    // Position tooltip in top-right area
    let tooltip_width = 30.min(area.width / 3);
    let tooltip_height = text.lines().count() as u16 + 2;
    let tooltip_area = Rect {
        x: area.width.saturating_sub(tooltip_width + 2),
        y: 1,
        width: tooltip_width,
        height: tooltip_height,
    };

    let tooltip = Paragraph::new(text.to_string())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .style(Style::default().fg(Color::White).bg(Color::DarkGray))
        .wrap(Wrap { trim: true });

    f.render_widget(tooltip, tooltip_area);
}

/// Render help overlay
fn render_help(f: &mut Frame, app: &App) {
    let area = f.area();

    // First render the main view behind
    render_viewing(f, app);

    // Then overlay help
    let help_area = centered_rect(60, 70, area);

    let help_text = vec![
        Line::from(Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  q/Esc      ", Style::default().fg(Color::Yellow)),
            Span::raw("Quit"),
        ]),
        Line::from(vec![
            Span::styled("  u/Backspace", Style::default().fg(Color::Yellow)),
            Span::raw("Go to parent directory"),
        ]),
        Line::from(vec![
            Span::styled("  Enter      ", Style::default().fg(Color::Yellow)),
            Span::raw("Open selected folder"),
        ]),
        Line::from(vec![
            Span::styled("  +/-/=      ", Style::default().fg(Color::Yellow)),
            Span::raw("Zoom in/out (change ring depth)"),
        ]),
        Line::from(vec![
            Span::styled("  r          ", Style::default().fg(Color::Yellow)),
            Span::raw("Rescan directory"),
        ]),
        Line::from(vec![
            Span::styled("  Tab        ", Style::default().fg(Color::Yellow)),
            Span::raw("Toggle focus (map/sidebar)"),
        ]),
        Line::from(vec![
            Span::styled("  j/k        ", Style::default().fg(Color::Yellow)),
            Span::raw("Navigate up/down in sidebar"),
        ]),
        Line::from(vec![
            Span::styled("  ?          ", Style::default().fg(Color::Yellow)),
            Span::raw("Show this help"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Mouse",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Left click ", Style::default().fg(Color::Yellow)),
            Span::raw("Open folder / Go up (center)"),
        ]),
        Line::from(vec![
            Span::styled("  Scroll     ", Style::default().fg(Color::Yellow)),
            Span::raw("Zoom in/out"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help ")
                .border_style(Style::default().fg(Color::Green)),
        )
        .style(Style::default().bg(Color::Black));

    f.render_widget(help, help_area);
}

/// Create a centered rectangle
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
