use crate::config::{Config, ServerConfig};
use anyhow::Result;
use lazy_static::lazy_static;
use log::{debug, error, info, LevelFilter};
use northstar_rcon_client::{AuthError, ClientRead, ClientWrite};
use regex::Regex;
use serenity::async_trait;
use serenity::builder::CreateInteractionResponse;
use serenity::futures::future::join_all;
use serenity::http::Http;
use serenity::model::application::command::Command;
use serenity::model::application::interaction::application_command::CommandDataOptionValue;
use serenity::model::application::interaction::Interaction;
use serenity::model::prelude::command::CommandOptionType;
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::utils::Color;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::time::sleep;
use tokio::try_join;

mod config;

#[tokio::main]
async fn main() {
    simple_logger::SimpleLogger::new()
        .with_utc_timestamps()
        .with_level(LevelFilter::Off)
        .with_module_level("forge", LevelFilter::Debug)
        .init()
        .unwrap();

    let mut args = std::env::args();
    let exe_name = args.next().unwrap();

    let config_file_path = match args.next() {
        Some(path) => path,
        None => {
            eprintln!("Usage {} [path to config file]", exe_name);
            eprintln!();
            std::process::exit(1);
        }
    };

    info!("Forge {}", env!("CARGO_PKG_VERSION"));

    let full_config_path = std::env::current_dir().unwrap().join(&config_file_path);
    let config = match load_config(&full_config_path) {
        Ok(config) => config,
        Err(err) => {
            error!("Failed to read config file: {}", err);
            std::process::exit(1);
        }
    };

    let config = Box::leak(Box::new(config));

    let mut client = Client::builder(&config.discord_token, GatewayIntents::empty())
        .event_handler(Handler {
            config,
            channel_requests: Mutex::new(HashMap::new()),
        })
        .await
        .expect("Error creating client");

    if let Err(err) = client.start().await {
        error!("Client error: {:?}", err);
        std::process::exit(1);
    }
}

fn load_config(config_path: &Path) -> Result<Config> {
    Ok(toml::from_str(&std::fs::read_to_string(config_path)?)?)
}

struct Handler {
    config: &'static Config,
    channel_requests: Mutex<HashMap<ChannelId, UnboundedSender<ServerRequest>>>,
}

impl Handler {
    async fn send_request_to_channel(
        &self,
        channel: ChannelId,
        request: ServerRequestType,
    ) -> Result<(), ()> {
        let req_receiver = {
            let channels = self.channel_requests.lock().unwrap();
            match channels.get(&channel) {
                Some(sender) => {
                    let (req_sender, req_receiver) = oneshot::channel();

                    sender
                        .send(ServerRequest {
                            ty: request,
                            completed: req_sender,
                        })
                        .unwrap();

                    req_receiver
                }
                None => return Err(()),
            }
        };

        req_receiver.await.map_err(|_| ())
    }

