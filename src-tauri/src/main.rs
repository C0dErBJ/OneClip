// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{atomic::AtomicBool, Arc, Mutex};
use clip::clip_frame::ClipFrame;
use config::{init_config, CONFIG};
use connect::{RWChannel, RemoteConnecter};
use tauri::{
    CustomMenuItem, Menu, MenuItem, Submenu, SystemTray, SystemTrayMenu, SystemTrayMenuItem,
};
use tokio::sync::broadcast;

pub mod clip;
pub mod config;
pub mod connect;
use tracing_appender::{non_blocking, rolling};
use tracing_error::ErrorLayer;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Registry};

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn greet(name: &str) -> String {
    tracing::info!("hellos");
    return format!("Hello, {}! You've been greeted from Rust!", name);
}
fn init_log() {
    let env_filter = tracing_subscriber::filter::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::filter::EnvFilter::new("info"));
    // 输出到控制台中
    let formatting_layer = fmt::layer().pretty().with_writer(std::io::stderr);
    // 输出到文件中
    let file_appender = rolling::daily("logs", "app.log");
    let (non_blocking_appender, _guard) = non_blocking(file_appender);
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(non_blocking_appender);

    // 注册
    Registry::default()
        .with(env_filter)
        .with(ErrorLayer::default())
        .with(formatting_layer)
        .with(file_layer)
        .init();
    color_eyre::install();
}
fn main() {
    //加载日志
    init_log();
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
