use std::path::PathBuf;

use configparser::ini::Ini;
use tokio::sync::watch;

pub static mut CONFIG: Option<&mut Ini> = None;
pub static mut CONFIG_FILE_PATH: Option<&mut PathBuf> = None;
pub static mut FILE_WATCHER: Option<watch::Sender<&&mut Ini>> = None;

pub fn init_config(resource_path: PathBuf) {
    let mut config = Ini::new();
    let _config_map = config.load(resource_path.clone()).unwrap();
    let or_config = Box::new(config);
    unsafe {
        CONFIG = Some(Box::leak(or_config));
        CONFIG_FILE_PATH = Some(Box::leak(Box::new(resource_path)));
        let (rx, _) = watch::channel(CONFIG.as_ref().unwrap());
        FILE_WATCHER = Some(rx);
    };
}

pub fn change_config(sec: String, key: String, value: String) {
    unsafe {
        CONFIG
            .as_mut()
            .unwrap()
            .set(sec.as_str(), key.as_str(), Some(value));
        let _ = CONFIG
            .as_mut()
            .unwrap()
            .write(CONFIG_FILE_PATH.as_ref().unwrap());
        let _ = FILE_WATCHER
            .as_mut()
            .unwrap()
            .send(CONFIG.as_ref().unwrap());
    };
}