    async fn send_request_to_all_channels(&self, request: ServerRequestType) -> Result<(), ()> {
        let futures = {
            let channels = self.channel_requests.lock().unwrap();
            channels
                .values()
                .map(|sender| {
                    let (req_sender, req_receiver) = oneshot::channel();

                    sender
                        .send(ServerRequest {
                            ty: request.clone(),
                            completed: req_sender,
                        })
                        .unwrap();

                    req_receiver
                })
                .collect::<Vec<_>>()
        };

        let res: Result<Vec<_>, _> = join_all(futures).await.into_iter().collect();

        res.map(|_| ()).map_err(|_| ())
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Connected to Discord as {}", ready.user.name);

        // Register commands
        debug!("Registering commands...");
        Command::set_global_application_commands(&ctx.http, |commands| {
            commands
                .create_application_command(|command| {
                    command
                        .name("exec")
                        .description("Execute a command on a server.")
                        .create_option(|option| {
                            option
                                .name("command")
                                .description("A command to execute.")
                                .kind(CommandOptionType::String)
                                .required(true)
                        })
                })
                .create_application_command(|command| {
                    command
                        .name("execall")
                        .description("Execute a command on all servers.")
                        .create_option(|option| {
                            option
                                .name("command")
                                .description("A command to execute.")
                                .kind(CommandOptionType::String)
                                .required(true)
                        })
                })
        })
        .await
        .unwrap();

        // Start listeners
        debug!("Starting server listeners...");
        let mut requests = self.channel_requests.lock().unwrap();

        for (server_name, server_config) in &self.config.servers {
            let config = self.config;
            let http = ctx.http.clone();

            let (event_sender, event_receiver) = unbounded_channel();
            let (request_sender, request_receiver) = unbounded_channel();

            requests.insert(ChannelId::from(server_config.channel), request_sender);

            tokio::spawn(async move {
                run_server_discord_client(
                    config,
                    server_name,
                    server_config,
                    &http,
                    event_receiver,
                )
                .await;
            });
            tokio::spawn(async move {
                run_server_rcon_client(server_config, event_sender, request_receiver).await;
            });
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let command = match interaction {
            Interaction::ApplicationCommand(command) => command,
            _ => return,
        };

        match command.data.name.as_str() {
            "exec" => {
                let cmd = match command.data.options[0].resolved.as_ref() {
                    Some(CommandDataOptionValue::String(val)) => val,
                    _ => unreachable!(),
                };

                match self
                    .send_request_to_channel(
                        command.channel_id,
                        ServerRequestType::ExecCommand {
                            cmd: cmd.to_string(),
                        },
                    )
                    .await
                {
                    Ok(()) => command
                        .create_interaction_response(&ctx.http, |r| interaction_command(r, cmd))
                        .await
                        .unwrap(),
                    Err(()) => command
                        .create_interaction_response(&ctx.http, |r| {
                            interaction_error(r, "not in a linked channel")
                        })
                        .await
                        .unwrap(),
                }
            }
            "execall" => {
                let cmd = match command.data.options[0].resolved.as_ref() {
                    Some(CommandDataOptionValue::String(val)) => val,
                    _ => unreachable!(),
                };

                match self
                    .send_request_to_all_channels(ServerRequestType::ExecCommand {
                        cmd: cmd.to_string(),
                    })
                    .await
                {
                    Ok(()) => command
                        .create_interaction_response(&ctx.http, |r| interaction_command(r, cmd))
                        .await
                        .unwrap(),
                    Err(()) => command
                        .create_interaction_response(&ctx.http, |r| {
                            interaction_error(r, "not in a linked channel")
                        })
                        .await
                        .unwrap(),
                }
            }
            _ => {}
        }
    }
}

fn interaction_error<'a, 'b>(
    response: &'a mut CreateInteractionResponse<'b>,
    err: &str,
) -> &'a mut CreateInteractionResponse<'b> {
    let err_str = format!("Error: {}", err);
    response.interaction_response_data(|data| {
        data.ephemeral(true)
            .embed(|embed| embed.color(Color::new(0xFF0000)).description(err_str))
    })
}

fn interaction_command<'a, 'b>(
    response: &'a mut CreateInteractionResponse<'b>,
    cmd: &str,
) -> &'a mut CreateInteractionResponse<'b> {
    let str = format!("```{}```", cmd);
    response.interaction_response_data(|data| data.ephemeral(true).content(str))
}

#[derive(Debug)]
enum ServerEvent {
    Connected,
    FailedToConnect {
        reason: String,
    },
    Disconnected {
        reason: String,
    },
    PlayerJoin {
        name: String,
        uid: u64,
    },
    PlayerLeave {
        name: String,
        uid: u64,
    },
    PlayerChat {
        name: String,
        uid: u64,
        message: String,
    },
    GameStart {
        map: String,
        mode: String,
    },
}

#[derive(Debug, Clone)]
enum ServerRequestType {
    ExecCommand { cmd: String },
}

struct ServerRequest {
    ty: ServerRequestType,
    completed: oneshot::Sender<()>,
}

impl Debug for ServerRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRequest")
            .field("ty", &self.ty)
            .finish()
    }
}

