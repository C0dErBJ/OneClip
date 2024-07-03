use std::sync::{Arc, Mutex};

use crate::clip::clip_frame::ClipFrame;
use clip_frame::clip_frame::Frametype;
use clipboard_rs::{
    Clipboard, ClipboardContext, ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext,
    ContentFormat,
};
use md5::{Digest, Md5};
use tokio::sync::broadcast;

pub mod clip_frame {
    include!(concat!(env!("OUT_DIR"), "/clip_frame.rs"));
}
struct Manager {
    ctx: ClipboardContext,
    sender: broadcast::Sender<ClipFrame>,
    current_content: Arc<Mutex<Vec<u8>>>,
}

impl Manager {
    pub fn new(
        sender: broadcast::Sender<ClipFrame>,
        current_content: Arc<Mutex<Vec<u8>>>,
    ) -> Self {
        let ctx = ClipboardContext::new().unwrap();
        Manager {
            ctx: ctx,
            sender: sender,
            current_content: current_content,
        }
    }
}

impl ClipboardHandler for Manager {
    fn on_clipboard_change(&mut self) {
        //   let types = self.ctx.available_formats().unwrap();
        // let buffer = self.ctx.get_buffer("public.file-url").unwrap();
        // let string = String::from_utf8(buffer).unwrap();
        // let te = self.ctx.get_rich_text();
        // TODO
        let cf = ContentFormat::Text;
        let clip_frame = to_clip_frame(cf, self.ctx.get_buffer("text").unwrap());
        let ss: broadcast::Sender<ClipFrame> = self.sender.clone();
        let lock = Arc::clone(&self.current_content);
        tokio::task::block_in_place(move || {
            let content = lock.lock().unwrap();
            let mut hasher = Md5::new();
            hasher.update(clip_frame.content.get(0).unwrap());
            let hash = hasher.finalize();
            if hash[..].to_vec() != *content {
                let _ = ss.send(clip_frame);
            } else {
                tracing::info!("剪贴板文字与远端一致本次不发送");
            }
        });
    }
}

pub async fn start(sender: broadcast::Sender<ClipFrame>, current_content: Arc<Mutex<Vec<u8>>>) {
    tracing::info!("设置剪贴板监听");
    let manager = Manager::new(sender, current_content);
    let mut watcher = ClipboardWatcherContext::new().unwrap();
    let _shutdown = watcher.add_handler(manager).get_shutdown_channel();
    watcher.start_watch();
    tracing::warn!("设置剪贴板监听结束");
}

pub async fn set_clip(
    mut receiver: broadcast::Receiver<ClipFrame>,
    current_content: Arc<Mutex<Vec<u8>>>,
) {
    tracing::info!("设置剪贴板更新");
    loop {
        let val = receiver.recv().await;
        match val {
            Ok(key) => {
                tracing::info!("收到剪贴板更新");
                let ctx = ClipboardContext::new().unwrap();
                let mut content = current_content.lock().unwrap();
                //计算内容hash，确保不重复更新
                let mut hasher = Md5::new();
                hasher.update(key.content.get(0).unwrap());
                let hash = hasher.finalize();
                *content = hash[..].to_vec();
                let con: ClipFrame = key.clone();
                let (cf, buf) = to_clip(key);
                match cf {
                    clipboard_rs::ContentFormat::Text => {
                        let _ = ctx.set_text(String::from_utf8(buf).unwrap());
                    }
                    // todo
                    _ => {}
                };
            }
            _ => {}
        }
    }
}

fn to_clip(cf: ClipFrame) -> (ContentFormat, Vec<u8>) {
    match cf.frame_type() {
        Frametype::Text => (ContentFormat::Text, cf.content.get(0).unwrap().to_vec()),
        // todo
        _ => (ContentFormat::Text, Vec::new()),
    }
}
fn to_clip_frame(content_format: ContentFormat, buffer: Vec<u8>) -> ClipFrame {
    match content_format {
        clipboard_rs::ContentFormat::Text => {
            let mut ct = Vec::new();
            ct.push(buffer);
            ClipFrame {
                content_num: 1,
                frame_type: 1,
                content: ct,
            }
        }
        // todo
        _ => ClipFrame {
            content_num: 1,
            frame_type: 1,
            content: Vec::new(),
        },
    }
}
