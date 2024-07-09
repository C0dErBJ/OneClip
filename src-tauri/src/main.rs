// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use clip::clip_frame::ClipFrame;
use config::{init_config, CONFIG};
use configparser::ini::Ini;
use connect::{RWChannel, RemoteConnecter};
use tauri::{
    CustomMenuItem, Menu, MenuItem, Submenu, SystemTray, SystemTrayMenu, SystemTrayMenuItem,
};
use tokio::{sync::broadcast, task::JoinSet};

pub mod clip;
pub mod config;
pub mod connect;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn greet(name: &str) -> String {
    return format!("Hello, {}! You've been greeted from Rust!", name);
}

fn main() {
    //加载日志
    tracing_subscriber::registry().with(fmt::layer()).init();
    //加载日志
    let quit = CustomMenuItem::new("quit".to_string(), "Quit");
    let hide = CustomMenuItem::new("hide".to_string(), "Hide");
    let tray_menu = SystemTrayMenu::new()
        .add_item(quit)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(hide);
    let tray = SystemTray::new().with_menu(tray_menu);

    tauri::Builder::default()
        .system_tray(tray)
        .setup(|app| {
            let resource_path = app
                .path_resolver()
                .resolve_resource("config/config.ini")
                .expect("failed to resolve resource");

            tauri::async_runtime::spawn(async move {
                //初始化日志
                tracing::info!("初始化线程开始");
                unsafe { init_config(resource_path) }
                let (tx_read, _) = broadcast::channel::<ClipFrame>(20);
                let (tx_write, _) = broadcast::channel::<ClipFrame>(20);
                let config = unsafe { CONFIG.as_ref().unwrap() };
                let connector = connect::setup(config, tx_read.clone(), tx_write.clone());
                let clip_watcher = clip::setup(config, tx_read, tx_write);
                //初始化时，先做一次链接
                tokio::select! {
                    _=connect::start(connector, String::from("client"))=>{}
                    _=clip::start(clip_watcher)=>{}
                }

                tracing::info!("初始化线程结束");
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
