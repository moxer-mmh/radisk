use uuid::Uuid;

/// Menu item action
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuAction {
    Open,
    OpenTerminal,
    CenterMap,
    CopyPath,
    Rescan,
    Delete,
}

impl MenuAction {
    pub fn label(&self) -> &'static str {
        match self {
            MenuAction::Open => "Open",
            MenuAction::OpenTerminal => "Open Terminal Here",
            MenuAction::CenterMap => "Center Map Here",
            MenuAction::CopyPath => "Copy Path",
            MenuAction::Rescan => "Rescan",
            MenuAction::Delete => "Delete",
        }
    }
}

/// Context menu state
pub struct ContextMenu {
    pub visible: bool,
    pub x: u16,
    pub y: u16,
    pub selected_index: usize,
    pub segment_uuid: Option<Uuid>,
    pub segment_name: String,
    pub segment_path: String,
    pub is_folder: bool,
}

impl ContextMenu {
    pub fn new() -> Self {
        Self {
            visible: false,
            x: 0,
            y: 0,
            selected_index: 0,
            segment_uuid: None,
            segment_name: String::new(),
            segment_path: String::new(),
            is_folder: false,
        }
    }

    /// Show the context menu at position for a segment
    pub fn show(
        &mut self,
        x: u16,
        y: u16,
        uuid: Uuid,
        name: String,
        path: String,
        is_folder: bool,
    ) {
        self.visible = true;
        self.x = x;
        self.y = y;
        self.selected_index = 0;
        self.segment_uuid = Some(uuid);
        self.segment_name = name;
        self.segment_path = path;
        self.is_folder = is_folder;
    }

    /// Hide the context menu
    pub fn hide(&mut self) {
        self.visible = false;
        self.segment_uuid = None;
    }

    /// Get available menu items based on segment type
    pub fn menu_items(&self) -> Vec<MenuAction> {
        let mut items = vec![MenuAction::Open];

        if self.is_folder {
            items.push(MenuAction::OpenTerminal);
            items.push(MenuAction::CenterMap);
        }

        items.push(MenuAction::CopyPath);

        if self.is_folder {
            items.push(MenuAction::Rescan);
        }

        items.push(MenuAction::Delete);

        items
    }

    /// Get the number of visible menu items
    pub fn item_count(&self) -> usize {
        self.menu_items().len()
    }

    /// Move selection up
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Move selection down
    pub fn move_down(&mut self) {
        if self.selected_index < self.item_count().saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    /// Get the currently selected action
    pub fn selected_action(&self) -> Option<MenuAction> {
        self.menu_items().get(self.selected_index).copied()
    }
}

impl Default for ContextMenu {
    fn default() -> Self {
        Self::new()
    }
}
