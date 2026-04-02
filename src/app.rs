use crate::color::ColorConfig;
use crate::context_menu::{ContextMenu, MenuAction};
use crate::radial::{build_radial_map, RadialConfig, RadialMap, Segment};
use crate::renderer::{CanvasCoords, RadialRenderer};
use crate::scanner::{self, ScanConfig, ScanProgress};
use crate::tree::{format_size, FolderId, TreeArena, TreeItem};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use uuid::Uuid;

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppMode {
    Scanning,
    Viewing,
    Help,
}

/// Application focus
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Map,
    Sidebar,
}

/// Navigation history entry
#[derive(Debug, Clone)]
pub struct NavEntry {
    pub path: PathBuf,
    pub root_id: FolderId,
}

/// Application state
pub struct App {
    pub mode: AppMode,
    pub focus: Focus,
    pub arena: Option<TreeArena>,
    pub current_path: PathBuf,
    pub ring_depth: usize,
    pub radial_map: Option<RadialMap>,
    pub renderer: RadialRenderer,
    pub hovered_uuid: Option<Uuid>,
    pub selected_uuid: Option<Uuid>,
    pub sidebar_index: usize,
    pub sidebar_hover_index: Option<usize>,
    pub terminal_size: (u16, u16),
    pub should_quit: bool,
    pub scan_progress: Option<ScanProgress>,
    pub scan_rx: Option<mpsc::Receiver<ScanProgress>>,
    pub nav_history: Vec<PathBuf>,
    pub status_message: String,
    pub canvas_coords: Option<CanvasCoords>,
    pub context_menu: ContextMenu,
}

impl App {
    pub fn new(path: PathBuf, ring_depth: usize, term_width: u16, term_height: u16) -> Self {
        Self {
            mode: AppMode::Scanning,
            focus: Focus::Map,
            arena: None,
            current_path: path,
            ring_depth,
            radial_map: None,
            renderer: RadialRenderer::new(ColorConfig::default()),
            hovered_uuid: None,
            selected_uuid: None,
            sidebar_index: 0,
            sidebar_hover_index: None,
            terminal_size: (term_width, term_height),
            should_quit: false,
            scan_progress: None,
            scan_rx: None,
            nav_history: Vec::new(),
            status_message: String::new(),
            canvas_coords: None,
            context_menu: ContextMenu::new(),
        }
    }

    /// Start scanning the current path
    pub fn start_scan(&mut self) {
        let path = self.current_path.clone();
        let (tx, rx) = mpsc::channel();

        self.mode = AppMode::Scanning;
        self.scan_rx = Some(rx);
        self.scan_progress = Some(ScanProgress {
            files_scanned: 0,
            total_size: 0,
        });

        // Spawn scan thread
        thread::spawn(move || {
            let config = ScanConfig::default();
            match scanner::scan_with_progress(&path, &config, Some(tx.clone())) {
                Ok(arena) => {
                    // Send final progress
                    let root_id = arena.root().unwrap();
                    let _ = tx.send(ScanProgress {
                        files_scanned: arena.total_file_count(root_id),
                        total_size: arena.folder(root_id).file.size,
                    });
                    // Note: We can't send the arena through the channel easily
                    // Instead, we'll do a synchronous scan after progress updates
                }
                Err(e) => {
                    eprintln!("Scan error: {}", e);
                }
            }
        });

        // Also do a synchronous scan for the actual data
        self.scan_sync();
    }

    /// Synchronous scan (for actual data)
    fn scan_sync(&mut self) {
        let config = ScanConfig::default();
        match scanner::scan_directory(&self.current_path, &config) {
            Ok(arena) => {
                let root_id = arena.root().unwrap();
                self.arena = Some(arena);
                self.mode = AppMode::Viewing;
                self.rebuild_map();
                self.status_message = format!(
                    "Scanned {} files ({})",
                    self.arena.as_ref().unwrap().total_file_count(root_id),
                    format_size(self.arena.as_ref().unwrap().folder(root_id).file.size)
                );
            }
            Err(e) => {
                self.status_message = format!("Error: {}", e);
                self.mode = AppMode::Viewing;
            }
        }
    }

    /// Rebuild the radial map from current state
    pub fn rebuild_map(&mut self) {
        if let Some(ref arena) = self.arena {
            if let Some(root_id) = arena.root() {
                let config = RadialConfig {
                    ring_depth: self.ring_depth,
                    terminal_width: self.terminal_size.0,
                    terminal_height: self.terminal_size.1,
                    ..Default::default()
                };
                self.radial_map = Some(build_radial_map(arena, root_id, &config));
                self.canvas_coords = Some(CanvasCoords::new(
                    self.terminal_size.0 as usize,
                    self.terminal_size.1 as usize,
                ));
            }
        }
    }

