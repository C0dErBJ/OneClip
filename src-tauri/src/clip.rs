use crate::clip::{self, clip_frame::ClipFrame};
use base64::{
    alphabet::URL_SAFE,
    engine::{self, general_purpose},
    prelude::BASE64_STANDARD,
    Engine,
};
use clip_frame::clip_frame::Frametype;
use clipboard_rs::{
    common::{RustImage, RustImageBuffer},
    Clipboard, ClipboardContext, ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext,
    ContentFormat, RustImageData, WatcherShutdown,
};
use configparser::ini::Ini;
use std::{
    borrow::Borrow,
    default,
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
};
use tauri::Url;
use tokio::{sync::broadcast, task::JoinSet};
use urlencoding::decode;
pub mod clip_frame {
    include!(concat!(env!("OUT_DIR"), "/clip_frame.rs"));
}

struct Manager {
    writer_sender: broadcast::Sender<ClipFrame>,
    remote_flag: Arc<Mutex<bool>>,
    ctx: ClipboardContext,
}

impl Manager {
    pub fn new(writer_sender: broadcast::Sender<ClipFrame>, remote_flag: Arc<Mutex<bool>>) -> Self {
        let ctx = ClipboardContext::new().unwrap();
        Manager {
            writer_sender,
            remote_flag,
            ctx,
        }
    }
}

impl ClipboardHandler for Manager {
    fn on_clipboard_change(&mut self) {
        let mut flag = self.remote_flag.lock().unwrap();
        if !*flag {
            let clip_frame = self.clip_data_converter();
            let ss: broadcast::Sender<ClipFrame> = self.writer_sender.clone();
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
pub struct ClipboardChangeWatcher {
    reader_sender: broadcast::Sender<ClipFrame>,
    writer_sender: broadcast::Sender<ClipFrame>,
    remote_flag: Arc<Mutex<bool>>,
    tmp_dir: String,
}
impl ClipboardChangeWatcher {
    pub fn new(
        reader_sender: broadcast::Sender<ClipFrame>,
        writer_sender: broadcast::Sender<ClipFrame>,
        tmp_dir: String,
    ) -> Self {
        let remote_income_flag = Arc::new(Mutex::new(false));
        ClipboardChangeWatcher {
            reader_sender,
            writer_sender,
            remote_flag: remote_income_flag,
            tmp_dir,
        }
    }
}
pub trait ClipConverter {
    fn remote_data_loader(ctx: ClipboardContext, clip_frame: ClipFrame, tmp_dir: String);
    fn clip_data_converter(&self) -> ClipFrame;
}
impl ClipConverter for Manager {
    fn remote_data_loader(ctx: ClipboardContext, clip_frame: ClipFrame, tmp_dir: String) {
        match clip_frame.frame_type() {
            Frametype::Text => {
                let _ = ctx.set_text(
                    String::from_utf8(clip_frame.content.get(0).unwrap().to_vec()).unwrap(),
                );
            }
            Frametype::Html => {
                let _ = ctx.set_html(
                    String::from_utf8(clip_frame.content.get(0).unwrap().to_vec()).unwrap(),
                );
            }
            Frametype::Rtf => {
                let _ = ctx.set_rich_text(
                    String::from_utf8(clip_frame.content.get(0).unwrap().to_vec()).unwrap(),
                );
            }
            Frametype::Image => {
                let img = RustImage::from_bytes(clip_frame.content.get(0).unwrap());
                let _ = ctx.set_image(img.unwrap());
            }
            //todo
            Frametype::Files => {
                let mut local_files = Vec::new();
                for file_num in 0..(clip_frame.content.len()) {
                    let file_name =
                        String::from_utf8(clip_frame.file_names.get(file_num).unwrap().to_vec())
                            .unwrap();

                    let file_path = Path::new(tmp_dir.as_str()).join(file_name.clone());
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

    fn clip_data_converter(&self) -> ClipFrame {
        let mut clip_frame = ClipFrame::default();
        clip_frame.content = Vec::new();
        clip_frame.file_names = Vec::new();
        if self.ctx.has(ContentFormat::Files) {
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
        } else if self.ctx.has(ContentFormat::Html) {
            clip_frame.set_frame_type(Frametype::Html);
            clip_frame.clip_type = "public.utf8-plain-text".to_string();
            let buffer = self.ctx.get_html().unwrap();
            clip_frame.content.push(buffer.as_bytes().to_vec());
            clip_frame.content_num = 1;
        } else if self.ctx.has(ContentFormat::Image) {
            clip_frame.set_frame_type(Frametype::Image);
            clip_frame.clip_type = "public.png".to_string();
            let img = self.ctx.get_image().unwrap();
            let mut buffer = img.to_png().unwrap().get_bytes().to_vec();
            clip_frame.content.push(buffer);
            clip_frame.content_num = 1;
        } else if self.ctx.has(ContentFormat::Rtf) {
            //富文本
            clip_frame.set_frame_type(Frametype::Rtf);
            clip_frame.clip_type = "public.rtf".to_string();
            let buffer = self.ctx.get_rich_text().unwrap();
            clip_frame.content.push(buffer.as_bytes().to_vec());
            clip_frame.content_num = 1;
        } else {
            //普通文本
            clip_frame.set_frame_type(Frametype::Text);
            clip_frame.clip_type = "public.utf8-plain-text".to_string();
            let buffer = self.ctx.get_text().unwrap();
            clip_frame.content.push(buffer.as_bytes().to_vec());
            clip_frame.content_num = 1;
        }
        clip_frame
    }
}

async fn local_clip_change_watcher(
    writer_sender: broadcast::Sender<ClipFrame>,
    remote_flag: Arc<Mutex<bool>>,
) {
    tracing::info!("设置剪贴板监听");
    let manager = Manager::new(writer_sender, remote_flag);
    let mut watcher: ClipboardWatcherContext<Manager> = ClipboardWatcherContext::new().unwrap();
    let _shutdown = watcher.add_handler(manager).get_shutdown_channel();
    watcher.start_watch();
    tracing::warn!("设置剪贴板监听结束");
}

async fn remote_clip_change_watcher(
    mut receiver: broadcast::Receiver<ClipFrame>,
    remote_flag: Arc<Mutex<bool>>,
    tmp_dir: String,
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
                Manager::remote_data_loader(ctx, key, tmp_dir.clone());
            }
            _ => {}
        }
    }
}
pub fn setup(
    config: &'static Ini,
    reader_sender: broadcast::Sender<ClipFrame>,
    writer_sender: broadcast::Sender<ClipFrame>,
) -> ClipboardChangeWatcher {
    let dir = config.get("default", "default_tmp_dir").unwrap();
    ClipboardChangeWatcher::new(reader_sender, writer_sender.clone(), dir)
}
pub async fn start(watcher: ClipboardChangeWatcher) {
    let mut set = JoinSet::new();
    set.spawn(local_clip_change_watcher(
        watcher.writer_sender,
        watcher.remote_flag.clone(),
    ));

    set.spawn(remote_clip_change_watcher(
        watcher.reader_sender.subscribe(),
        watcher.remote_flag,
        watcher.tmp_dir,
    ));
    while let Some(res) = set.join_next().await {
        let _ = res.unwrap();
    }
}
