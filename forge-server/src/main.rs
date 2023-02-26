use crate::config::Config;
use crate::server::Server;
use anyhow::Result;
use forge_shared::{ClientEvent, ClientPacket, ServerEvent, ServerPacket};
use log::{debug, error, info, warn, LevelFilter};
use serenity::async_trait;
use serenity::model::prelude::*;
use serenity::prelude::*;
use serenity::utils::Color;
use std::path::Path;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::{join, try_join};

mod config;
mod server;

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

    let Some(config_file_path) = args.next() else {
        eprintln!("Usage {} [path to config file]", exe_name);
        eprintln!();
        std::process::exit(1);
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

    let (client_sender, client_receiver) = unbounded_channel();
    let (server_sender, server_receiver) = unbounded_channel();

    join!(
        run_server(config, client_sender, server_receiver),
        run_client(config, client_receiver, server_sender),
    );
}

fn load_config(config_path: &Path) -> Result<Config> {
    Ok(toml::from_str(&std::fs::read_to_string(config_path)?)?)
}

async fn run_server(
    config: &'static Config,
    client_sender: UnboundedSender<ClientPacket>,
    mut server_receiver: UnboundedReceiver<ServerPacket>,
) {
    let server = Server::new(config.listen)
        .await
        .expect("Error starting server");
    info!("Listening on {}", server.local_addr().unwrap());

    let send_loop = async {
        loop {
            let Some(packet) = server_receiver.recv().await else { break };
            server.send(&packet).await;
        }
    };

    join!(server.receive(client_sender), send_loop,);
}

async fn run_client(
    config: &'static Config,
    client_receiver: UnboundedReceiver<ClientPacket>,
    server_sender: UnboundedSender<ServerPacket>,
) {
    let mut client = Client::builder(&config.discord_token, GatewayIntents::empty())
        .event_handler(Handler {
            config,
            server_sender,
        })
        .await
        .expect("Error creating client");

    let http = client.cache_and_http.http.clone();
    let display_loop = async move {
        run_client_display_loop(config, http.as_ref(), client_receiver).await;
        Ok(())
    };
    let client_start = client.start();

    if let Err(err) = try_join!(display_loop, client_start) {
        error!("Client error: {:?}", err);
        std::process::exit(1);
    }
}

async fn run_client_display_loop(
    config: &'static Config,
    http: &serenity::http::Http,
    mut client_receiver: UnboundedReceiver<ClientPacket>,
) {
    loop {
        let packet = client_receiver
            .recv()
            .await
            .expect("Failed to receive packet");
        let Some(server_config) = config.servers.get(&packet.name) else {
            warn!("Event from unknown client \"{}\": {}", packet.name, packet.event);
            continue;
        };
        let channel = ChannelId(server_config.channel);

        let res = match packet.event {
            ClientEvent::GameStart { map, mode } => {
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
                            embed.description(format!("Starting **{mode_en}** on **{map_en}**."))
                        })
                    })
                    .await
                    .map(|_| ())
            }
            ClientEvent::ClientConnecting { name, uid } => channel
                .send_message(http, |m| {
                    m.embed(|embed| embed.description(format!("**{name}** (`{uid}`) joined.")))
                })
                .await
                .map(|_| ()),
            ClientEvent::ClientDisconnected { name, uid } => channel
                .send_message(http, |m| {
                    m.embed(|embed| embed.description(format!("**{name}** (`{uid}`) left.")))
                })
                .await
                .map(|_| ()),
            ClientEvent::ClientChat {
                name,
                message,
                is_team,
                ..
            } => channel
                .send_message(http, |m| {
                    m.content(format!(
                        "{}**{name}**: {message}",
                        if is_team { "[TEAM] " } else { "" }
                    ))
                })
                .await
                .map(|_| ()),
        };

        if let Err(err) = res {
            error!("Failed to send Discord message: {}", err);
        }
    }
}

struct Handler {
    config: &'static Config,
    server_sender: UnboundedSender<ServerPacket>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Connected to Discord as {}", ready.user.name);

        // Register commands
        debug!("Registering commands...");
        command::Command::set_global_application_commands(&ctx.http, |commands| {
            commands
                .create_application_command(|command| {
                    command
                        .name("exec")
                        .description("Execute a command on a server.")
                        .create_option(|option| {
                            option
                                .name("command")
                                .description("A command to execute.")
                                .kind(command::CommandOptionType::String)
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
                                .kind(command::CommandOptionType::String)
                                .required(true)
                        })
                })
        })
        .await
        .unwrap();

        debug!("😎");
    }

    async fn interaction_create(&self, ctx: Context, interaction: interaction::Interaction) {
        let command = match interaction {
            interaction::Interaction::ApplicationCommand(command) => command,
            _ => return,
        };

        match command.data.name.as_str() {
            "exec" => {
                let cmd = match command.data.options[0].resolved.as_ref() {
                    Some(interaction::application_command::CommandDataOptionValue::String(val)) => {
                        val
                    }
                    _ => unreachable!(),
                };

                let client_name = self
                    .config
                    .servers
                    .iter()
                    .find(|(_, config)| config.channel == command.channel_id.0)
                    .map(|(name, _)| name);
                match client_name {
                    Some(name) => {
                        self.server_sender
                            .send(ServerPacket {
                                name: Some(name.to_string()),
                                event: ServerEvent::ExecCommand {
                                    command: cmd.clone(),
                                },
                            })
                            .expect("Failed to send server packet");

                        command
                            .create_interaction_response(&ctx.http, |r| interaction_command(r, cmd))
                            .await
                            .unwrap();
                    }
                    None => {
                        command
                            .create_interaction_response(&ctx.http, |r| {
                                interaction_error(r, "not in a linked channel")
                            })
                            .await
                            .unwrap();
                    }
                }
            }
            "execall" => {
                let cmd = match command.data.options[0].resolved.as_ref() {
                    Some(interaction::application_command::CommandDataOptionValue::String(val)) => {
                        val
                    }
                    _ => unreachable!(),
                };

                self.server_sender
                    .send(ServerPacket {
                        name: None,
                        event: ServerEvent::ExecCommand {
                            command: cmd.clone(),
                        },
                    })
                    .expect("Failed to send server packet");

                command
                    .create_interaction_response(&ctx.http, |r| interaction_command(r, cmd))
                    .await
                    .unwrap();
            }
            _ => {}
        }
    }
}

fn interaction_error<'a, 'b>(
    response: &'a mut serenity::builder::CreateInteractionResponse<'b>,
    err: &str,
) -> &'a mut serenity::builder::CreateInteractionResponse<'b> {
    let err_str = format!("Error: {}", err);
    response.interaction_response_data(|data| {
        data.ephemeral(true)
            .embed(|embed| embed.color(Color::new(0xFF0000)).description(err_str))
    })
}

fn interaction_command<'a, 'b>(
    response: &'a mut serenity::builder::CreateInteractionResponse<'b>,
    cmd: &str,
) -> &'a mut serenity::builder::CreateInteractionResponse<'b> {
    let str = format!("```{}```", cmd);
    response.interaction_response_data(|data| data.ephemeral(true).content(str))
}
