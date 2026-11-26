pub use crate::input::{volume_key, volume_slot, DirectInput, InputEvent};
#[cfg(target_os = "linux")]
use crate::linux as native;
#[cfg(target_os = "macos")]
use crate::macos as native;
pub use native::{replay, set_startup, shutdown, startup_enabled};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};

pub struct Listener {
    pub events: mpsc::Receiver<InputEvent>,
    tray: Option<crate::desktop_tray::TrayHandle>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Listener {
    pub fn start_with_wake(
        vid: u16,
        pid: u16,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let (events, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let worker = native::start(vid, pid, events.clone(), wake.clone(), stop.clone())?;
        let tray = crate::desktop_tray::open(events, wake).ok();
        Ok(Self {
            events: receiver,
            tray,
            stop,
            worker: Some(worker),
        })
    }

    pub fn tray_available(&self) -> bool {
        self.tray.as_ref().is_some_and(|tray| tray.available())
    }

    pub fn refresh_tray_language(&self) {
        if let Some(tray) = &self.tray {
            tray.refresh_language();
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
pub struct ScreenToggle(bool);

impl ScreenToggle {
    pub fn toggle(&mut self) -> Result<bool, String> {
        native::toggle_display(&mut self.0)
    }
}

pub fn wake_gui(_window: usize) {
    // eframe's context is awakened directly by the portable input listener.
}

pub fn claim_instance() -> Result<Option<File>, String> {
    let dir = crate::device::settings_dir();
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("instancia.lock"))
        .map_err(|error| error.to_string())?;
    match file.try_lock() {
        Ok(()) => {
            let _ = take_show_request();
            Ok(Some(file))
        }
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error.to_string()),
    }
}

pub fn restore_existing_app(_title: &str) {
    let _ = fs::write(crate::device::settings_dir().join("mostrar"), b"");
}

pub fn take_show_request() -> bool {
    fs::remove_file(crate::device::settings_dir().join("mostrar")).is_ok()
}
