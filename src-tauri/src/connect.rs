use crate::clip::clip_frame::{self, ClipFrame};
use crate::config::FILE_WATCHER;
use configparser::ini::Ini;
use futures::channel::oneshot;
use prost::Message;
use std::collections::HashMap;
use std::io::Cursor;
use tokio::io::{self, AsyncReadExt, Interest};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinSet;

#[derive(Clone, Debug)]
pub struct RemoteConnecter {
    host: String,
    port: String,
    pub switch: broadcast::Sender<i32>,
    reader_sender: broadcast::Sender<ClipFrame>,
    writer_sender: broadcast::Sender<ClipFrame>,
}

impl RemoteConnecter {
    pub fn new(
        host: String,
        port: String,
        reader_sender: broadcast::Sender<ClipFrame>,
        writer_sender: broadcast::Sender<ClipFrame>,
    ) -> Self {
        let (switch_tx, _) = broadcast::channel(5);
        RemoteConnecter {
            host,
            port,
            switch: switch_tx,
            reader_sender,
            writer_sender,
        }
    }
}
pub trait RWChannel {
    fn get_hook(&self) -> (broadcast::Sender<ClipFrame>, broadcast::Receiver<ClipFrame>);
}

impl RWChannel for RemoteConnecter {
    fn get_hook(&self) -> (broadcast::Sender<ClipFrame>, broadcast::Receiver<ClipFrame>) {
        (self.writer_sender.clone(), self.reader_sender.subscribe())
    }
}

pub trait SocketConnecter {
    fn do_server(&self) -> impl std::future::Future<Output = ()> + Send;
    fn do_client(&self) -> impl std::future::Future<Output = ()> + Send;
    fn socket_read(
        read: OwnedReadHalf,
        sender_read: broadcast::Sender<ClipFrame>,
    ) -> impl std::future::Future<Output = ()> + Send;
    fn socket_write(
        write: OwnedWriteHalf,
        receiver: broadcast::Receiver<ClipFrame>,
    ) -> impl std::future::Future<Output = ()> + Send;
}

impl SocketConnecter for RemoteConnecter {
    async fn do_server(&self) {
        tracing::info!("socket server线程开始");
        let mut addr = String::from("");
        addr.push_str(self.host.as_str());
        addr.push(':');
        addr.push_str(self.port.as_str());
        let listener = TcpListener::bind(addr).await.unwrap();
        loop {
            let (socket, addr) = listener.accept().await.unwrap();
            tracing::info!("new client: {:?}", addr);
            let (read_half, write_half) = socket.into_split();
            let new_sender = self.reader_sender.clone();
            let mut new_receiver = self.writer_sender.subscribe();
            tokio::select! {
                           msg=async {
            let mut recv=self.switch.subscribe();
            recv.recv().await.unwrap()
                           }=>{
                            tracing::warn!("socket server 关闭");
                           }
                            _ = async {
                                let mut set = JoinSet::new();
                                set.spawn(RemoteConnecter::socket_read(read_half, new_sender));
                                set.spawn(RemoteConnecter::socket_write(write_half, new_receiver));
                                while let Some(res) = set.join_next().await {
                                    let _ = res.unwrap();
                                }
                            }=>{

                             }
                        }
        }
    }

    async fn do_client(&self) {
        tracing::info!("socket client线程开始");
        let mut addr = String::from("");
        addr.push_str(self.host.as_str());
        addr.push(':');
        addr.push_str(self.port.as_str());

        match TcpStream::connect(addr).await {
            Ok(socket) => {
                let (read, write) = socket.into_split();
                let mut set = JoinSet::new();
                set.spawn(RemoteConnecter::socket_read(
                    read,
                    self.reader_sender.clone(),
                ));
                set.spawn(RemoteConnecter::socket_write(
                    write,
                    self.writer_sender.subscribe(),
                ));
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
}

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
    tracing::info!("socket接收线程开始");
    let mut is_connect = true;
    while is_connect {
        let ready = read.ready(Interest::READABLE).await.unwrap();
        let mut full_data = Vec::new();
        while ready.is_readable() {
            let mut tmp_data = Vec::new();
            match read.try_read_buf(&mut tmp_data) {
                Ok(size) => {
                    if size == 0 {
                        tracing::info!("客户端断开链接");
                        is_connect = false;
                        break;
                    } else {
                        full_data.append(&mut tmp_data);
                    }
                }
                Err(e) => match e {
                    _WouldBlock => {
                        tracing::info!("远端无数据，停止读取");
                        break;
                    }
                    _ => {
                        break;
                    }
                },
            }
        }
        if !is_connect {
            break;
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
    tracing::info!("socket接收线程结束");
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

pub fn setup(
    config: &'static Ini,
    reader_sender: broadcast::Sender<ClipFrame>,
    writer_sender: broadcast::Sender<ClipFrame>,
) -> RemoteConnecter {
    let host = config.get("default", "host").unwrap();
    let port = config.get("default", "port").unwrap();
    let clip = RemoteConnecter::new(host, port, reader_sender, writer_sender);
    clip
}

pub async fn start(connector: RemoteConnecter, mode: String) {
    tokio::spawn(async {
        let mut config_changer = unsafe { FILE_WATCHER.as_ref().unwrap().subscribe() };
        match config_changer.changed().await {
            Ok(a) => {
                // let config = config_changer.borrow();
            }
            Err(_) => {}
        }
    });
    if mode.eq_ignore_ascii_case("client") {
        connector.do_client().await;
    } else if mode.eq_ignore_ascii_case("server") {
        connector.do_server().await;
    } else {
        tracing::info!("unknown mode: {}", mode);
        return;
    }
}
