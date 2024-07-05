use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use crate::clip::{self, clip_frame::ClipFrame};
use base64::{
    alphabet::URL_SAFE,
    engine::{self, general_purpose},
    prelude::BASE64_STANDARD,
    Engine,
};
use clip_frame::clip_frame::Frametype;
use clipboard_rs::{
    common::RustImageBuffer, Clipboard, ClipboardContext, ClipboardHandler, ClipboardWatcher,
    ClipboardWatcherContext, ContentFormat,
};

use encoding_rs::GBK;
use md5::{Digest, Md5};
use tauri::Url;
use tokio::{sync::broadcast, task::JoinSet};
use urlencoding::decode;

pub mod clip_frame {
    include!(concat!(env!("OUT_DIR"), "/clip_frame.rs"));
}
struct Manager {
    ctx: ClipboardContext,
    sender: broadcast::Sender<ClipFrame>,
    remote_flag: Arc<Mutex<bool>>,
}

impl Manager {
    pub fn new(sender: broadcast::Sender<ClipFrame>, remote_flag: Arc<Mutex<bool>>) -> Self {
        let ctx = ClipboardContext::new().unwrap();
        Manager {
            ctx: ctx,
            sender: sender,
            remote_flag: remote_flag,
        }
    }
}

impl ClipboardHandler for Manager {
    fn on_clipboard_change(&mut self) {
        let mut flag = self.remote_flag.lock().unwrap();
        if !*flag {
            let clip_frame = self.detect();
            let ss: broadcast::Sender<ClipFrame> = self.sender.clone();
            tokio::task::block_in_place(move || {
                if clip_frame.content.len() == 0 {
                    tracing::info!("无识别内容");
                    return;
                }
                let _ = ss.send(clip_frame);
            });
        } else {
            tracing::info!("剪贴板文字与远端一致本次不发送");
        }
        *flag = false;
    }
}
pub trait ClipboardTypeDetector {
    fn detect(&self) -> ClipFrame;
}
impl ClipboardTypeDetector for Manager {
    fn detect(&self) -> ClipFrame {
        let mut clip_frame = ClipFrame::default();
        clip_frame.content = Vec::new();
        clip_frame.file_names = Vec::new();
        let types = self.ctx.available_formats().unwrap();
        if types.contains(&String::from("public.file-url")) {
            //判断为文件
            clip_frame.set_frame_type(Frametype::Files);
            clip_frame.clip_type = "public.file-url".to_string();
            match self.ctx.get_files() {
                Ok(files) => {
                    tracing::info!("{:?}", files);
                    clip_frame.content_num = files.len() as i32;
                    for file_path in files {
                        let plain_path = decode(file_path.as_str()).unwrap().to_string();
                        let file = Path::new(&plain_path);
                        let filename = file.file_name().unwrap().as_encoded_bytes().to_vec();
                        clip_frame.file_names.push(filename);
                        match File::open(plain_path.replace("file://", "")) {
                            Ok(mut fs) => {
                                let mut buf = Vec::new();
                                let _ = fs.read_to_end(&mut buf).unwrap();
                                clip_frame.content.push(buf);
                            }
                            Err(e) => {
                                tracing::error!("文件读取失败[{}]", e);
                            }
                        }
                    }
                }
                Err(_) => {}
            }
        } else if types.contains(&String::from("public.html"))
            || types.contains(&String::from("public.utf8-plain-text"))
        {
            clip_frame.set_frame_type(Frametype::Html);
            clip_frame.clip_type = "public.utf8-plain-text".to_string();
            let buffer = self.ctx.get_buffer("public.utf8-plain-text").unwrap();
            clip_frame.content.push(buffer);
            clip_frame.content_num = 1;
            //html文本
        } else if types.contains(&String::from("public.png"))
            || types.contains(&String::from("public.jpg"))
            || types.contains(&String::from("public.bmp"))
        {
            let mut buffer = Vec::new();
            clip_frame.set_frame_type(Frametype::Image);
            match self.ctx.get_buffer("public.png") {
                Ok(rs) => {
                    buffer = rs;
                    clip_frame.clip_type = "public.png".to_string();
                }
                Err(_) => match self.ctx.get_buffer("public.jpg") {
                    Ok(rs) => {
                        buffer = rs;
                        clip_frame.clip_type = "public.jpg".to_string();
                    }
                    Err(_) => match self.ctx.get_buffer("public.bmp") {
                        Ok(rs) => {
                            buffer = rs;
                            clip_frame.clip_type = "public.bmp".to_string();
                        }
                        Err(_) => {}
                    },
                },
            };
            clip_frame.content.push(buffer);
            clip_frame.content_num = 1;
            //图片
        } else if types.contains(&String::from("public.rtf"))
            || types.contains(&String::from("public.utf8-plain-text"))
        {
            //富文本
            clip_frame.set_frame_type(Frametype::Rtf);
            clip_frame.clip_type = "public.rtf".to_string();
            let buffer = self.ctx.get_buffer("public.rtf").unwrap();
            clip_frame.content.push(buffer);
            clip_frame.content_num = 1;
        } else {
            //普通文本
            clip_frame.set_frame_type(Frametype::Text);
            clip_frame.clip_type = "public.utf8-plain-text".to_string();
            let buffer = self.ctx.get_buffer("public.utf8-plain-text").unwrap();
            clip_frame.content.push(buffer);
            clip_frame.content_num = 1;
        }
        clip_frame
    }
}

