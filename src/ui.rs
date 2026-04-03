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

    // Tooltip if hovered (only when context menu is not visible)
    if !app.context_menu.visible {
        if let Some(tooltip) = app.tooltip_text() {
            render_tooltip(f, &tooltip);
        }
    }

    // Context menu (rendered last so it's on top)
    if app.context_menu.visible {
        render_context_menu(f, app);
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

    // Calculate max radius from map (this is already scaled to fit)
    let max_radius = map
        .rings
        .last()
        .map(|r| r.outer_radius)
        .unwrap_or(map.center_radius);

    // Canvas area after borders
    let inner_width = (area.width.saturating_sub(2)) as f64;
    let inner_height = (area.height.saturating_sub(2)) as f64;

    // Braille resolution: 2 dots wide, 4 dots tall per cell
    let pixel_width = inner_width * 2.0;
    let pixel_height = inner_height * 4.0;

    // Calculate bounds that will fit the map exactly
    // We need the map to fit within the canvas, so bounds should match max_radius
    // But account for aspect ratio to keep it circular
    let aspect_ratio = pixel_height / pixel_width;

    // Set bounds - the map's max_radius should fill most of the canvas
    // Use max_radius as the x bound, and scale y to maintain aspect ratio
    let x_bound = max_radius;
    let y_bound = max_radius * aspect_ratio;

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
        .x_bounds([-x_bound, x_bound])
        .y_bounds([-y_bound, y_bound])
        .paint(|ctx| {
            // Draw center circle first (background)
            let center_clr = center_color(&app.renderer.config);
            ctx.draw(&crate::renderer::CenterShape {
                radius: map.center_radius,
                color: center_clr.to_ratatui(),
                center_x: 0.0,
                center_y: 0.0,
            });

            // Draw segments and strokes
            let (fill_shapes, stroke_shapes, circle_shapes) = app.renderer.render_shapes(map);

            for shape in fill_shapes {
                ctx.draw(&shape);
            }
            for shape in stroke_shapes {
                ctx.draw(&shape);
            }
            for shape in circle_shapes {
                ctx.draw(&shape);
            }
        });

    f.render_widget(canvas, area);

    // Overlay center text (folder name + total size)
    let center_text = format!(
        "{}\n{}",
        map.root_name,
        crate::tree::format_size(map.root_size)
    );

    // Calculate text area size based on center circle radius
    // The diagonal of the inscribed square = 2 * radius / sqrt(2)
    let text_width = (map.center_radius * 1.2) as u16;
    let text_height = 2;
    let text_width = text_width.max(8).min(area.width.saturating_sub(4));

    // Position text in center of canvas area
    let text_area = Rect {
        x: area.x + area.width / 2 - text_width / 2,
        y: area.y + area.height / 2 - text_height / 2,
        width: text_width,
        height: text_height,
    };

    let label = Paragraph::new(center_text)
        .style(Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 46)))
        .alignment(ratatui::layout::Alignment::Center);

    f.render_widget(label, text_area);
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

/// Render context menu popup
fn render_context_menu(f: &mut Frame, app: &App) {
    let menu = &app.context_menu;
    let items = menu.menu_items();
    let area = f.area();

    // Calculate menu dimensions
    let menu_width: u16 = 25;
    let menu_height: u16 = items.len() as u16 + 2; // +2 for borders

    // Position menu at cursor, but keep within screen bounds
    let menu_x = menu.x.min(area.width.saturating_sub(menu_width));
    let menu_y = menu.y.min(area.height.saturating_sub(menu_height));

    let menu_area = Rect {
        x: menu_x,
        y: menu_y,
        width: menu_width,
        height: menu_height,
    };

    // Build menu items
    let menu_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let style = if i == menu.selected_index || menu.hovered_index == Some(i) {
                // Yellow background for selected/hovered
                Style::default().fg(Color::White).bg(Color::Yellow)
            } else {
                // Normal
                Style::default().fg(Color::White)
            };
            let prefix = if i == menu.selected_index || menu.hovered_index == Some(i) {
                "> "
            } else {
                "  "
            };
            let content = format!("{}{}", prefix, action.label());
            ListItem::new(content).style(style)
        })
        .collect();

    let menu_widget = List::new(menu_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", menu.segment_name))
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .style(Style::default().bg(Color::DarkGray));

    f.render_widget(menu_widget, menu_area);
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
