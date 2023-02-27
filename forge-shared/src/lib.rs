use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ClientEvent {
    GameStart {
        map: String,
        mode: String,
    },
    ClientConnecting {
        name: String,
        uid: String,
    },
    ClientDisconnected {
        name: String,
        uid: String,
    },
    ClientChat {
        name: String,
        uid: String,
        message: String,
        is_team: bool,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ClientPacket {
    pub name: String,
    pub event: ClientEvent,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ServerEvent {
    ExecCommand { command: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerPacket {
    pub name: Option<String>,
    pub event: ServerEvent,
}

impl std::fmt::Display for ClientEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientEvent::GameStart { map, mode } => write!(f, "GameStart map={map}, mode={mode}"),
            ClientEvent::ClientConnecting { name, uid } => {
                write!(f, "ClientConnecting name={name}, uid={uid}")
            }
            ClientEvent::ClientDisconnected { name, uid } => {
                write!(f, "ClientDisconnected name={name}, uid={uid}")
            }
            ClientEvent::ClientChat {
                name, uid, message, ..
            } => write!(f, "ClientChat name={name}, uid={uid}, message={message}"),
        }
    }
}

impl std::fmt::Display for ServerEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerEvent::ExecCommand { command } => write!(f, "ExecCommand command={command}"),
        }
    }
}

pub struct ReceiveBuffer<T, F> {
    data: Vec<u8>,
    on_parsed: F,

    _items: PhantomData<T>,
}

impl<T: DeserializeOwned, F: FnMut(T)> ReceiveBuffer<T, F> {
    pub fn new(on_parsed: F) -> Self {
        ReceiveBuffer {
            data: Vec::new(),
            on_parsed,

            _items: PhantomData::default(),
        }
    }

    pub fn read<R: std::io::Read>(&mut self, mut r: R) -> std::io::Result<()> {
        let mut read = self.start_read();
        let write_len = r.read(read.data())?;
        if write_len == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into())
        }

        read.finish(write_len);
        Ok(())
    }

    pub fn start_read(&mut self) -> ReceiveBufferRead<T, F> {
        let start_index = self.data.len();
        self.data.resize(start_index + 8192, 0);

        ReceiveBufferRead {
            buffer: self,
            start_index,
        }
    }
}

pub struct ReceiveBufferRead<'b, T, F> {
    buffer: &'b mut ReceiveBuffer<T, F>,
    start_index: usize,
}

impl<'b, T: DeserializeOwned, F: FnMut(T)> ReceiveBufferRead<'b, T, F> {
    pub fn data(&mut self) -> &mut [u8] {
        &mut self.buffer.data[self.start_index..]
    }

    pub fn finish(self, write_len: usize) {
        let buffer = self.buffer;
        buffer.data.truncate(self.start_index + write_len);

        let mut read_index = 0;
        while read_index < buffer.data.len() {
            let read_slice = &buffer.data[read_index..];
            if read_slice.len() < std::mem::size_of::<u32>() {
                break;
            }

            let (len_bytes, remaining_bytes) = read_slice.split_at(std::mem::size_of::<u32>());
            let len = u32::from_ne_bytes(len_bytes.try_into().unwrap()) as usize;

            if remaining_bytes.len() < len {
                break;
            }

            let read_slice = &remaining_bytes[..len];
            read_index += std::mem::size_of::<u32>() + remaining_bytes.len();

            let val = bincode::deserialize(read_slice).expect("bincode deserialize failed");
            (buffer.on_parsed)(val);
        }

        buffer.data.drain(..read_index);
    }
}

pub fn serialize<T: Serialize>(val: &T) -> Vec<u8> {
    let u32_size = std::mem::size_of::<u32>();
    let mut data = vec![0; u32_size];

    bincode::serialize_into(&mut data, val).expect("bincode serialize failed");
    let val_size = data.len() - u32_size;

    data[..u32_size].copy_from_slice(&(val_size as u32).to_ne_bytes());
    data
}
