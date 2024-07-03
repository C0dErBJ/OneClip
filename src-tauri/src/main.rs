// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};

use clip::clip_frame::ClipFrame;
use configparser::ini::Ini;
use tokio::{sync::broadcast, task::JoinSet};

pub mod clip;
pub mod connect;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn greet(name: &str) -> String {
    return format!("Hello, {}! You've been greeted from Rust!", name);
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let resource_path = app
                .path_resolver()
                .resolve_resource("config/config.ini")
                .expect("failed to resolve resource");

            tauri::async_runtime::spawn(async move {
                //初始化日志
                tracing_subscriber::registry().with(fmt::layer()).init();

                let (tx_read, mut rx_read) = broadcast::channel::<ClipFrame>(20);
                let (tx_write, _) = broadcast::channel::<ClipFrame>(20);
                let lock: Vec<u8> = Vec::new();
                let current_clip = Arc::new(Mutex::new(lock));
                let current_clip_clone = Arc::clone(&current_clip);
                let mut config = Ini::new();
                let _config_map = config.load(resource_path).unwrap();

                let mut set = JoinSet::new();
                let new_tx_write = tx_write.clone();

                let mode = config.get("基础设置", "mode").unwrap();
                if mode == "client" {
                    set.spawn(connect::do_client(
                        String::from(config.get("基础设置", "server_ip").unwrap()),
                        tx_read,
                        tx_write,
                    ));
                } else {
                    set.spawn(connect::do_server(
                        String::from(config.get("基础设置", "host_port").unwrap()),
                        tx_read,
                        tx_write,
                    ));
                }

                set.spawn(clip::start(new_tx_write, current_clip));
                set.spawn(clip::set_clip(rx_read, current_clip_clone));
                while let Some(res) = set.join_next().await {
                    let _ = res.unwrap();
                }
                tracing::info!("all done");
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
