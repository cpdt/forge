global function InitForgeServer

void function HandleClientConnecting(entity player) {
    print("dev.cpdt.forge:playerJoin name=\"" + player.GetPlayerName() + "\" uid=\"" + player.GetUID() + "\"")
}

void function HandleClientDisconnected(entity player) {
    print("dev.cpdt.forge:playerLeave name=\"" + player.GetPlayerName() + "\" uid=\"" + player.GetUID() + "\"")
}

ClServer_MessageStruct function HandleReceivedChat(ClServer_MessageStruct message) {
    print("dev.cpdt.forge:playerChat name=\"" + message.player.GetPlayerName() + "\" uid=\"" + message.player.GetUID() + "\" message=" + message.message)
    return message
}

void function InitForgeServer() {
    AddCallback_OnClientConnecting(HandleClientConnecting)
    AddCallback_OnClientDisconnected(HandleClientDisconnected)
    AddCallback_OnReceivedSayTextMessage(HandleReceivedChat)

    string map = GetMapName()
    string mode = GameRules_GetGameMode()
    print("dev.cpdt.forge:gameStart map=\"" + map + "\" mode=\"" + mode + "\"")
}