fn convert_to_clip(ctx: ClipboardContext, clip_frame: ClipFrame) {
    match clip_frame.frame_type() {
        Frametype::Text => {
            let _ = ctx.set_buffer(
                clip_frame.clip_type.as_str(),
                clip_frame.content.get(0).unwrap().to_vec(),
            );
        }
        Frametype::Html => {
            let _ = ctx.set_buffer(
                clip_frame.clip_type.as_str(),
                clip_frame.content.get(0).unwrap().to_vec(),
            );
        }
        Frametype::Rtf => {
            let _ = ctx.set_rich_text(
                String::from_utf8(clip_frame.content.get(0).unwrap().to_vec()).unwrap(),
            );
        }
        Frametype::Image => {
            let _ = ctx.set_buffer(
                clip_frame.clip_type.as_str(),
                clip_frame.content.get(0).unwrap().to_vec(),
            );
        }
        //todo
        Frametype::Files => {
            let mut local_files = Vec::new();
            for file_num in 0..(clip_frame.content.len()) {
                let file_name =
                    String::from_utf8(clip_frame.file_names.get(file_num).unwrap().to_vec())
                        .unwrap();

                let file_path = Path::new("/Users/jialiangzhu/Downloads").join(file_name.clone());
                match File::create(file_path.to_str().unwrap()) {
                    Ok(mut file) => {
                        let _ = file.write_all(clip_frame.content.get(file_num).unwrap());
                        local_files.push(file_path.to_str().unwrap().to_string());
                    }
                    Err(_) => {}
                };
            }
            if local_files.len() > 0 {
                let _ = ctx.set_files(local_files);
            }
        }
        // todo
        _ => {}
    };
}

async fn clip_change_watcher(sender: broadcast::Sender<ClipFrame>, remote_flag: Arc<Mutex<bool>>) {
    tracing::info!("设置剪贴板监听");
    let manager = Manager::new(sender, remote_flag);
    let mut watcher = ClipboardWatcherContext::new().unwrap();
    let _shutdown = watcher.add_handler(manager).get_shutdown_channel();
    watcher.start_watch();
    tracing::warn!("设置剪贴板监听结束");
}

async fn remote_clip_change_watcher(
    mut receiver: broadcast::Receiver<ClipFrame>,
    remote_flag: Arc<Mutex<bool>>,
) {
    tracing::info!("设置剪贴板更新");
    loop {
        let val = receiver.recv().await;
        match val {
            Ok(key) => {
                tracing::info!("收到剪贴板更新");
                let ctx = ClipboardContext::new().unwrap();
                //信号量控制剪贴板更新，确保不重复更新
                let mut flag = remote_flag.lock().unwrap();
                *flag = true;
                convert_to_clip(ctx, key);
            }
            _ => {}
        }
    }
}

pub async fn start_clip(
    sender: broadcast::Sender<ClipFrame>,
    receiver: broadcast::Receiver<ClipFrame>,
    remote_flag: Arc<Mutex<bool>>,
) {
    let mut set = JoinSet::new();
    set.spawn(clip_change_watcher(sender, remote_flag.clone()));
    set.spawn(remote_clip_change_watcher(receiver, remote_flag));
    while let Some(res) = set.join_next().await {
        let _ = res.unwrap();
    }
}
