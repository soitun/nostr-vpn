import Foundation

enum ScreenshotFixtures {
    static func state() -> AppState {
        let ownNpub = fakeNpub("q")
        let macNpub = fakeNpub("p")
        let windowsNpub = fakeNpub("z")
        let linuxNpub = fakeNpub("r")
        let androidNpub = fakeNpub("t")
        let ipadNpub = fakeNpub("v")

        var state = AppState()
        state.rev = 1
        state.appVersion = appVersion
        state.platform = "iOS Simulator"
        state.mobile = true
        state.vpnControlSupported = true
        state.runtimeStatusDetail = "Fixture mode"
        state.vpnEnabled = true
        state.vpnActive = true
        state.vpnStatus = "Connected"
        state.daemonRunning = true
        state.ownNpub = ownNpub
        state.nodeName = "iPhone"
        state.selfMagicDnsName = ""
        state.tunnelIp = "10.44.0.2/32"
        state.endpoint = "local network"
        state.listenPort = 51820
        state.activeNetworkInvite = "nvpn://invite/demo-home-mesh"
        state.joinRequestQrCodeOrLink = "nvpn://join-request/demo"
        state.connectedPeerCount = 3
        state.expectedPeerCount = 4
        state.fipsConnectedPeerCount = 3
        state.fipsRosterPeerCount = 5
        state.nonFipsRosterPeerCount = 0
        state.meshReady = true
        state.exitNode = macNpub
        state.exitNodeLeakProtection = true
        state.exitNodeActive = true
        state.exitNodeStatusText = "Routing through laptop"
        state.advertiseExitNode = false
        state.advertisedRoutes = ["10.44.0.0/24"]
        state.wireguardExitEnabled = false
        state.wireguardExitConfigured = true
        state.wireguardExitInterface = "utun-demo"
        state.wireguardExitAddress = "10.64.12.4/32"
        state.wireguardExitEndpoint = "demo-wireguard.invalid:51820"
        state.wireguardExitAllowedIps = "0.0.0.0/0, ::/0"
        state.wireguardExitDns = "100.64.0.1"
        state.wireguardExitMtu = 1280
        state.wireguardExitPersistentKeepaliveSecs = 25
        state.wireguardExitConfig = """
        [Interface]
        PrivateKey = demo-private-key
        Address = 10.64.12.4/32
        DNS = 100.64.0.1

        [Peer]
        PublicKey = demo-peer-key
        Endpoint = demo-wireguard.invalid:51820
        AllowedIPs = 0.0.0.0/0, ::/0
        """
        state.magicDnsSuffix = "home.mesh"
        state.magicDnsStatus = "Ready"
        state.autoconnect = true
        state.connectToNonRosterFipsPeers = true
        state.fipsNostrDiscoveryEnabled = true
        state.fipsWebrtcEnabled = false
        state.fipsBootstrapEnabled = false
        state.inviteBroadcastActive = false
        state.nearbyDiscoveryActive = true
        state.nearbyDiscoveryRemainingSecs = 112
        state.configPath = "Fixture data"
        state.networks = [
            network(
                ownNpub: ownNpub,
                macNpub: macNpub,
                windowsNpub: windowsNpub,
                linuxNpub: linuxNpub,
                androidNpub: androidNpub,
                ipadNpub: ipadNpub
            )
        ]
        state.lanPeers = [
            lanPeer(name: "iPadOS nearby", network: "Home Mesh", invite: "nvpn://invite/demo-ipad")
        ]
        state.paidExitSeller = paidExitSeller()
        state.paidRouteMarket = paidRouteMarket()
        return state
    }

