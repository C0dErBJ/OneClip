use crate::clip::clip_frame::{self, ClipFrame};
use prost::Message;
use std::io::Cursor;
use tokio::io::{self, AsyncReadExt, Interest};
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
        let mut full_data = Vec::new();
        while ready.is_readable() {
            let mut tmp_data = Vec::new();
            match read.try_read_buf(&mut tmp_data) {
                Ok(size) => {
                    if size == 0 {
                        tracing::info!("客户端断开链接");
                        break;
                    } else {
                        full_data.append(&mut tmp_data);
                    }
                }
                Err(e) => match e {
                    WouldBlock => {
                        tracing::info!("远端无数据，停止读取");
                        break;
                    }
                    _ => {
                        break;
                    }
                },
            }
        }
        tracing::info!("准备解析，总计{}", full_data.len());
        //这段转换正式使用可以去掉
        let hex_string = String::from_utf8(full_data).unwrap();
        let plain_bytes: Option<Vec<u8>> = (0..hex_string.len())
            .step_by(2)
            .map(|i| {
                hex_string
                    .get(i..i + 2)
                    .and_then(|sub| u8::from_str_radix(sub, 16).ok())
            })
            .collect();
        //这段转换正式使用可以去掉
        match clip_frame::ClipFrame::decode(&mut Cursor::new(plain_bytes.unwrap())) {
            Ok(buf) => {
                let _ = sender_read.send(buf);
            }
            Err(e) => {
                tracing::warn!("远端消息解析有误{}", e);
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
                    //这段转换正式使用可以去掉
                    let hex_string = hex::encode(buf);
                    //这段转换正式使用可以去掉
                    match write.try_write(hex_string.as_bytes()) {
                        Ok(size) => {
                            tracing::info!("成功发送至对方，总计{}", size)
                        }
                        Err(_) => {}
                    }
                }
            }
            _ => {}
        }
    }
}
