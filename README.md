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

The bot requires a Northstar mod to be installed on your server to facilitate communication over RCON.
Copy `Snnag.ForgeServer-1.0.0` into your server's mod folder to install.

## Configuration

Using `config.examle.toml` as a base, fill out necessary fields:

 - `discord-token` is your Discord bot token.
 - `discord-application` is your Discord application ID.

Each Northstar server you want to control needs a section with these fields:

 - `address` is the address to the server, including the RCON port.
 - `password` is the RCON password.
 - `channel` is the Discord channel that this bot will be linked to.

## License

Provided under the MIT license. Check the LICENSE file for details.