    private static var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }

    static func dispatch(_ action: [String: Any], state original: AppState) -> AppState {
        var state = original
        state.error = ""
        state.rev += 1
        guard let type = action["type"] as? String else {
            return state
        }

        switch type {
        case "connect_vpn":
            state.vpnEnabled = true
            state.vpnActive = true
            state.vpnStatus = "Connected"
            state.connectedPeerCount = min(state.expectedPeerCount, 3)
            state.fipsConnectedPeerCount = min(state.fipsRosterPeerCount, 3)
        case "disconnect_vpn":
            state.vpnEnabled = false
            state.vpnActive = false
            state.vpnStatus = "Disconnected"
            state.connectedPeerCount = 0
            state.fipsConnectedPeerCount = 0
        case "set_network_enabled":
            if let networkId = action["networkId"] as? String,
               let enabled = action["enabled"] as? Bool,
               let index = state.networks.firstIndex(where: { $0.id == networkId }) {
                state.networks[index].enabled = enabled
            }
        case "update_settings":
            if let patch = action["patch"] as? [String: Any] {
                applySettings(patch, to: &state)
            }
        default:
            break
        }
        return state
    }

    static func qrMatrix() -> QrMatrix {
        let width = 25
        var cells = Array(repeating: false, count: width * width)
        for y in 0..<width {
            for x in 0..<width {
                let finder = (x < 7 && y < 7)
                    || (x >= width - 7 && y < 7)
                    || (x < 7 && y >= width - 7)
                cells[y * width + x] = finder || ((x * 3 + y * 5) % 7 == 0)
            }
        }
        var matrix = QrMatrix()
        matrix.width = width
        matrix.cells = cells
        return matrix
    }

    private static func network(
        ownNpub: String,
        macNpub: String,
        windowsNpub: String,
        linuxNpub: String,
        androidNpub: String,
        ipadNpub: String
    ) -> NetworkState {
        var network = NetworkState()
        network.id = "fixture-home"
        network.name = "Home Mesh"
        network.enabled = true
        network.networkId = "mesh-demo-home"
        network.localIsAdmin = true
        network.joinRequestsEnabled = true
        network.onlineCount = 3
        network.expectedCount = 4
        network.inboundJoinRequests = []
        network.participants = [
            participant(
                npub: ownNpub,
                alias: "iPhone",
                magicDns: "",
                ip: "10.44.0.2/32",
                isAdmin: true,
                reachable: true,
                state: "local",
                meshState: "local",
                status: "This device"
            ),
            participant(
                npub: macNpub,
                alias: "Laptop",
                magicDns: "",
                ip: "10.44.0.3/32",
                isAdmin: true,
                reachable: true,
                offersExitNode: true,
                state: "online",
                transport: "LAN direct",
                status: "Online via LAN"
            ),
            participant(
                npub: windowsNpub,
                alias: "Windows PC",
                magicDns: "",
                ip: "10.44.0.4/32",
                reachable: true,
                state: "online",
                status: "Online through relay path"
            ),
            participant(
                npub: linuxNpub,
                alias: "Linux server",
                magicDns: "",
                ip: "10.44.0.5/32",
                reachable: true,
                offersExitNode: true,
                state: "online",
                status: "Online via mesh"
            ),
            participant(
                npub: androidNpub,
                alias: "Android phone",
                magicDns: "",
                ip: "10.44.0.6/32",
                reachable: false,
                state: "offline",
                status: "Last seen 18 min ago"
            ),
            participant(
                npub: ipadNpub,
                alias: "iPad",
                magicDns: "",
                ip: "10.44.0.7/32",
                reachable: false,
                state: "offline",
                status: "Last seen yesterday"
            ),
        ]
        return network
    }

    private static func participant(
        npub: String,
        alias: String,
        magicDns: String,
        ip: String,
        isAdmin: Bool = false,
        reachable: Bool,
        offersExitNode: Bool = false,
        state: String,
        meshState: String = "",
        transport: String = "",
        status: String
    ) -> ParticipantState {
        var participant = ParticipantState()
        participant.npub = npub
        participant.pubkeyHex = fakeHex(String(npub.dropFirst(5).first ?? "0"))
        participant.alias = alias
        participant.magicDnsAlias = alias.lowercased()
        participant.magicDnsName = magicDns
        participant.tunnelIp = ip
        participant.isAdmin = isAdmin
        participant.reachable = reachable
        participant.offersExitNode = offersExitNode
        participant.fipsEndpointNpub = fakeNpub("s")
        participant.fipsTransportAddr = transport
        participant.fipsTransportType = transport.isEmpty ? "" : "udp"
        participant.fipsSrttMs = reachable ? 18 : 0
        participant.fipsSrttAgeMs = reachable ? 1_100 : 0
        participant.fipsPacketsSent = reachable ? 1482 : 0
        participant.fipsPacketsRecv = reachable ? 1516 : 0
        participant.fipsBytesSent = reachable ? 481_280 : 0
        participant.fipsBytesRecv = reachable ? 722_944 : 0
        participant.state = state
        participant.meshState = meshState
        participant.statusText = status
        participant.lastFipsControlSeenText = reachable ? "seen now" : ""
        participant.lastFipsDataSeenText = reachable ? "seen now" : ""
        participant.lastSeenText = reachable ? "now" : "yesterday"
        return participant
    }

    private static func lanPeer(name: String, network: String, invite: String) -> LanPeerState {
        var peer = LanPeerState()
        peer.npub = fakeNpub("y")
        peer.nodeName = name
        peer.networkName = network
        peer.invite = invite
        peer.lastSeenText = "Nearby now"
        return peer
    }

    private static func paidExitSeller() -> PaidExitSellerState {
        var seller = PaidExitSellerState()
        seller.supported = false
        seller.enabled = false
        seller.statusText = "Selling internet is not supported on iOS"
        seller.upstream = "unsupported"
        seller.privateVpnAccess = "denied"
        seller.internetText = "My internet"
        seller.priceText = paidRoutePriceText(priceMsat: 2_500)
        seller.priceMsat = 2_500
        seller.perUnits = 1_000_000
        seller.perUnitsText = "1 MB"
        seller.acceptedMints = ["https://mint.minibits.cash/Bitcoin"]
        seller.maxChannelCapacitySat = 250
        seller.channelExpirySecs = 86_400
        seller.freeProbeUnits = 1_048_576
        seller.freeProbeText = "1 MB"
        seller.graceUnits = 262_144
        seller.graceText = "256 KB"
        seller.countryCode = "FI"
        seller.region = "Uusimaa"
        seller.asn = 14_593
        seller.networkClass = "satellite"
        seller.ipv4 = true
        seller.ipv6 = false
        seller.channelCreditMsat = 35_000
        seller.channelCreditText = "35 sat"
        seller.channelCreditTitleText = "Pending buyer credit"
        seller.channelCreditHelpText = "Collect to move it into wallet"
        seller.currentConnectionCount = 1
        seller.pastConnectionCount = 3
        seller.totalBillableBytes = 48_000_000
        seller.totalTrafficText = "45.8 MB used"
        seller.totalPaidMsat = 92_000
        seller.totalPaidText = "92 sat paid"
        seller.totalDueMsat = 88_000
        seller.totalDueText = "88 sat due"
        seller.totalUnpaidMsat = 0
        seller.totalUnpaidText = ""
        return seller
    }

    private static func paidRouteMarket() -> PaidRouteMarketState {
        var market = PaidRouteMarketState()
        market.supported = true
        market.statusText = "2 internet sellers found"
        market.storePath = "Fixture wallet"
        market.wallet = paidRouteWallet()
        market.lastPaymentAction = paidRoutePaymentAction()
        market.offers = [
            paidRouteOffer(
                key: "seller-fi:internet-exit",
                sellerNpub: fakeNpub("f"),
                statusText: "FI - satellite - 42 ms - seen 2m ago",
                priceMsat: 2_500,
                countryCode: "FI",
                region: "Uusimaa",
                asn: 14_593,
                networkClass: "satellite",
                ipv6: false,
                latencyMs: 42,
                jitterMs: 7,
                packetLossPpm: 500,
                downBps: 25_000_000,
                upBps: 5_000_000
            ),
            paidRouteOffer(
                key: "seller-de:internet-exit",
                sellerNpub: fakeNpub("d"),
                statusText: "DE - datacenter - 18 ms - seen 5m ago",
                priceMsat: 800,
                countryCode: "DE",
                region: "Hesse",
                asn: 12_345,
                networkClass: "datacenter",
                ipv6: true,
                latencyMs: 18,
                jitterMs: 3,
                packetLossPpm: 100,
                downBps: 90_000_000,
                upBps: 20_000_000
            ),
        ]
        market.channels = [
            paidRouteChannel(
                channelId: "channel-fi-1",
                role: "buyer",
                counterpartyNpub: fakeNpub("f"),
                paidMsat: 2_500
            )
        ]
        market.sessions = [
            paidRouteSession(
                sessionId: "session-fi-1",
                leaseId: "lease-fi-1",
                channelId: "channel-fi-1",
                statusText: "Awaiting payment update"
            )
        ]
        return market
    }

    private static func paidRouteWallet() -> PaidRouteWalletState {
        var wallet = PaidRouteWalletState()
        wallet.defaultMint = "https://mint.minibits.cash/Bitcoin"
        wallet.balanceKnown = true
        wallet.totalBalanceMsat = 123_000
        wallet.totalBalanceText = paidRouteMsatText(123_000)

        var mint = PaidRouteWalletMintState()
        mint.url = wallet.defaultMint
        mint.label = ""
        mint.isDefault = true
        mint.balanceKnown = true
        mint.balanceMsat = 123_000
        mint.balanceText = paidRouteMsatText(123_000)
        mint.lastCheckedUnix = 1_780_650_000
        wallet.mints = [mint]

        var action = PaidRouteWalletActionState()
        action.kind = "topup"
        action.statusText = "Invoice ready"
        action.mintUrl = wallet.defaultMint
        action.amountSat = 1_000
        action.amountText = "1000 sat"
        action.feeText = ""
        action.quoteId = "quote-demo"
        action.paymentRequest = "lnbc1000n1pdemoexamplepaidroutewalletinvoice"
        action.expiresAtUnix = 1_780_653_600
        wallet.lastAction = action
        return wallet
    }

    private static func paidRoutePaymentAction() -> PaidRoutePaymentActionState {
        var action = PaidRoutePaymentActionState()
        action.kind = "balance_update"
        action.statusText = "Signed 3,750 msat balance update"
        action.payloadType = "spilman_balance_update"
        action.sessionId = "session-fi-1"
        action.leaseId = "lease-fi-1"
        action.channelId = "channel-fi-1"
        action.buyerNpub = fakeNpub("q")
        action.sellerNpub = fakeNpub("f")
        action.envelopeJson = "{\"type\":\"balance_update\",\"session_id\":\"session-fi-1\"}"
        action.paidMsat = 3_750
        action.paidText = "\(paidRouteMsatText(3_750)) paid"
        action.deliveredUnits = 2_500_000
        action.deliveredUsageText = "2.4 MB used"
        action.amountDueMsat = 3_750
        action.amountDueText = "\(paidRouteMsatText(3_750)) due"
        action.unpaidMsat = 0
        action.unpaidText = ""
        action.allowRouting = true
        return action
    }

    private static func paidRouteOffer(
        key: String,
        sellerNpub: String,
        statusText: String,
        priceMsat: UInt64,
        countryCode: String,
        region: String,
        asn: UInt32,
        networkClass: String,
        ipv6: Bool,
        latencyMs: UInt32,
        jitterMs: UInt32,
        packetLossPpm: UInt32,
        downBps: UInt64,
        upBps: UInt64
    ) -> PaidRouteOfferState {
        var offer = PaidRouteOfferState()
        offer.key = key
        offer.offerId = "internet-exit"
        offer.sellerNpub = sellerNpub
        offer.statusText = statusText
        offer.priceText = paidRoutePriceText(priceMsat: priceMsat)
        offer.priceMsat = priceMsat
        offer.perUnits = 1_000_000
        offer.perUnitsText = "1 MB"
        offer.acceptedMints = ["https://mint.minibits.cash/Bitcoin"]
        offer.maxChannelCapacitySat = 250
        offer.channelExpirySecs = 900
        offer.freeProbeUnits = 1_048_576
        offer.freeProbeText = "1 MB"
        offer.graceUnits = 262_144
        offer.graceText = "256 KB"
        offer.countryCode = countryCode
        offer.region = region
        offer.asn = asn
        offer.networkClass = networkClass
        offer.ipv4 = true
        offer.ipv6 = ipv6
        offer.hasQuality = true
        offer.qualityText = String(
            format: "%u ms · %u ms jitter · %.2f%% loss",
            latencyMs,
            jitterMs,
            Double(packetLossPpm) / 10_000.0
        )
        offer.bandwidthText = "\(downBps / 1_000_000) Mbps down · \(upBps / 1_000_000) Mbps up"
        offer.latencyMs = latencyMs
        offer.jitterMs = jitterMs
        offer.packetLossPpm = packetLossPpm
        offer.downBps = downBps
        offer.upBps = upBps
        offer.uptimeSecs = 3_600
        offer.firstSeenUnix = 1_780_649_000
        offer.lastSeenUnix = 1_780_650_000
        offer.relayUrls = ["wss://relay.damus.io"]
        return offer
    }

    private static func paidRoutePriceText(priceMsat: UInt64) -> String {
        guard priceMsat > 0 else { return "free" }
        let perGBMsat = priceMsat * 1_000
        let bytesPerSat = 1_000_000_000 / priceMsat
        let purchasingPower: String
        if bytesPerSat >= 1_000_000 {
            var megabytes = String(format: "%.2f", Double(bytesPerSat) / 1_000_000)
            while megabytes.last == "0" { megabytes.removeLast() }
            if megabytes.last == "." { megabytes.removeLast() }
            purchasingPower = "\(megabytes) MB"
        } else {
            purchasingPower = "\(bytesPerSat / 1_000) KB"
        }
        return "\(paidRouteMsatText(perGBMsat)) / GB · 1 sat ≈ \(purchasingPower)"
    }

    private static func paidRouteMsatText(_ msat: UInt64) -> String {
        let whole = msat / 1_000
        let remainder = msat % 1_000
        return remainder == 0
            ? "\(whole) sat"
            : "\(whole).\(String(format: "%03d", Int(remainder))) sat"
    }

    private static func paidRouteChannel(
        channelId: String,
        role: String,
        counterpartyNpub: String,
        paidMsat: UInt64
    ) -> PaidRouteChannelState {
        var channel = PaidRouteChannelState()
        channel.channelId = channelId
        channel.offerId = "internet-exit"
        channel.role = role
        channel.status = "active"
        channel.mintUrl = "https://mint.minibits.cash/Bitcoin"
        channel.counterpartyNpub = counterpartyNpub
        channel.capacitySat = 250
        channel.capacityText = "250 sat"
        channel.paidMsat = paidMsat
        channel.paidText = "\(paidRouteMsatText(paidMsat)) paid"
        channel.updatedAtUnix = 1_780_650_000
        channel.expiresAtUnix = 1_780_650_900
        return channel
    }

    private static func paidRouteSession(
        sessionId: String,
        leaseId: String,
        channelId: String,
        statusText: String
    ) -> PaidRouteSessionState {
        var session = PaidRouteSessionState()
        session.sessionId = sessionId
        session.leaseId = leaseId
        session.channelId = channelId
        session.statusText = statusText
        session.lifecycleStatus = "active"
        session.accessState = "grace"
        session.titleText = "Ready"
        session.detailText = "Grace, 2.4 MB used, 3.750 sat due"
        session.paymentChannelReady = true
        session.allowRouting = true
        session.deliveredUnits = 2_500_000
        session.usageText = "2.4 MB used"
        session.amountDueMsat = 3_750
        session.amountDueText = "\(paidRouteMsatText(3_750)) due"
        session.paidMsat = 2_500
        session.paidText = "\(paidRouteMsatText(2_500)) paid"
        session.unpaidMsat = 1_250
        session.unpaidText = "\(paidRouteMsatText(1_250)) behind"
        session.activeMillis = 31_000
        session.bytes = 2_500_000
        session.packets = 1_920
        session.realizedExitIp = "198.51.100.42"
        session.claimedCountryCode = "FI"
        session.observedCountryCode = "FI"
        session.countryClaimStatus = "match"
        session.locationText = "198.51.100.42 - FI matches claim"
        session.observedAsn = 14_593
        session.hasQuality = true
        session.qualityText = "42 ms · 7 ms jitter · 0.05% loss"
        session.bandwidthText = "25 Mbps down · 5 Mbps up"
        session.latencyMs = 42
        session.jitterMs = 7
        session.packetLossPpm = 500
        session.downBps = 25_000_000
        session.upBps = 5_000_000
        session.updatedAtUnix = 1_780_650_000
        session.expiresAtUnix = 1_780_650_900
        session.settlementText = "Channel ends in 15 min"
        return session
    }

    private static func applySettings(_ patch: [String: Any], to state: inout AppState) {
        if let exitNode = patch["exitNode"] as? String {
            state.exitNode = exitNode
            state.exitNodeActive = !exitNode.isEmpty
        }
        if let enabled = patch["wireguardExitEnabled"] as? Bool {
            state.wireguardExitEnabled = enabled
            if enabled {
                state.exitNode = ""
                state.exitNodeActive = false
            }
        }
        if let leakProtection = patch["exitNodeLeakProtection"] as? Bool {
            state.exitNodeLeakProtection = leakProtection
        }
        if let advertise = patch["advertiseExitNode"] as? Bool {
            state.advertiseExitNode = advertise
        }
        if let autoconnect = patch["autoconnect"] as? Bool {
            state.autoconnect = autoconnect
        }
        if let connectToNonRosterFipsPeers = patch["connectToNonRosterFipsPeers"] as? Bool {
            state.connectToNonRosterFipsPeers = connectToNonRosterFipsPeers
        }
        if let fipsNostrDiscoveryEnabled = patch["fipsNostrDiscoveryEnabled"] as? Bool {
            state.fipsNostrDiscoveryEnabled = fipsNostrDiscoveryEnabled
        }
        if let fipsWebrtcEnabled = patch["fipsWebrtcEnabled"] as? Bool {
            state.fipsWebrtcEnabled = fipsWebrtcEnabled
        }
        if let fipsBootstrapEnabled = patch["fipsBootstrapEnabled"] as? Bool {
            state.fipsBootstrapEnabled = fipsBootstrapEnabled
        }
        if let nodeName = patch["nodeName"] as? String {
            state.nodeName = nodeName
        }
        if let tunnelIp = patch["tunnelIp"] as? String {
            state.tunnelIp = tunnelIp
        }
        if let endpoint = patch["endpoint"] as? String {
            state.endpoint = endpoint
        }
        if let listenPort = patch["listenPort"] as? Int {
            state.listenPort = listenPort
        }
        if let config = patch["wireguardExitConfig"] as? String {
            state.wireguardExitConfig = config
            state.wireguardExitConfigured = !config.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    private static func fakeNpub(_ character: Character) -> String {
        "npub1" + String(repeating: String(character), count: 58)
    }

    private static func fakeHex(_ character: String) -> String {
        let scalar = character.unicodeScalars.first?.value ?? 0
        let byte = String(format: "%02x", scalar % 256)
        return String(repeating: byte, count: 32)
    }
}