async fn run_server_discord_client(
    config: &Config,
    name: &str,
    server_config: &ServerConfig,
    http: &Http,
    mut events: UnboundedReceiver<ServerEvent>,
) {
    let channel = ChannelId::from(server_config.channel);
    loop {
        let res = match events.recv().await {
            Some(ServerEvent::Connected) => channel
                .send_message(http, |m| {
                    m.embed(|embed| embed.description(format!("Connected to **{}**.", name)))
                })
                .await
                .map(|_| ()),
            Some(ServerEvent::FailedToConnect { reason }) => channel
                .send_message(http, |m| {
                    m.embed(|embed| {
                        embed.description(format!("Failed to connect to **{}**: {}", name, reason))
                    })
                })
                .await
                .map(|_| ()),
            Some(ServerEvent::Disconnected { reason }) => channel
                .send_message(http, |m| {
                    m.embed(|embed| {
                        embed.description(format!("Disconnected from **{}**: {}", name, reason))
                    })
                })
                .await
                .map(|_| ()),
            Some(ServerEvent::PlayerJoin { name, .. }) => channel
                .send_message(http, |m| {
                    m.embed(|embed| embed.description(format!("**{}** joined.", name)))
                })
                .await
                .map(|_| ()),
            Some(ServerEvent::PlayerLeave { name, .. }) => channel
                .send_message(http, |m| {
                    m.embed(|embed| embed.description(format!("**{}** left.", name)))
                })
                .await
                .map(|_| ()),
            Some(ServerEvent::PlayerChat { name, message, .. }) => channel
                .send_message(http, |m| m.content(format!("**{}**: {}", name, message)))
                .await
                .map(|_| ()),
            Some(ServerEvent::GameStart { map, mode }) => {
                let map_en = config
                    .maps
                    .get(&map)
                    .cloned()
                    .unwrap_or_else(|| format!("`{}`", map));
                let mode_en = config
                    .modes
                    .get(&mode)
                    .cloned()
                    .unwrap_or_else(|| format!("`{}`", mode));

                channel
                    .send_message(http, |m| {
                        m.embed(|embed| {
                            embed
                                .description(format!("Starting **{}** on **{}**.", mode_en, map_en))
                        })
                    })
                    .await
                    .map(|_| ())
            }
            None => return,
        };

        if let Err(err) = res {
            error!("Failed to send Discord message: {}", err);
        }
    }
}

async fn run_server_rcon_client(
    server_config: &ServerConfig,
    events: UnboundedSender<ServerEvent>,
    mut requests: UnboundedReceiver<ServerRequest>,
) {
    loop {
        let can_reconnect =
            run_rcon_client_until_disconnected(server_config, &events, &mut requests).await;
        if !can_reconnect {
            break;
        }

        debug!("Reconnecting in 5s...");
        sleep(Duration::from_secs(5)).await;
    }
}

