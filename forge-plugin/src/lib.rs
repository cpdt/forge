use crate::config::Config;
use forge_shared::{
    serialize, ClientEvent, ClientPacket, ReceiveBuffer, ServerEvent, ServerPacket,
};
use rrplug::bindings::squirreldatatypes::HSquirrelVM;
use rrplug::prelude::*;
use rrplug::wrappers::northstar::ScriptVmType;
use rrplug::wrappers::squirrel::CSquirrelVMHandle;
use rrplug::{call_sq_function, sq_return_null, sqfunction};
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod config;

#[derive(Debug)]
pub struct ForgePlugin {
    config: Option<Config>,
    sq: Mutex<PluginSqSide>,
    socket: Mutex<PluginSocketSide>,
}

// mega hack
#[derive(Debug, Clone, Copy)]
struct SquirrelVMWrapper(*mut HSquirrelVM);
unsafe impl Send for SquirrelVMWrapper {}

#[derive(Debug)]
struct PluginSqSide {
    server_sqvm: Option<SquirrelVMWrapper>,
    client_sender: Sender<ClientEvent>,
    command_receiver: Receiver<String>,
}

#[derive(Debug)]
struct PluginSocketSide {
    client_receiver: Receiver<ClientEvent>,
    command_sender: Sender<String>,
}

impl Plugin for ForgePlugin {
    fn new() -> Self {
        let (client_sender, client_receiver) = channel();
        let (command_sender, command_receiver) = channel();

        ForgePlugin {
            config: None,

            sq: Mutex::new(PluginSqSide {
                server_sqvm: None,
                client_sender,
                command_receiver,
            }),
            socket: Mutex::new(PluginSocketSide {
                client_receiver,
                command_sender,
            }),
        }
    }

    fn initialize(&mut self, plugin_data: &PluginData) {
        log::info!(
            "Loading config from {}/forge.toml",
            std::env::current_dir().unwrap().display()
        );

        let config_file =
            std::fs::read_to_string("forge.toml").expect("Failed to open `forge.toml`");
        let config = toml::from_str(&config_file).expect("Failed to parse `forge.toml`");
        self.config = Some(config);

        plugin_data.register_sq_functions(info_process).unwrap();
        plugin_data.register_sq_functions(info_game_start).unwrap();
        plugin_data
            .register_sq_functions(info_client_connecting)
            .unwrap();
        plugin_data
            .register_sq_functions(info_client_disconnected)
            .unwrap();
        plugin_data.register_sq_functions(info_client_chat).unwrap();
    }

    fn main(&self) {
        let config = self
            .config
            .as_ref()
            .expect("`main` was called before `initialize`");
        let socket = self.socket.lock().unwrap();

        loop {
            log::info!("Connecting to {}", config.remote);
            let mut stream = match TcpStream::connect(config.remote) {
                Ok(stream) => stream,
                Err(err) => {
                    log::error!("Failed to connect: {}", err);
                    continue;
                }
            };

            std::thread::scope(|s| {
                let has_socket_closed = Arc::new(AtomicBool::new(false));

                let command_sender = socket.command_sender.clone();
                let mut recv_stream = stream.try_clone().unwrap();
                let recv_has_socket_closed = has_socket_closed.clone();

                s.spawn(move || {
                    let mut buffer = ReceiveBuffer::new(|packet: ServerPacket| {
                        let ignore = packet.name.map(|name| name != config.name).unwrap_or(false);
                        if ignore {
                            return;
                        };

                        log::info!("IN {}", packet.event);
                        match packet.event {
                            ServerEvent::ExecCommand { command } => {
                                command_sender
                                    .send(command)
                                    .expect("Failed to send command");
                            }
                        }
                    });

                    while !recv_has_socket_closed.load(Ordering::Acquire) {
                        if let Err(err) = buffer.read(&mut recv_stream) {
                            log::error!("Read error: {}", err);
                            recv_has_socket_closed.store(true, Ordering::Release);
                            break;
                        }
                    }
                });

                // Send loop
                while !has_socket_closed.load(Ordering::Acquire) {
                    let Ok(event) = socket.client_receiver.recv_timeout(Duration::from_secs(5)) else { continue };
                    log::info!("OUT {event}");
                    let packet = ClientPacket {
                        name: config.name.clone(),
                        event,
                    };

                    let serialized = serialize(&packet);
                    if let Err(err) = stream.write_all(&serialized) {
                        log::error!("Write error: {}", err);
                        has_socket_closed.store(true, Ordering::Release);
                        break;
                    }
                }
            });
        }
    }

    fn on_sqvm_created(&self, sqvm_handle: &CSquirrelVMHandle) {
        if sqvm_handle.get_context() == ScriptVmType::Server {
            let mut lock = self.sq.lock().unwrap();
            lock.server_sqvm = Some(SquirrelVMWrapper(unsafe { sqvm_handle.get_sqvm() }));
        }
    }

    fn on_sqvm_destroyed(&self, context: ScriptVmType) {
        if context == ScriptVmType::Server {
            let mut lock = self.sq.lock().unwrap();
            lock.server_sqvm = None;
        }
    }
}

entry!(ForgePlugin);

fn send_client_event(event: ClientEvent) {
    let plugin = PLUGIN.wait();
    let sq = plugin.sq.lock().unwrap();
    sq.client_sender.send(event).expect("Failed to send event");
}

// Called by the ForgeIntegration mod
#[sqfunction(VM=Server, ExportName=ForgePlugin_Process)]
fn process() {
    let plugin = PLUGIN.wait();
    let sq = plugin.sq.lock().unwrap();
    let sqvm = sq
        .server_sqvm
        .expect("`ForgePlugin_Process` was called while SQVM was destroyed?");
    let functions = SQFUNCTIONS.server.wait();

    while let Ok(command) = sq.command_receiver.try_recv() {
        call_sq_function!(sqvm.0, functions, "ServerCommand", command)
            .expect("Failed to run `ServerCommand`");
    }

    sq_return_null!()
}

#[sqfunction(VM=Server, ExportName=ForgePlugin_GameStart)]
fn game_start(map: String, mode: String) {
    send_client_event(ClientEvent::GameStart { map, mode });
    sq_return_null!();
}

#[sqfunction(VM=Server, ExportName=ForgePlugin_ClientConnecting)]
fn client_connecting(name: String, uid: String) {
    send_client_event(ClientEvent::ClientConnecting { name, uid });
    sq_return_null!();
}

#[sqfunction(VM=Server, ExportName=ForgePlugin_ClientDisconnected)]
fn client_disconnected(name: String, uid: String) {
    send_client_event(ClientEvent::ClientDisconnected { name, uid });
    sq_return_null!();
}

#[sqfunction(VM=Server, ExportName=ForgePlugin_ClientChat)]
fn client_chat(name: String, uid: String, message: String, is_team: bool) {
    send_client_event(ClientEvent::ClientChat {
        name,
        uid,
        message,
        is_team,
    });
    sq_return_null!();
}
