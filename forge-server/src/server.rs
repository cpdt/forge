use forge_shared::{serialize, ClientPacket, ReceiveBuffer, ServerPacket};
use log::{debug, error, info};
use serenity::futures::future::join_all;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpListener;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

struct Stream {
    id: u64,
    write: OwnedWriteHalf,
    read: JoinHandle<()>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.read.abort();
    }
}

pub struct Server {
    next_id: AtomicU64,
    listener: TcpListener,
    streams: Arc<Mutex<Vec<Stream>>>,
}

impl Server {
    pub async fn new(addr: SocketAddr) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Server {
            next_id: AtomicU64::new(0),
            listener,
            streams: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub async fn receive(&self, sender: UnboundedSender<ClientPacket>) {
        loop {
            let (socket, addr) = match self.listener.accept().await {
                Ok((socket, addr)) => (socket, addr),
                Err(err) => {
                    error!("Accept error: {err}");
                    continue;
                }
            };
            debug!("Connection from {addr}");

            let (read_half, write_half) = socket.into_split();

            let stream_id = self.next_id.fetch_add(1, Ordering::AcqRel);

            let streams = Arc::downgrade(&self.streams);
            let sender = sender.clone();

            let read = tokio::spawn(async move {
                if let Err(err) = stream_read_loop(read_half, sender).await {
                    error!("{addr} read error: {err}");

                    // Remove the error stream
                    if let Some(streams) = streams.upgrade() {
                        let mut streams = streams.lock().await;
                        streams.retain(|write| write.id != stream_id);
                        info!("{} client(s) connected", streams.len());
                    }
                }
            });

            self.push_stream(stream_id, write_half, read);
        }
    }

    pub async fn send(&self, packet: &ServerPacket) {
        debug!(
            "OUT ({}) {}",
            packet
                .name
                .as_ref()
                .map(|name| name as &str)
                .unwrap_or("<everyone>"),
            packet.event
        );
        let serialized = serialize(packet);

        let mut streams = self.streams.lock().await;

        let results = join_all(
            streams
                .iter_mut()
                .map(|stream| stream.write.write_all(&serialized)),
        )
        .await;

        // Remove any streams that had write errors
        let mut index = 0;
        streams.retain(|write_half| {
            let res = &results[index];
            index += 1;

            if let Err(err) = res {
                error!(
                    "{} write error: {}",
                    write_half.write.local_addr().unwrap(),
                    err
                );
            }

            res.is_ok()
        });

        if streams.len() != results.len() {
            info!("{} client(s) connected", streams.len());
        }
    }

    fn push_stream(&self, id: u64, write: OwnedWriteHalf, read: JoinHandle<()>) {
        let mut streams = self.streams.blocking_lock();
        streams.push(Stream { id, write, read });
        info!("{} client(s) connected", streams.len());
    }
}

async fn stream_read_loop(
    mut read_half: OwnedReadHalf,
    sender: UnboundedSender<ClientPacket>,
) -> std::io::Result<()> {
    let mut buffer = ReceiveBuffer::new(|packet: ClientPacket| {
        debug!("IN ({}) {}", packet.name, packet.event);
        sender.send(packet).expect("Failed to send packet");
    });

    loop {
        let mut read = buffer.start_read();
        let write_len = read_half.read(read.data()).await?;
        read.finish(write_len);
    }
}