lazy_static! {
    static ref PLAYER_JOIN: Regex =
        Regex::new(r#"^\[SERVER SCRIPT\] dev.cpdt.forge:playerJoin name="(.*)" uid="(.*)"$"#)
            .unwrap();
    static ref PLAYER_LEAVE: Regex =
        Regex::new(r#"^\[SERVER SCRIPT\] dev.cpdt.forge:playerLeave name="(.*)" uid="(.*)"$"#)
            .unwrap();
    static ref PLAYER_CHAT: Regex = Regex::new(
        r#"^\[SERVER SCRIPT\] dev.cpdt.forge:playerChat name="(.*)" uid="(.*)" message=(.*)$"#
    )
    .unwrap();
    static ref GAME_START: Regex =
        Regex::new(r#"^\[SERVER SCRIPT\] dev.cpdt.forge:gameStart map="(.*)" mode="(.*)"$"#)
            .unwrap();
}

async fn run_rcon_client_until_disconnected(
    server_config: &ServerConfig,
    events: &UnboundedSender<ServerEvent>,
    requests: &mut UnboundedReceiver<ServerRequest>,
) -> bool {
    let client = match northstar_rcon_client::connect(&server_config.address).await {
        Ok(client) => client,
        Err(err) => {
            error!("Failed to connect to {}: {}", server_config.address, err);
            return true;
        }
    };

    let (read, write) = match client.authenticate(&server_config.password).await {
        Ok(pair) => pair,
        Err((_, AuthError::Fatal(err))) => {
            error!(
                "Error while authenticating with {}: {}",
                server_config.address, err
            );
            return true;
        }
        Err((_, auth_error)) => {
            error!(
                "Failed to authenticate with {}: {}",
                server_config.address, auth_error
            );
            events
                .send(ServerEvent::FailedToConnect {
                    reason: format!("{}", auth_error),
                })
                .unwrap();
            return false;
        }
    };

    info!("Connected to {}", server_config.address);

    events.send(ServerEvent::Connected).unwrap();

    if let Err(err) = run_rcon_client_post_auth(read, write, events, requests).await {
        error!(
            "Error while connected to {}: {}",
            server_config.address, err
        );
        events
            .send(ServerEvent::Disconnected {
                reason: format!("{}", err),
            })
            .unwrap();
    }

    true
}

async fn run_rcon_client_post_auth(
    read: ClientRead,
    mut write: ClientWrite,
    events: &UnboundedSender<ServerEvent>,
    requests: &mut UnboundedReceiver<ServerRequest>,
) -> northstar_rcon_client::Result<()> {
    write.enable_console_logs().await?;

    let recv_thread = rcon_recv_thread(read, events);
    let send_thread = rcon_send_thread(write, requests);

    try_join!(recv_thread, send_thread)?;
    Ok(())
}

async fn rcon_recv_thread(
    mut read: ClientRead,
    events: &UnboundedSender<ServerEvent>,
) -> northstar_rcon_client::Result<()> {
    loop {
        let log = read.receive_console_log().await?;

        if let Some(captures) = PLAYER_JOIN.captures(&log) {
            events
                .send(ServerEvent::PlayerJoin {
                    name: captures.get(1).unwrap().as_str().to_string(),
                    uid: captures.get(2).unwrap().as_str().parse().unwrap(),
                })
                .unwrap();
        } else if let Some(captures) = PLAYER_LEAVE.captures(&log) {
            events
                .send(ServerEvent::PlayerLeave {
                    name: captures.get(1).unwrap().as_str().to_string(),
                    uid: captures.get(2).unwrap().as_str().parse().unwrap(),
                })
                .unwrap();
        } else if let Some(captures) = PLAYER_CHAT.captures(&log) {
            events
                .send(ServerEvent::PlayerChat {
                    name: captures.get(1).unwrap().as_str().to_string(),
                    uid: captures.get(2).unwrap().as_str().parse().unwrap(),
                    message: captures.get(3).unwrap().as_str().to_string(),
                })
                .unwrap();
        } else if let Some(captures) = GAME_START.captures(&log) {
            events
                .send(ServerEvent::GameStart {
                    map: captures.get(1).unwrap().as_str().to_string(),
                    mode: captures.get(2).unwrap().as_str().to_string(),
                })
                .unwrap();
        }
    }
}

async fn rcon_send_thread(
    mut write: ClientWrite,
    requests: &mut UnboundedReceiver<ServerRequest>,
) -> northstar_rcon_client::Result<()> {
    loop {
        let request = requests.recv().await.unwrap();

        let result = match request.ty {
            ServerRequestType::ExecCommand { cmd } => write.exec_command(&cmd).await,
        };
        let _ = request.completed.send(());

        if let Err(err) = result {
            return Err(err);
        }
    }
}
