import Foundation

final class NativeCoreClient {
    private var handle: OpaquePointer?
    private let dataDir: String

    init(dataDir: String, appVersion: String) {
        self.dataDir = dataDir
        handle = dataDir.withCString { dataDirPointer in
            appVersion.withCString { versionPointer in
                nostr_vpn_app_new(dataDirPointer, versionPointer)
            }
        }
    }

    deinit {
        close()
    }

    func close() {
        guard let handle else {
            return
        }
        nostr_vpn_app_free(handle)
        self.handle = nil
    }

    func state() -> AppState {
        parseState(consume(nostr_vpn_app_state_json(requireHandle())))
    }

    func refresh() -> AppState {
        parseState(consume(nostr_vpn_app_refresh_json(requireHandle())))
    }

    func dispatch(_ action: [String: Any]) -> AppState {
        guard JSONSerialization.isValidJSONObject(action),
              let data = try? JSONSerialization.data(withJSONObject: action),
              let json = String(data: data, encoding: .utf8)
        else {
            var state = state()
            state.error = "Invalid native action JSON"
            return state
        }

        return parseState(
            json.withCString { actionPointer in
                consume(nostr_vpn_app_dispatch_json(requireHandle(), actionPointer))
            }
        )
    }

    func qrMatrix(invite: String) -> QrMatrix {
        let json = invite.withCString { textPointer in
            consume(nostr_vpn_qr_matrix_json(textPointer))
        }
        guard let data = json.data(using: .utf8),
              let matrix = try? JSONDecoder().decode(QrMatrix.self, from: data)
        else {
            return QrMatrix()
        }
        return matrix
    }

    func decodeQrImage(path: String) -> QrDecodeResult {
        let json = path.withCString { pathPointer in
            consume(nostr_vpn_decode_qr_image_json(pathPointer))
        }
        guard let data = json.data(using: .utf8),
              let result = try? JSONDecoder().decode(QrDecodeResult.self, from: data)
        else {
            return QrDecodeResult(error: "Invalid QR decode response")
        }
        return result
    }

    func mobileTunnelConfigJson() -> String {
        dataDir.withCString { dataDirPointer in
            consume(nostr_vpn_mobile_tunnel_config_json(dataDirPointer))
        }
    }

    func mobileTunnelProviderOptionsConfigJson() -> String {
        dataDir.withCString { dataDirPointer in
            consume(nostr_vpn_mobile_tunnel_provider_options_config_json(dataDirPointer))
        }
    }

    private func parseState(_ json: String) -> AppState {
        guard let data = json.data(using: .utf8),
              let state = try? JSONDecoder().decode(AppState.self, from: data)
        else {
            var state = AppState()
            state.error = "Invalid native app state"
            return state
        }
        return state
    }

    private func requireHandle() -> OpaquePointer? {
        handle
    }

    private func consume(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
        guard let pointer else {
            return ""
        }
        defer { nostr_vpn_string_free(pointer) }
        return String(cString: pointer)
    }
}

enum NativeActions {
    static func connectVpn() -> [String: Any] {
        ["type": "connect_vpn"]
    }

    static func disconnectVpn() -> [String: Any] {
        ["type": "disconnect_vpn"]
    }

    static func importInvite(_ invite: String) -> [String: Any] {
        ["type": "import_network_invite", "invite": invite]
    }

    static func importJoinRequest(_ request: String) -> [String: Any] {
        ["type": "import_join_request", "request": request]
    }

    static func linkNetwork(_ link: String) -> [String: Any] {
        importInvite(link)
    }

    static func resetNetworkInvite(networkId: String) -> [String: Any] {
        ["type": "reset_network_invite", "networkId": networkId]
    }

    static func startInviteBroadcast() -> [String: Any] {
        ["type": "start_invite_broadcast"]
    }

    static func stopInviteBroadcast() -> [String: Any] {
        ["type": "stop_invite_broadcast"]
    }

    static func startNearbyDiscovery() -> [String: Any] {
        ["type": "start_nearby_discovery"]
    }

    static func stopNearbyDiscovery() -> [String: Any] {
        ["type": "stop_nearby_discovery"]
    }

    static func addNetwork(_ name: String) -> [String: Any] {
        ["type": "add_network", "name": name]
    }

