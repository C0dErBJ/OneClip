use crate::clip::clip_frame::{self, ClipFrame};
use prost::Message;
use std::io::Cursor;
use tokio::io::Interest;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::task::JoinSet;

pub async fn do_server(
    port: String,
    sender_read: broadcast::Sender<ClipFrame>,
    sender_write: broadcast::Sender<ClipFrame>,
) {
    tracing::info!("socket server线程开始");
    let mut addr = String::from("127.0.0.1:");
    addr.push_str(&port);
    let listener = TcpListener::bind(addr).await.unwrap();
    loop {
        let (socket, addr) = listener.accept().await.unwrap();
        tracing::info!("new client: {:?}", addr);
        let (read_half, write_half) = socket.into_split();
        let new_sender = sender_read.clone();
        let mut new_receiver = sender_write.subscribe();
        tokio::spawn(async move {
            let mut set = JoinSet::new();
            set.spawn(socket_read(read_half, new_sender));
            set.spawn(socket_write(write_half, new_receiver));
            while let Some(res) = set.join_next().await {
                let _ = res.unwrap();
            }
        });
    }
}

pub async fn do_client(
    addr: String,
    sender_read: broadcast::Sender<ClipFrame>,
    sender_write: broadcast::Sender<ClipFrame>,
) {
    tracing::info!("socket client线程开始");
    match TcpStream::connect(addr).await {
        Ok(socket) => {
            let (read, write) = socket.into_split();
            let mut set = JoinSet::new();
            set.spawn(socket_read(read, sender_read));
            set.spawn(socket_write(write, sender_write.subscribe()));
            while let Some(res) = set.join_next().await {
                let _ = res.unwrap();
            }
        }
        Err(e) => {
            tracing::error!("无法连接到服务端[{}]", e);
        }
    }
    tracing::info!("socket client线程结束");
}

async fn socket_read(read: OwnedReadHalf, sender_read: broadcast::Sender<ClipFrame>) {
    loop {
        tracing::info!("socket接收线程开始");
        let ready = read.ready(Interest::READABLE).await.unwrap();
        if ready.is_readable() {
            let mut data = Vec::new();
            match read.try_read_buf(&mut data) {
                Ok(size) => {
                    if size == 0 {
                        tracing::info!("客户端断开链接");
                        break;
                    } else {
                        tracing::info!("收到远端信息");
                        match clip_frame::ClipFrame::decode(&mut Cursor::new(data)) {
                            Ok(buf) => {
                                let _ = sender_read.send(buf);
                            }
                            Err(_) => {
                                tracing::warn!("远端消息解析有误");
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
async fn socket_write(write: OwnedWriteHalf, mut receiver: broadcast::Receiver<ClipFrame>) {
    tracing::info!("socket发送线程开始");
    loop {
        let val = receiver.recv().await;
        match val {
            Ok(key) => {
                tracing::info!("准备发送");
                let ready = write.ready(Interest::WRITABLE).await.unwrap();
                if ready.is_writable() {
                    let mut buf = Vec::new();
                    buf.reserve(key.encoded_len());
                    key.encode(&mut buf).unwrap();
                    match write.try_write(&buf) {
                        Ok(_) => {
                            tracing::info!("成功发送至对方")
                        }
                        Err(_) => {}
                    }
                }
            }
            _ => {}
        }
    }
}
