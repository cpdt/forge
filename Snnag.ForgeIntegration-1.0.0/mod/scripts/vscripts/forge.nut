global function ForgeIntegration_Init

void function HandleClientConnecting(entity player) {
    ForgePlugin_ClientConnecting(player.GetPlayerName(), player.GetUID())
}

void function HandleClientDisconnected(entity player) {
    ForgePlugin_ClientDisconnected(player.GetPlayerName(), player.GetUID())
}

ClServer_MessageStruct function HandleReceivedChat(ClServer_MessageStruct message) {
    ForgePlugin_ClientChat(message.player.GetPlayerName(), message.player.GetUID(), message.message, message.isTeam)
    return message
}

void function ProcessLoop() {
    while (true) {
        ForgePlugin_Process()
        WaitFrame()
    }
}

void function ForgeIntegration_Init() {
    AddCallback_OnClientConnecting(HandleClientConnecting)
    AddCallback_OnClientDisconnected(HandleClientDisconnected)
    AddCallback_OnReceivedSayTextMessage(HandleReceivedChat)

    string map = GetMapName()
    string mode = GameRules_GetGameMode()
    ForgePlugin_GameStart(map, mode)

    thread ProcessLoop()
}
