# forge

This is a Discord bot for Northstar chat relay and server moderation, written in Rust.

## Features

 - Echos chat and certain in-game events (currently game start, player join and player leave). Chat is only one-way at
   the moment.
 - Supports multiple bots, each with their own channel.
 - Uses Discord application commands to execute commands on each server, so you can use Discord's command permission
   system.

## Commands

 - `/exec <command>` executes a command on the server that's linked to the channel this command is sent in.
 - `/execall <command>` executes a command on all servers.

## Installation

The bot requires a plugin and a Northstar mod to be installed on your server to facilitate communication. It also needs
a version of Northstar supporting [plugins v2](https://github.com/R2Northstar/NorthstarLauncher/pull/343).

Build `forge-plugin` and copy `forge-plugin.dll` into your server's plugins directory, then copy
`Snnag.ForgeIntegration-1.0.0` into your server's mod folder.

## Configuration

There are two separate configuration files: one for the plugin and one for the server.

### Plugin configuration

Copy `forge.example.toml` to the same directory as your Northstar installation, and rename it `forge.toml`. Replace the
name with your own identifier, and set `remote` to point at your Forge server.

### Server configuration

Using `config.examle.toml` as a base, fill out necessary fields:

 - `listen` is the socket address the server listens on for connections from `forge-plugin`.
 - `discord-token` is your Discord bot token.
 - `discord-application` is your Discord application ID.

Each Northstar server you want to control needs a section with these fields:

 - `channel` is the Discord channel that this bot will be linked to.

The names of the servers in `config.toml` should match the names set in each `forge.toml` file.

## License

Provided under the MIT license. Check the LICENSE file for details.