    static func manualAddNetwork(adminNpub: String, meshNetworkId: String) -> [String: Any] {
        [
            "type": "manual_add_network",
            "adminNpub": adminNpub,
            "meshNetworkId": meshNetworkId,
        ]
    }

    static func setNetworkEnabled(_ networkId: String, _ enabled: Bool) -> [String: Any] {
        ["type": "set_network_enabled", "networkId": networkId, "enabled": enabled]
    }

    static func removeNetwork(_ networkId: String) -> [String: Any] {
        ["type": "remove_network", "networkId": networkId]
    }

    static func requestNetworkJoin(networkId: String) -> [String: Any] {
        ["type": "request_network_join", "networkId": networkId]
    }

    static func requestDeviceApproval(networkId: String) -> [String: Any] {
        ["type": "request_device_approval", "networkId": networkId]
    }

    static func updateSettings(_ patch: [String: Any]) -> [String: Any] {
        ["type": "update_settings", "patch": patch]
    }

    static func addParticipant(networkId: String, npub: String, alias: String) -> [String: Any] {
        [
            "type": "add_participant",
            "networkId": networkId,
            "npub": npub,
            "alias": alias.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? NSNull() : alias,
        ]
    }

    static func setParticipantAlias(npub: String, alias: String) -> [String: Any] {
        ["type": "set_participant_alias", "npub": npub, "alias": alias]
    }

    static func setParticipantEndpointHints(npub: String, endpointHints: [String]) -> [String: Any] {
        ["type": "set_participant_endpoint_hints", "npub": npub, "endpointHints": endpointHints]
    }

    static func addPaidRouteWalletMint(url: String, label: String?) -> [String: Any] {
        ["type": "add_paid_route_wallet_mint", "url": url, "label": jsonValue(label)]
    }

    static func removePaidRouteWalletMint(url: String) -> [String: Any] {
        ["type": "remove_paid_route_wallet_mint", "url": url]
    }

    static func setPaidRouteDefaultMint(url: String) -> [String: Any] {
        ["type": "set_paid_route_default_mint", "url": url]
    }

    static func refreshPaidRouteWallet(refresh: Bool = true) -> [String: Any] {
        ["type": "refresh_paid_route_wallet", "refresh": refresh]
    }

    static func topUpPaidRouteWallet(mintUrl: String?, amountSat: UInt64) -> [String: Any] {
        ["type": "top_up_paid_route_wallet", "mintUrl": jsonValue(mintUrl), "amountSat": amountSat]
    }

    static func receivePaidRouteWalletToken(token: String) -> [String: Any] {
        ["type": "receive_paid_route_wallet_token", "token": token]
    }

    static func sendPaidRouteWalletToken(mintUrl: String?, amountSat: UInt64) -> [String: Any] {
        ["type": "send_paid_route_wallet_token", "mintUrl": jsonValue(mintUrl), "amountSat": amountSat]
    }

    static func withdrawPaidRouteWalletLightning(mintUrl: String?, invoice: String) -> [String: Any] {
        ["type": "withdraw_paid_route_wallet_lightning", "mintUrl": jsonValue(mintUrl), "invoice": invoice]
    }

    static func buyPaidRouteOffer(offerKey: String, mintUrl: String? = nil, channelCapacitySat: UInt64? = nil) -> [String: Any] {
        [
            "type": "buy_paid_route_offer",
            "offerKey": offerKey,
            "mintUrl": jsonValue(mintUrl),
            "channelCapacitySat": jsonValue(channelCapacitySat),
        ]
    }

    static func selectPaidRouteSession(sessionId: String, connect: Bool) -> [String: Any] {
        ["type": "select_paid_route_session", "sessionId": sessionId, "connect": connect]
    }

    static func probePaidRouteSession(sessionId: String, timeoutSecs: UInt64 = 5) -> [String: Any] {
        ["type": "probe_paid_route_session", "sessionId": sessionId, "timeoutSecs": timeoutSecs]
    }

    static func openPaidRouteChannelFromWallet(
        sessionId: String,
        mintUrl: String? = nil,
        paidMsat: UInt64? = nil,
        maxAmountPerOutput: UInt64? = nil,
        keysetId: String? = nil
    ) -> [String: Any] {
        [
            "type": "open_paid_route_channel_from_wallet",
            "sessionId": sessionId,
            "mintUrl": jsonValue(mintUrl),
            "paidMsat": jsonValue(paidMsat),
            "maxAmountPerOutput": jsonValue(maxAmountPerOutput),
            "keysetId": jsonValue(keysetId),
        ]
    }

