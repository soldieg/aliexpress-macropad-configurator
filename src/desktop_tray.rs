use crate::{i18n::tr_native as tr, input::InputEvent};
use std::sync::{mpsc::Sender, Arc};

const LOGO: &[u8] = include_bytes!("../assets/logo.png");
const TITLE: &str = "Macro para Mini Teclados";

#[derive(Clone)]
struct Dispatch {
    events: Sender<InputEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Dispatch {
    fn emit(&self, event: InputEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }
}

fn logo(size: u32) -> Result<image::RgbaImage, String> {
    image::load_from_memory_with_format(LOGO, image::ImageFormat::Png)
        .map(|source| {
            source
                .resize(size, size, image::imageops::FilterType::Lanczos3)
                .into_rgba8()
        })
        .map_err(|error| format!("Não foi possível carregar o ícone da bandeja. {error}"))
}

pub use native::TrayHandle;

// The native macOS implementation must be opened and dropped on the GUI thread.
pub fn open(
    events: Sender<InputEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<TrayHandle, String> {
    native::open(Dispatch { events, wake })
}

#[cfg(target_os = "linux")]
mod native {
    use super::{logo, tr, Arc, Dispatch, InputEvent, TITLE};
    use ksni::{
        blocking::{Handle, TrayMethods},
        menu::StandardItem,
        Icon, MenuItem, OfflineReason, Tray,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct StatusTray {
        available: Arc<AtomicBool>,
        dispatch: Dispatch,
        icon: Icon,
    }

    impl Tray for StatusTray {
        fn id(&self) -> String {
            env!("CARGO_PKG_NAME").into()
        }

        fn title(&self) -> String {
            tr(TITLE).into()
        }

        fn icon_pixmap(&self) -> Vec<Icon> {
            vec![self.icon.clone()]
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            self.dispatch.emit(InputEvent::ShowWindow);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                StandardItem {
                    label: tr("Abrir").into(),
                    activate: Box::new(|tray: &mut Self| {
                        tray.dispatch.emit(InputEvent::ShowWindow)
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: tr("Sair").into(),
                    activate: Box::new(|tray: &mut Self| tray.dispatch.emit(InputEvent::Quit)),
                    ..Default::default()
                }
                .into(),
            ]
        }

        fn menu_about_to_show(&mut self) {
            // Regenerate labels in the active language when the shell opens the menu.
        }

        fn watcher_offline(&self, _reason: OfflineReason) -> bool {
            self.available.store(false, Ordering::Release);
            self.dispatch.emit(InputEvent::ShowWindow);
            true
        }

        fn watcher_online(&self) {
            self.available.store(true, Ordering::Release);
            (self.dispatch.wake)();
        }
    }

    pub struct TrayHandle {
        handle: Handle<StatusTray>,
        available: Arc<AtomicBool>,
    }

    impl TrayHandle {
        pub fn available(&self) -> bool {
            self.available.load(Ordering::Acquire) && !self.handle.is_closed()
        }

        pub fn refresh_language(&self) {
            let _ = self.handle.update(|_| {});
        }
    }

    impl Drop for TrayHandle {
        fn drop(&mut self) {
            self.handle.shutdown();
        }
    }

    pub(super) fn open(dispatch: Dispatch) -> Result<TrayHandle, String> {
        let pixels = logo(40)?;
        let (width, height) = pixels.dimensions();
        let mut data = pixels.into_raw();
        // StatusNotifierItem requires ARGB32 in network byte order.
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }

        let available = Arc::new(AtomicBool::new(true));
        let handle = StatusTray {
            available: Arc::clone(&available),
            dispatch,
            icon: Icon {
                width: width as i32,
                height: height as i32,
                data,
            },
        }
        .spawn()
        .map_err(|error| format!("A bandeja StatusNotifierItem não está disponível. {error}"))?;
        Ok(TrayHandle { handle, available })
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{logo, tr, Arc, Dispatch, InputEvent, TITLE};
    use std::sync::{Mutex, Once};
    use tray_icon::{
        menu::{Menu, MenuEvent, MenuId, MenuItem},
        Icon, TrayIcon, TrayIconBuilder,
    };

    struct MenuTarget {
        open: MenuId,
        quit: MenuId,
        dispatch: Dispatch,
    }

    static HANDLER: Once = Once::new();
    static TARGETS: Mutex<Vec<MenuTarget>> = Mutex::new(Vec::new());

    fn dispatch_menu(event: &MenuEvent) -> bool {
        let selected = TARGETS.lock().ok().and_then(|targets| {
            targets.iter().find_map(|target| {
                let action = if event.id == target.open {
                    InputEvent::ShowWindow
                } else if event.id == target.quit {
                    InputEvent::Quit
                } else {
                    return None;
                };
                Some((target.dispatch.clone(), action))
            })
        });
        if let Some((dispatch, action)) = selected {
            dispatch.emit(action);
            true
        } else {
            false
        }
    }

    pub struct TrayHandle {
        tray: TrayIcon,
        open: MenuItem,
        quit: MenuItem,
    }

    impl TrayHandle {
        pub fn available(&self) -> bool {
            true
        }

        pub fn refresh_language(&self) {
            self.open.set_text(tr("Abrir"));
            self.quit.set_text(tr("Sair"));
            let _ = self.tray.set_tooltip(Some(tr(TITLE)));
        }
    }

    impl Drop for TrayHandle {
        fn drop(&mut self) {
            if let Ok(mut targets) = TARGETS.lock() {
                targets.retain(|target| &target.open != self.open.id());
            }
        }
    }

    pub(super) fn open(dispatch: Dispatch) -> Result<TrayHandle, String> {
        let pixels = logo(40)?;
        let (width, height) = pixels.dimensions();
        let icon = Icon::from_rgba(pixels.into_raw(), width, height)
            .map_err(|error| format!("Ícone de bandeja inválido. {error}"))?;

        let open = MenuItem::new(tr("Abrir"), true, None);
        let quit = MenuItem::new(tr("Sair"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &quit])
            .map_err(|error| format!("Não foi possível criar o menu da bandeja. {error}"))?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(tr(TITLE))
            .with_menu(Box::new(menu))
            .build()
            .map_err(|error| format!("Não foi possível criar o ícone da bandeja. {error}"))?;

        HANDLER.call_once(|| {
            let previous = MenuEvent::set_event_handler(None);
            let _ = MenuEvent::set_event_handler(Some(Arc::new(move |event| {
                if !dispatch_menu(&event) {
                    if let Some(handler) = &previous {
                        handler(event);
                    }
                }
            })));
        });
        TARGETS
            .lock()
            .map_err(|_| "O menu da bandeja não está disponível.")?
            .push(MenuTarget {
                open: open.id().clone(),
                quit: quit.id().clone(),
                dispatch,
            });

        Ok(TrayHandle { tray, open, quit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_dispatch_sends_the_requested_action_and_wakes_once() {
        let (events, receiver) = std::sync::mpsc::channel();
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let dispatch = Dispatch {
            events,
            wake: Arc::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }),
        };
        dispatch.emit(InputEvent::ShowWindow);
        assert_eq!(receiver.recv().unwrap(), InputEvent::ShowWindow);
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
        dispatch.emit(InputEvent::Quit);
        assert_eq!(receiver.recv().unwrap(), InputEvent::Quit);
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 2);
        drop(receiver);
        dispatch.emit(InputEvent::Quit);
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 2);
    }
}
