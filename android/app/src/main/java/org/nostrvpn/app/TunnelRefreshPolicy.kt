package org.nostrvpn.app

internal object TunnelRefreshPolicy {
    private val networkActions = setOf(
        "import_join_request",
        "manual_add_network",
        "add_network",
        "rename_network",
        "remove_network",
        "set_network_enabled",
        "set_network_mesh_id",
        "set_network_join_requests_enabled",
        "add_participant",
        "add_admin",
        "remove_participant",
        "remove_admin",
        "accept_join_request",
        "set_participant_alias",
        "set_participant_endpoint_hints",
    )

    private val tunnelSettingKeys = setOf(
        "listenPort",
        "endpoint",
        "relays",
        "disabledRelays",
        "exitNode",
        "exitNodeLeakProtection",
        "exitDnsMode",
        "exitDnsDohProvider",
        "exitDnsCustomDohUrl",
        "exitDnsCustomDohBootstrapIps",
        "exitDnsThroughExitServers",
        "advertiseExitNode",
        "advertisedRoutes",
        "wireguardExitEnabled",
        "wireguardExitInterface",
        "wireguardExitAddress",
        "wireguardExitPrivateKey",
        "wireguardExitPeerPublicKey",
        "wireguardExitPeerPresharedKey",
        "wireguardExitEndpoint",
        "wireguardExitAllowedIps",
        "wireguardExitDns",
        "wireguardExitMtu",
        "wireguardExitPersistentKeepaliveSecs",
        "wireguardExitConfig",
    )

    fun requiresTunnelRefresh(type: String, updateSettingKeys: Set<String> = emptySet()): Boolean =
        type in networkActions ||
            (type == "update_settings" && updateSettingKeys.any(tunnelSettingKeys::contains))
}

internal enum class TunnelServiceCommand {
    NONE,
    CONNECT,
    DISCONNECT,
}

internal object TunnelServiceCommandPolicy {
    fun commandAfterAction(
        actionType: String,
        wasEnabled: Boolean,
        isEnabled: Boolean,
        requiresRefresh: Boolean,
    ): TunnelServiceCommand =
        when {
            actionType == "disconnect_vpn" -> TunnelServiceCommand.DISCONNECT
            actionType == "connect_vpn" && isEnabled -> TunnelServiceCommand.CONNECT
            !wasEnabled && isEnabled -> TunnelServiceCommand.CONNECT
            wasEnabled && !isEnabled -> TunnelServiceCommand.DISCONNECT
            wasEnabled && isEnabled && requiresRefresh -> TunnelServiceCommand.CONNECT
            else -> TunnelServiceCommand.NONE
        }
}