    static func signPaidRoutePaymentEnvelopeFromWallet(
        sessionId: String,
        kind: String = "balance-update",
        deliveredUnits: UInt64? = nil,
        paidMsat: UInt64? = nil
    ) -> [String: Any] {
        [
            "type": "sign_paid_route_payment_envelope_from_wallet",
            "sessionId": sessionId,
            "kind": kind,
            "deliveredUnits": jsonValue(deliveredUnits),
            "paidMsat": jsonValue(paidMsat),
        ]
    }

    static func closePaidRouteChannelFromWallet(sessionId: String, publish: Bool = true) -> [String: Any] {
        ["type": "close_paid_route_channel_from_wallet", "sessionId": sessionId, "publish": publish]
    }

    static func sendPaidRoutePaymentEnvelope(envelopeJson: String) -> [String: Any] {
        ["type": "send_paid_route_payment_envelope", "envelopeJson": envelopeJson]
    }

    static func streamPaidRoutePayments(publish: Bool = true, minIncrementMsat: UInt64 = 1, limit: UInt64 = 0) -> [String: Any] {
        [
            "type": "stream_paid_route_payments",
            "publish": publish,
            "minIncrementMsat": minIncrementMsat,
            "limit": limit,
        ]
    }

    static func receivePaidRoutePayments(durationSecs: UInt64 = 5) -> [String: Any] {
        ["type": "receive_paid_route_payments", "durationSecs": durationSecs]
    }

    static func collectDuePaidExitChannels() -> [String: Any] {
        ["type": "collect_due_paid_exit_channels"]
    }

    static func publishPaidExitOffer() -> [String: Any] {
        ["type": "publish_paid_exit_offer"]
    }

    static func setPaidRouteMarketFilter(
        query: String = "",
        countryCode: String = "",
        networkClass: String = "",
        mintUrl: String = "",
        requireIpv4: Bool = false,
        requireIpv6: Bool = false,
        sort: String = "quality"
    ) -> [String: Any] {
        [
            "type": "set_paid_route_market_filter",
            "query": query,
            "countryCode": countryCode,
            "networkClass": networkClass,
            "mintUrl": mintUrl,
            "requireIpv4": requireIpv4,
            "requireIpv6": requireIpv6,
            "sort": sort,
        ]
    }

    static func discoverPaidRouteOffers(durationSecs: UInt64 = 5) -> [String: Any] {
        ["type": "discover_paid_route_offers", "durationSecs": durationSecs]
    }

    private static func jsonValue(_ value: String?) -> Any {
        guard let value else { return NSNull() }
        return value
    }

    private static func jsonValue(_ value: UInt64?) -> Any {
        guard let value else { return NSNull() }
        return value
    }

    static func addAdmin(networkId: String, npub: String) -> [String: Any] {
        ["type": "add_admin", "networkId": networkId, "npub": npub]
    }

    static func removeAdmin(networkId: String, npub: String) -> [String: Any] {
        ["type": "remove_admin", "networkId": networkId, "npub": npub]
    }

    static func removeParticipant(networkId: String, npub: String) -> [String: Any] {
        ["type": "remove_participant", "networkId": networkId, "npub": npub]
    }

    static func acceptJoinRequest(networkId: String, requesterNpub: String) -> [String: Any] {
        [
            "type": "accept_join_request",
            "networkId": networkId,
            "requesterNpub": requesterNpub,
        ]
    }

    static func approveDeviceLink(networkId: String, requesterNpub: String) -> [String: Any] {
        [
            "type": "approve_device_link",
            "networkId": networkId,
            "requesterNpub": requesterNpub,
        ]
    }

    static func rejectDeviceLink(networkId: String, requesterNpub: String) -> [String: Any] {
        [
            "type": "reject_device_link",
            "networkId": networkId,
            "requesterNpub": requesterNpub,
        ]
    }

    static func rejectJoinRequest(networkId: String, requesterNpub: String) -> [String: Any] {
        [
            "type": "reject_join_request",
            "networkId": networkId,
            "requesterNpub": requesterNpub,
        ]
    }

    static func setJoinRequests(networkId: String, enabled: Bool) -> [String: Any] {
        [
            "type": "set_network_join_requests_enabled",
            "networkId": networkId,
            "enabled": enabled,
        ]
    }
}
