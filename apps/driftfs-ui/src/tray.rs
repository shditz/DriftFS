use std::sync::mpsc::{channel, Receiver};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum TrayCommand {
    ShowWindow,
    ToggleMount,
    OpenExplorer,
    Quit,
}

pub struct SystemTrayManager {
    _tray_icon: TrayIcon,
    status_item: MenuItem,
    mount_item: MenuItem,
    command_rx: Receiver<TrayCommand>,
}

impl SystemTrayManager {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (cmd_tx, cmd_rx) = channel();

        let menu = Menu::new();
        let status_item = MenuItem::new("Status: Unmounted", false, None);
        let mount_item = MenuItem::new("Mount Drive (G:)", true, None);
        let open_item = MenuItem::new("Open in Explorer", true, None);
        let show_item = MenuItem::new("Open DriftFS", true, None);
        let separator = PredefinedMenuItem::separator();
        let quit_item = MenuItem::new("Quit DriftFS", true, None);

        menu.append(&status_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&mount_item)?;
        menu.append(&open_item)?;
        menu.append(&separator)?;
        menu.append(&show_item)?;
        menu.append(&quit_item)?;

        let icon = create_app_icon();

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("DriftFS - Google Drive Virtual Filesystem")
            .with_icon(icon)
            .build()?;

        let mount_id = mount_item.id().clone();
        let open_id = open_item.id().clone();
        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();

        let tx_clone = cmd_tx.clone();
        std::thread::spawn(move || loop {
            if let Ok(event) = MenuEvent::receiver().recv() {
                if event.id == mount_id {
                    let _ = tx_clone.send(TrayCommand::ToggleMount);
                } else if event.id == open_id {
                    let _ = tx_clone.send(TrayCommand::OpenExplorer);
                } else if event.id == show_id {
                    let _ = tx_clone.send(TrayCommand::ShowWindow);
                } else if event.id == quit_id {
                    let _ = tx_clone.send(TrayCommand::Quit);
                }
            }
        });

        let tx_tray = cmd_tx;
        std::thread::spawn(move || {
            while let Ok(event) = TrayIconEvent::receiver().recv() {
                if let TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    ..
                } = event
                {
                    let _ = tx_tray.send(TrayCommand::ShowWindow);
                }
            }
        });

        Ok(Self {
            _tray_icon: tray_icon,
            status_item,
            mount_item,
            command_rx: cmd_rx,
        })
    }

    pub fn try_recv(&self) -> Option<TrayCommand> {
        self.command_rx.try_recv().ok()
    }

    pub fn update_mount_status(&self, is_mounted: bool, drive_letter: &str) {
        if is_mounted {
            self.status_item
                .set_text(format!("Status: Mounted ({drive_letter})"));
            self.mount_item
                .set_text(format!("Unmount Drive ({drive_letter})"));
        } else {
            self.status_item.set_text("Status: Unmounted");
            self.mount_item
                .set_text(format!("Mount Drive ({drive_letter})"));
        }
    }
}

fn create_app_icon() -> Icon {
    const WIDTH: u32 = 32;
    const HEIGHT: u32 = 32;
    const RAW_RGBA: &[u8] = include_bytes!("../../../assets/icons/driftfs_32.rgba");
    Icon::from_rgba(RAW_RGBA.to_vec(), WIDTH, HEIGHT).expect("valid icon buffer")
}