    /// Handle terminal resize
    pub fn resize(&mut self, width: u16, height: u16) {
        self.terminal_size = (width, height);
        self.rebuild_map();
    }

    /// Handle key event
    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.mode {
            AppMode::Scanning => {
                if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                    self.should_quit = true;
                }
            }
            AppMode::Viewing => {
                // If context menu is visible, handle menu navigation
                if self.context_menu.visible {
                    self.handle_context_menu_key(key);
                } else {
                    self.handle_viewing_key(key);
                }
            }
            AppMode::Help => {
                self.mode = AppMode::Viewing;
            }
        }
    }

    /// Handle key event when context menu is visible
    fn handle_context_menu_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.context_menu.hide(),
            KeyCode::Up | KeyCode::Char('k') => self.context_menu.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.context_menu.move_down(),
            KeyCode::Enter => self.execute_menu_action(),
            _ => {}
        }
    }

    fn handle_viewing_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') => self.mode = AppMode::Help,
            KeyCode::Char('u') | KeyCode::Backspace => self.navigate_up(),
            KeyCode::Enter => self.navigate_into_hovered(),
            KeyCode::Char('+') | KeyCode::Char('=') => self.zoom_in(),
            KeyCode::Char('-') => self.zoom_out(),
            KeyCode::Char('r') => self.start_scan(),
            KeyCode::Tab => self.toggle_focus(),
            KeyCode::Up | KeyCode::Char('k') => self.move_hover_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_hover_down(),
            _ => {}
        }
    }

    /// Handle mouse event
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.context_menu.visible {
                    // Check if clicking on menu item
                    self.handle_context_menu_click(mouse.column, mouse.row);
                } else {
                    self.handle_click_at(mouse.column, mouse.row);
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                // Show context menu if hovering over a segment
                self.show_context_menu(mouse.column, mouse.row);
            }
            MouseEventKind::Moved => {
                if !self.context_menu.visible {
                    self.handle_hover_at(mouse.column, mouse.row);
                }
            }
            MouseEventKind::ScrollUp => self.zoom_in(),
            MouseEventKind::ScrollDown => self.zoom_out(),
            _ => {}
        }
    }

    /// Calculate canvas area based on terminal size
    /// This matches the actual canvas widget area including its borders
    fn canvas_area(&self) -> ratatui::layout::Rect {
        let total_width = self.terminal_size.0;
        let total_height = self.terminal_size.1;
        let sidebar_width = (total_width * 25) / 100;

        // Map area is after sidebar
        let map_width = total_width - sidebar_width;
        // Canvas height is total height minus status bar (2 rows)
        let canvas_height = total_height - 2;

        ratatui::layout::Rect {
            x: sidebar_width,
            y: 0,
            width: map_width,
            height: canvas_height,
        }
    }

    /// Calculate sidebar area based on terminal size
    fn sidebar_area(&self) -> ratatui::layout::Rect {
        let total_width = self.terminal_size.0;
        let sidebar_width = (total_width * 25) / 100;
        ratatui::layout::Rect {
            x: 0,
            y: 0,
            width: sidebar_width,
            height: self.terminal_size.1,
        }
    }

    /// Convert terminal cell coordinates to canvas coordinates
    /// Returns None if the point is not in the canvas area
    fn terminal_to_canvas(&self, col: u16, row: u16) -> Option<(f64, f64)> {
        let canvas = self.canvas_area();

        // The canvas widget has borders (Block::default().borders(Borders::ALL))
        // So the inner drawing area starts at x+1, y+1 and ends at x+w-1, y+h-1
        let inner_x = canvas.x + 1;
        let inner_y = canvas.y + 1;
        let inner_width = canvas.width.saturating_sub(2);
        let inner_height = canvas.height.saturating_sub(2);

        // Check if within the inner canvas area
        if col < inner_x
            || col >= inner_x + inner_width
            || row < inner_y
            || row >= inner_y + inner_height
        {
            return None;
        }

        // Get the radial map bounds
        let radial_map = self.radial_map.as_ref()?;
        let max_radius = radial_map
            .rings
            .last()
            .map(|r| r.outer_radius)
            .unwrap_or(radial_map.center_radius);

        // Match the bounds calculation from render_radial_map
        let pixel_width = inner_width as f64 * 2.0;
        let pixel_height = inner_height as f64 * 4.0;
        let aspect_ratio = pixel_height / pixel_width;

        // Bounds match what's set in ui.rs
        let x_bound = max_radius;
        let y_bound = max_radius * aspect_ratio;

        // Convert from cell position to relative position (0 to 1)
        let rel_x = (col - inner_x) as f64 / inner_width as f64;
        let rel_y = (row - inner_y) as f64 / inner_height as f64;

        // Convert to canvas coordinates
        // Canvas: x goes from -x_bound (left) to +x_bound (right)
        // Canvas: y goes from -y_bound (bottom) to +y_bound (top)
        // Terminal: y=0 at top, increases downward
        let canvas_x = -x_bound + rel_x * 2.0 * x_bound;
        let canvas_y = y_bound - rel_y * 2.0 * y_bound; // Y inverted

        Some((canvas_x, canvas_y))
    }

    /// Handle click at screen position
    fn handle_click_at(&mut self, col: u16, row: u16) {
        let sidebar = self.sidebar_area();

        // Check if click is within sidebar
        if col >= sidebar.x
            && col < sidebar.x + sidebar.width
            && row >= sidebar.y
            && row < sidebar.y + sidebar.height
        {
            // Calculate item index (subtract 1 for border, 1 for title)
            if row >= 2 {
                let clicked_index = (row - 2) as usize;
                let items = self.sidebar_items();
                if clicked_index < items.len() {
                    self.sidebar_index = clicked_index;
                    self.focus = Focus::Sidebar;

                    // If it's a folder, navigate into it
                    let item = items[clicked_index];
                    if item.is_folder() {
                        if let Some(ref arena) = self.arena {
                            if let TreeItem::Folder(id, _) = item {
                                let path = arena.folder(id).file.path.clone();
                                self.navigate_into(path);
                            }
                        }
                    }
                    return;
                }
            }
            self.focus = Focus::Sidebar;
            return;
        }

        // Convert to canvas coordinates
        if let Some((canvas_x, canvas_y)) = self.terminal_to_canvas(col, row) {
            self.handle_canvas_click(canvas_x, canvas_y);
        }
    }

    /// Handle hover at screen position
    fn handle_hover_at(&mut self, col: u16, row: u16) {
        let sidebar = self.sidebar_area();

        // Check if hover is within sidebar
        if col >= sidebar.x
            && col < sidebar.x + sidebar.width
            && row >= sidebar.y
            && row < sidebar.y + sidebar.height
        {
            if row >= 2 {
                let hover_index = (row - 2) as usize;
                let items = self.sidebar_items();
                if hover_index < items.len() {
                    self.sidebar_hover_index = Some(hover_index);
                } else {
                    self.sidebar_hover_index = None;
                }
            } else {
                self.sidebar_hover_index = None;
            }
            // Clear map hover when in sidebar
            self.hovered_uuid = None;
            self.renderer.set_hovered(None);
            return;
        }

        // Clear sidebar hover when in map
        self.sidebar_hover_index = None;

        // Convert to canvas coordinates and handle hover
        if let Some((canvas_x, canvas_y)) = self.terminal_to_canvas(col, row) {
            self.handle_canvas_hover(canvas_x, canvas_y);
        } else {
            // Not in map area
            self.hovered_uuid = None;
            self.renderer.set_hovered(None);
        }
    }

    /// Handle mouse click in canvas coordinates
    fn handle_canvas_click(&mut self, x: f64, y: f64) {
        if let Some(ref map) = self.radial_map {
            // Calculate radius from center (0, 0) in canvas coords
            let radius = (x * x + y * y).sqrt();

            // Check if clicked on center (go up)
            if radius < map.center_radius {
                self.navigate_up();
                return;
            }

            // Calculate angle from canvas coordinates
            let mut angle = y.atan2(x).to_degrees();
            if angle < 0.0 {
                angle += 360.0;
            }

            // Find segment at this position
            for ring in &map.rings {
                if radius >= ring.inner_radius && radius <= ring.outer_radius {
                    let angle_16ths = ((angle * 16.0) as u32) % 5760;
                    for segment in &ring.segments {
                        if segment.contains_angle(angle_16ths) {
                            if segment.is_folder && !segment.is_fake {
                                self.navigate_into(PathBuf::from(&segment.path));
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Handle mouse hover in canvas coordinates
    fn handle_canvas_hover(&mut self, x: f64, y: f64) {
        if let Some(ref map) = self.radial_map {
            // Calculate radius and angle from center
            let radius = (x * x + y * y).sqrt();
            let mut angle = y.atan2(x).to_degrees();
            if angle < 0.0 {
                angle += 360.0;
            }

            // Find segment at this position
            for ring in &map.rings {
                if radius >= ring.inner_radius && radius <= ring.outer_radius {
                    let angle_16ths = ((angle * 16.0) as u32) % 5760;
                    for segment in &ring.segments {
                        if segment.contains_angle(angle_16ths) {
                            self.hovered_uuid = Some(segment.uuid);
                            self.renderer.set_hovered(Some(segment.uuid));
                            return;
                        }
                    }
                }
            }
        }

        // No segment found
        self.hovered_uuid = None;
        self.renderer.set_hovered(None);
    }

    /// Show context menu at cursor position if hovering over a segment
    fn show_context_menu(&mut self, col: u16, row: u16) {
        // Get the segment under cursor
        if let Some(uuid) = self.hovered_uuid {
            if let Some(ref map) = self.radial_map {
                // Find the segment
                for ring in &map.rings {
                    for segment in &ring.segments {
                        if segment.uuid == uuid {
                            self.context_menu.show(
                                col,
                                row,
                                segment.uuid,
                                segment.name.clone(),
                                segment.path.clone(),
                                segment.is_folder,
                            );
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Handle click on context menu
    fn handle_context_menu_click(&mut self, col: u16, row: u16) {
        // Calculate which menu item was clicked
        let menu = &self.context_menu;
        let items = menu.menu_items();
        let menu_x = menu.x;
        let menu_y = menu.y;
        let menu_width: u16 = 25;

        if col >= menu_x
            && col < menu_x + menu_width
            && row > menu_y
            && row <= menu_y + items.len() as u16
        {
            let clicked_index = (row - menu_y - 1) as usize;
            if clicked_index < items.len() {
                self.context_menu.selected_index = clicked_index;
                self.execute_menu_action();
            }
        } else {
            // Click outside menu - close it
            self.context_menu.hide();
        }
    }

    /// Execute the currently selected menu action
    fn execute_menu_action(&mut self) {
        if let Some(action) = self.context_menu.selected_action() {
            let path = self.context_menu.segment_path.clone();
            let is_folder = self.context_menu.is_folder;

            match action {
                MenuAction::Open => {
                    if is_folder {
                        self.navigate_into(PathBuf::from(&path));
                    } else {
                        // Open file with system handler
                        #[cfg(unix)]
                        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                        #[cfg(target_os = "macos")]
                        let _ = std::process::Command::new("open").arg(&path).spawn();
                        #[cfg(target_os = "windows")]
                        let _ = std::process::Command::new("cmd")
                            .args(["/C", "start", &path])
                            .spawn();
                    }
                }
                MenuAction::OpenTerminal => {
                    if is_folder {
                        // Try common terminal emulators
                        let terminals =
                            ["alacritty", "kitty", "gnome-terminal", "xterm", "konsole"];
                        for term in &terminals {
                            if let Ok(mut child) =
                                std::process::Command::new(term).current_dir(&path).spawn()
                            {
                                let _ = child.wait();
                                break;
                            }
                        }
                    }
                }
                MenuAction::CenterMap => {
                    if is_folder {
                        self.navigate_into(PathBuf::from(&path));
                    }
                }
                MenuAction::CopyPath => {
                    // Copy path to clipboard using arboard
                    match arboard::Clipboard::new() {
                        Ok(mut clipboard) => match clipboard.set_text(&path) {
                            Ok(_) => {
                                self.status_message = format!("Copied: {}", path);
                            }
                            Err(e) => {
                                self.status_message = format!("Failed to copy: {}", e);
                            }
                        },
                        Err(e) => {
                            self.status_message = format!("Clipboard unavailable: {}", e);
                        }
                    }
                }
                MenuAction::Rescan => {
                    self.start_scan();
                }
                MenuAction::Delete => {
                    // Delete file or directory
                    let path_buf = PathBuf::from(&path);
                    if is_folder {
                        if let Err(e) = std::fs::remove_dir_all(&path_buf) {
                            self.status_message = format!("Error: {}", e);
                        } else {
                            self.status_message = format!("Deleted: {}", path);
                            self.start_scan();
                        }
                    } else {
                        if let Err(e) = std::fs::remove_file(&path_buf) {
                            self.status_message = format!("Error: {}", e);
                        } else {
                            self.status_message = format!("Deleted: {}", path);
                            self.start_scan();
                        }
                    }
                }
            }
        }

        self.context_menu.hide();
    }

    /// Navigate up to parent directory
    pub fn navigate_up(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            self.nav_history.push(self.current_path.clone());
            self.current_path = parent.to_path_buf();
            self.hovered_uuid = None;
            self.renderer.set_hovered(None);
            self.start_scan();
        }
    }

    /// Navigate into a folder
    pub fn navigate_into(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.nav_history.push(self.current_path.clone());
            self.current_path = path;
            self.hovered_uuid = None;
            self.renderer.set_hovered(None);
            self.start_scan();
        }
    }

    /// Navigate into the currently hovered folder
    fn navigate_into_hovered(&mut self) {
        if let Some(uuid) = self.hovered_uuid {
            if let Some(ref map) = self.radial_map {
                if let Some(segment) = self.renderer.find_segment(map, &uuid) {
                    if segment.is_folder && !segment.is_fake {
                        self.navigate_into(PathBuf::from(&segment.path));
                    }
                }
            }
        }
    }

    /// Zoom in (reduce ring depth)
    pub fn zoom_in(&mut self) {
        if self.ring_depth > 1 {
            self.ring_depth -= 1;
            self.rebuild_map();
            self.status_message = format!("Zoom: {} rings", self.ring_depth);
        }
    }

    /// Zoom out (increase ring depth)
    pub fn zoom_out(&mut self) {
        if self.ring_depth < 10 {
            self.ring_depth += 1;
            self.rebuild_map();
            self.status_message = format!("Zoom: {} rings", self.ring_depth);
        }
    }

    /// Toggle focus between map and sidebar
    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Map => Focus::Sidebar,
            Focus::Sidebar => Focus::Map,
        };
    }

    /// Move hover up in sidebar
    fn move_hover_up(&mut self) {
        if self.sidebar_index > 0 {
            self.sidebar_index -= 1;
        }
    }

    /// Move hover down in sidebar
    fn move_hover_down(&mut self) {
        if let Some(ref arena) = self.arena {
            if let Some(root_id) = arena.root() {
                let items = arena.folder_items(root_id);
                if self.sidebar_index < items.len().saturating_sub(1) {
                    self.sidebar_index += 1;
                }
            }
        }
    }

    /// Get the currently hovered segment
    pub fn hovered_segment(&self) -> Option<&Segment> {
        if let Some(uuid) = self.hovered_uuid {
            if let Some(ref map) = self.radial_map {
                return self.renderer.find_segment(map, &uuid);
            }
        }
        None
    }

    /// Get segments for sidebar display
    pub fn sidebar_items(&self) -> Vec<crate::tree::TreeItem> {
        if let Some(ref arena) = self.arena {
            if let Some(root_id) = arena.root() {
                return arena.folder_items(root_id);
            }
        }
        Vec::new()
    }

    /// Update scan progress
    pub fn update_scan_progress(&mut self) {
        if let Some(ref rx) = self.scan_rx {
            while let Ok(progress) = rx.try_recv() {
                self.scan_progress = Some(progress);
            }
        }
    }

    /// Get status message
    pub fn status_text(&self) -> String {
        match self.mode {
            AppMode::Scanning => {
                if let Some(ref progress) = self.scan_progress {
                    format!(
                        "Scanning... {} files ({})",
                        progress.files_scanned,
                        format_size(progress.total_size)
                    )
                } else {
                    "Scanning...".to_string()
                }
            }
            AppMode::Viewing => {
                if !self.status_message.is_empty() {
                    self.status_message.clone()
                } else if let Some(ref arena) = self.arena {
                    if let Some(root_id) = arena.root() {
                        format!(
                            "{} files ({})",
                            arena.total_file_count(root_id),
                            format_size(arena.folder(root_id).file.size)
                        )
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            }
            AppMode::Help => "Press any key to close help".to_string(),
        }
    }

    /// Get tooltip text for hovered segment
    pub fn tooltip_text(&self) -> Option<String> {
        if let Some(segment) = self.hovered_segment() {
            let mut lines = vec![segment.name.clone(), format_size(segment.size)];

            if segment.is_folder {
                lines.push(format!("{} files", segment.file_count));
            }

            if segment.is_fake {
                lines.push(format!("{} small files", segment.file_count));
            }

            Some(lines.join("\n"))
        } else {
            None
        }
    }
}
