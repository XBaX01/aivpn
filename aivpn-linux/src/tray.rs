use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    TrayIcon, TrayIconBuilder,
};

pub struct Tray {
    _icon: TrayIcon,
    pub connect_id: tray_icon::menu::MenuId,
    pub quit_id: tray_icon::menu::MenuId,
}

impl Tray {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let connect_item = MenuItem::new("Connect / Show", true, None);
        let quit_item = MenuItem::new("Quit", true, None);

        let connect_id = connect_item.id().clone();
        let quit_id = quit_item.id().clone();

        let menu = Menu::new();
        menu.append(&connect_item)?;
        menu.append(&quit_item)?;

        // Minimal 16×16 RGBA icon (solid blue square)
        let rgba: Vec<u8> = (0..16 * 16)
            .flat_map(|_| [0x22_u8, 0x88, 0xff, 0xff])
            .collect();
        let icon = tray_icon::Icon::from_rgba(rgba, 16, 16)?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("AIVPN")
            .build()?;

        Ok(Self {
            _icon: tray,
            connect_id,
            quit_id,
        })
    }

    /// Drain pending tray menu events.
    pub fn poll(&self) -> TrayAction {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.quit_id {
                return TrayAction::Quit;
            }
            if event.id == self.connect_id {
                return TrayAction::ToggleWindow;
            }
        }
        TrayAction::None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrayAction {
    None,
    ToggleWindow,
    Quit,
}
