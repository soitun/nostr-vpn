import Foundation
import Darwin
import SwiftUI
import UIKit

@MainActor
final class AppModel: ObservableObject {
    nonisolated static let appGroupIdentifier = Bundle.main.object(
        forInfoDictionaryKey: "NVPNAppGroupIdentifier"
    ) as? String ?? "group.fi.siriusbusiness.nvpn"
    private static let configFileName = "config.toml"
    private static let mobileRuntimeStateFileName = "mobile-runtime-state.json"
    static let vpnDisclosureAcceptedKey = "vpnDisclosureAccepted"
    static let vpnDisclosurePromptMessage = "Review VPN data use before turning VPN on."

    @Published var state: AppState
    @Published var actionInFlight = false
    @Published var statusMessage = ""
    @Published var copiedValue = ""
    @Published var vpnDisclosurePromptVisible = false

    private let core: NativeCoreClient?
    private let vpnController = PacketTunnelController()
    private let supportDir: URL?
    private let fixtureMode: Bool
    private var refreshTask: Task<Void, Never>?
    private var copyClearTask: Task<Void, Never>?
    private var tunnelConfigSyncTask: Task<Void, Never>?
    private var launchAutomationHandled = false
    private var tunnelStateRefreshInFlight = false

    init() {
        fixtureMode = Self.fixtureModeRequested()
        if fixtureMode {
            supportDir = nil
            core = nil
            state = ScreenshotFixtures.state()
            return
        }

        supportDir = Self.supportDirectory()
        if let supportDir {
            try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
            Self.seedMobileConfig(in: supportDir, deviceName: Self.deviceName())
        }
        // Pass empty so the FFI falls back to its own CARGO_PKG_VERSION
        // (workspace-inherited). Avoids drift between MARKETING_VERSION in the
        // xcodeproj and the bundled nvpn binary.
        let client = NativeCoreClient(dataDir: supportDir?.path ?? "", appVersion: "")
        core = client
        state = client.state()
        debugLog("init args=\(Self.redactedDebugArguments(ProcessInfo.processInfo.arguments))")
    }

    deinit {
        refreshTask?.cancel()
        tunnelConfigSyncTask?.cancel()
        core?.close()
    }

    var activeNetwork: NetworkState? {
        state.activeNetwork
    }

    func start() {
        guard !fixtureMode else {
            return
        }
        guard refreshTask == nil else {
            return
        }
        refreshTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                self?.refresh()
            }
        }
        let launchAutomationHandled = runLaunchAutomationIfRequested()
        if !launchAutomationHandled, state.autoconnect, !state.vpnEnabled, activeNetwork != nil {
            debugLog("autoconnect starting PacketTunnel")
            if UserDefaults.standard.bool(forKey: Self.vpnDisclosureAcceptedKey) {
                setVpnEnabled(true)
            } else {
                requireVpnDisclosureReview()
            }
        }
    }

    func refresh() {
        guard let core else {
            state.rev += 1
            return
        }
        state = core.refresh()
        refreshTunnelSidecarState()
    }

    func dispatch(_ action: [String: Any], status: String = "") {
        guard !actionInFlight else {
            return
        }
        let actionType = action["type"] as? String ?? ""
        actionInFlight = true
        statusMessage = status
        if fixtureMode {
            state = ScreenshotFixtures.dispatch(action, state: state)
        } else if let core {
            state = core.dispatch(action)
        }
        actionInFlight = false
        statusMessage = state.error
        debugLog(
            "dispatch action=\(actionType) error=\(!state.error.isEmpty) vpn=\(state.vpnEnabled)/\(state.vpnActive) network=\(activeNetwork?.id ?? "nil")"
        )
        if state.error.isEmpty && actionRequiresPacketTunnelConfigSync(actionType) {
            schedulePacketTunnelConfigSync(reason: actionType)
        }
    }

    func toggleVpn() {
        setVpnEnabled(!state.vpnEnabled)
    }

    func requireVpnDisclosureReview() {
        vpnDisclosurePromptVisible = true
        statusMessage = Self.vpnDisclosurePromptMessage
    }

    func markVpnDisclosureAccepted() {
        UserDefaults.standard.set(true, forKey: Self.vpnDisclosureAcceptedKey)
        vpnDisclosurePromptVisible = false
        if statusMessage == Self.vpnDisclosurePromptMessage {
            statusMessage = ""
        }
    }

    private func setVpnEnabled(_ enabled: Bool, force: Bool = false) {
        debugLog("setVpnEnabled enabled=\(enabled) force=\(force) stateEnabled=\(state.vpnEnabled)")
        if fixtureMode {
            state.vpnEnabled = enabled
            state.vpnActive = enabled
            state.vpnStatus = enabled ? "Connected" : "Disconnected"
            state.connectedPeerCount = enabled ? min(state.expectedPeerCount, 3) : 0
            state.fipsConnectedPeerCount = enabled ? min(state.fipsRosterPeerCount, 3) : 0
            state.rev += 1
            statusMessage = ""
            return
        }
        guard let core else {
            statusMessage = "Native core unavailable"
            return
        }
        Task {
            if enabled {
                guard force || !state.vpnEnabled else {
                    debugLog("connect skipped: already enabled")
                    return
                }
                let tunnelConfigJson = core.mobileTunnelConfigJson()
                let providerOptionsConfigJson = core.mobileTunnelProviderOptionsConfigJson()
                debugLog("mobileTunnelConfigJson len=\(tunnelConfigJson.count)")
                if state.vpnEnabled {
                    statusMessage = "Turning VPN on"
                } else {
                    dispatch(NativeActions.connectVpn(), status: "Turning VPN on")
                }
                debugLog("starting PacketTunnel stateEnabled=\(state.vpnEnabled) network=\(activeNetwork?.id ?? "nil")")
                do {
                    try await vpnController.start(
                        state: state,
                        network: activeNetwork,
                        tunnelConfigJson: tunnelConfigJson,
                        providerOptionsConfigJson: providerOptionsConfigJson
                    )
                    debugLog("PacketTunnel start returned success")
                } catch {
                    dispatch(NativeActions.disconnectVpn(), status: "Turning VPN off")
                    statusMessage = error.localizedDescription
                    debugLog("PacketTunnel start failed: \(String(describing: error))")
                }
            } else {
                guard force || state.vpnEnabled else {
                    debugLog("disconnect skipped: already disabled")
                    return
                }
                if state.vpnEnabled {
                    dispatch(NativeActions.disconnectVpn(), status: "Turning VPN off")
                }
                do {
                    try await vpnController.stop()
                    debugLog("PacketTunnel stop returned success")
                } catch {
                    statusMessage = error.localizedDescription
                    debugLog("PacketTunnel stop failed: \(String(describing: error))")
                }
            }
        }
    }

    private func schedulePacketTunnelConfigSync(reason: String) {
        guard !fixtureMode else {
            return
        }
        guard state.vpnEnabled || state.vpnActive else {
            debugLog("PacketTunnel config sync skipped reason=\(reason) vpn off")
            return
        }
        tunnelConfigSyncTask?.cancel()
        tunnelConfigSyncTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 250_000_000)
            await self?.syncPacketTunnelConfig(reason: reason)
        }
    }

    private func syncPacketTunnelConfig(reason: String) async {
        guard let core else {
            statusMessage = "Native core unavailable"
            return
        }
        guard state.vpnEnabled || state.vpnActive else {
            debugLog("PacketTunnel config sync aborted reason=\(reason) vpn off")
            return
        }
        let tunnelConfigJson = core.mobileTunnelConfigJson()
        let providerOptionsConfigJson = core.mobileTunnelProviderOptionsConfigJson()
        debugLog(
            "PacketTunnel config sync begin reason=\(reason) configLen=\(tunnelConfigJson.count) network=\(activeNetwork?.id ?? "nil")"
        )
        statusMessage = "Updating VPN"
        do {
            try await vpnController.stop()
            debugLog("PacketTunnel config sync stop returned")
        } catch {
            debugLog("PacketTunnel config sync stop failed: \(String(describing: error))")
        }
        try? await Task.sleep(nanoseconds: 500_000_000)
        do {
            try await vpnController.start(
                state: state,
                network: activeNetwork,
                tunnelConfigJson: tunnelConfigJson,
                providerOptionsConfigJson: providerOptionsConfigJson
            )
            debugLog("PacketTunnel config sync start returned")
            refresh()
            statusMessage = state.error
        } catch {
            dispatch(NativeActions.disconnectVpn(), status: "Turning VPN off")
            statusMessage = error.localizedDescription
            debugLog("PacketTunnel config sync start failed: \(String(describing: error))")
        }
    }

    private func actionRequiresPacketTunnelConfigSync(_ type: String) -> Bool {
        switch type {
        case "import_network_invite",
             "import_join_request",
             "request_network_join",
             "manual_add_network",
             "set_network_enabled",
             "set_network_mesh_id",
             "set_network_join_requests_enabled",
             "add_participant",
             "set_participant_endpoint_hints",
             "add_admin",
             "remove_participant",
             "remove_admin",
             "accept_join_request",
             "reject_join_request":
            return true
        default:
            return false
        }
    }

    func importInvite(_ invite: String) {
        let trimmed = invite.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return
        }
        debugLog("importInvite len=\(trimmed.count)")
        dispatch(NativeActions.linkNetwork(trimmed), status: "Linking network")
    }

    func linkNetwork(_ link: String) {
        importInvite(link)
    }

    func handle(url: URL) {
        debugLog("handle url=\(url.absoluteString)")
        let raw = url.absoluteString
        if raw.lowercased().hasPrefix("nvpn://invite/") {
            importInvite(raw)
            return
        }

        guard url.scheme == "nvpn", url.host == "debug" else {
            return
        }

        let action = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if action == "tick" {
            refresh()
        } else if action == "connect" {
            setVpnEnabled(true, force: true)
        } else if action == "disconnect" {
            setVpnEnabled(false, force: true)
        }
    }

    private func runLaunchAutomationIfRequested() -> Bool {
        guard !launchAutomationHandled else {
            return false
        }
        launchAutomationHandled = true

        let rawArguments = ProcessInfo.processInfo.arguments
        let arguments = Set(rawArguments)
        debugLog("launch automation args=\(Self.redactedDebugArguments(rawArguments))")
        let importedInvite = importDebugInviteIfPresent(arguments: rawArguments)
        let addedNetwork = addDebugNetworkIfPresent(arguments: rawArguments)
        if arguments.contains("--nvpn-debug-exit-probe") {
            Task {
                await runDebugExitProbe(arguments: rawArguments)
            }
            return true
        }
        if arguments.contains("--nvpn-connect") {
            setVpnEnabled(true, force: true)
            return true
        }
        if arguments.contains("--nvpn-disconnect") {
            setVpnEnabled(false, force: true)
            return true
        }
        return importedInvite || addedNetwork
    }

    private func importDebugInviteIfPresent(arguments: [String]) -> Bool {
        #if DEBUG
        guard let invite = Self.argumentValue(after: "--nvpn-debug-import-invite", in: arguments),
              !invite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            return false
        }
        importInvite(invite)
        refresh()
        return true
        #else
        return false
        #endif
    }

    private func addDebugNetworkIfPresent(arguments: [String]) -> Bool {
        #if DEBUG
        guard let name = Self.argumentValue(after: "--nvpn-debug-add-network", in: arguments) else {
            return false
        }
        let normalized = name.trimmingCharacters(in: .whitespacesAndNewlines)
        dispatch(NativeActions.addNetwork(normalized.isEmpty ? "iOS smoke" : normalized))
        refresh()
        return true
        #else
        return false
        #endif
    }

    private func runDebugExitProbe(arguments: [String]) async {
        #if DEBUG
        let urlString = Self.argumentValue(after: "--nvpn-debug-fetch-url", in: arguments)
            ?? "https://am.i.mullvad.net/json"
        let resultName = Self.argumentValue(after: "--nvpn-debug-result", in: arguments)
            ?? "debug-exit-probe.json"
        let waitSeconds = Self.argumentValue(after: "--nvpn-debug-wait-seconds", in: arguments)
            .flatMap(Double.init) ?? 12
        let exitNode = Self.argumentValue(after: "--nvpn-debug-exit-node", in: arguments)
        let clearExit = arguments.contains("--nvpn-debug-clear-exit")
        let wireGuardConfig = Self.wireGuardConfig(from: arguments, supportDir: supportDir)
        let resolveHost = Self.argumentValue(after: "--nvpn-debug-resolve-host", in: arguments)
        let skipFetch = arguments.contains("--nvpn-debug-skip-fetch")
        var result: [String: Any] = [
            "url": urlString,
            "phase": "starting",
            "startedAt": ISO8601DateFormatter().string(from: Date()),
        ]

        await stopVpnForDebugProbe()

        if let wireGuardConfig, !wireGuardConfig.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            dispatch(NativeActions.updateSettings([
                "wireguardExitConfig": wireGuardConfig,
                "wireguardExitEnabled": true,
                "exitNode": "",
            ]))
        } else if let exitNode, !exitNode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            dispatch(NativeActions.updateSettings([
                "exitNode": exitNode,
                "wireguardExitEnabled": false,
                "exitNodeLeakProtection": true,
            ]))
        } else if clearExit {
            dispatch(NativeActions.updateSettings([
                "exitNode": "",
                "wireguardExitEnabled": false,
                "exitNodeLeakProtection": false,
            ]))
        }
        refresh()

        writeDebugProbeResult(result, name: resultName)
        let startError = await startVpnForDebugProbe()
        if let error = startError {
            result["startError"] = error
        } else if waitSeconds > 0 {
            try? await Task.sleep(nanoseconds: UInt64(waitSeconds * 1_000_000_000))
        }
        refresh()
        result["phase"] = "finished"
        if let status = await vpnController.statusRawValue() {
            result["packetTunnelStatusRawValue"] = status
            if status == 3 {
                for (key, value) in await runDebugTunPacketProbe() {
                    result[key] = value
                }
            }
        }
        if let runtimeJson = await vpnController.runtimeStateJson() {
            result["packetTunnelRuntimeStateJson"] = String(runtimeJson.prefix(4096))
        }
        result["exitNode"] = state.exitNode
        result["vpnEnabled"] = state.vpnEnabled
        result["vpnActive"] = state.vpnActive
        result["connectedPeerCount"] = state.connectedPeerCount
        result["expectedPeerCount"] = state.expectedPeerCount
        result["fipsConnectedPeerCount"] = state.fipsConnectedPeerCount
        result["fipsRosterPeerCount"] = state.fipsRosterPeerCount
        result["nonFipsRosterPeerCount"] = state.nonFipsRosterPeerCount
        result["exitNodeLeakProtection"] = state.exitNodeLeakProtection
        result["wireguardExitEnabled"] = state.wireguardExitEnabled
        result["wireguardExitConfigured"] = state.wireguardExitConfigured
        result["wireguardExitEndpoint"] = state.wireguardExitEndpoint

        if let host = resolveHost?.trimmingCharacters(in: .whitespacesAndNewlines),
           !host.isEmpty {
            let resolved = Self.resolveDebugHost(host)
            result["resolvedHost"] = host
            result["resolvedAddresses"] = resolved.addresses
            if let error = resolved.error {
                result["resolveError"] = error
            }
        }

        if !skipFetch {
            for (key, value) in await fetchDebugProbe(urlString: urlString) {
                result[key] = value
            }
        }
        result["finishedAt"] = ISO8601DateFormatter().string(from: Date())
        writeDebugProbeResult(result, name: resultName)
        #endif
    }

    private func stopVpnForDebugProbe() async {
        refresh()
        guard state.vpnEnabled else {
            return
        }
        dispatch(NativeActions.disconnectVpn())
        do {
            try await vpnController.stop()
        } catch {
            debugLog("debug probe stop failed: \(String(describing: error))")
        }
        for _ in 0..<20 {
            if let status = await vpnController.statusRawValue(), status <= 1 {
                break
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
        }
        refresh()
    }

    private func startVpnForDebugProbe() async -> String? {
        guard let core else {
            return "Native core unavailable"
        }
        let tunnelConfigJson = core.mobileTunnelConfigJson()
        let providerOptionsConfigJson = core.mobileTunnelProviderOptionsConfigJson()
        if !state.vpnEnabled {
            dispatch(NativeActions.connectVpn())
        }
        do {
            try await vpnController.start(
                state: state,
                network: activeNetwork,
                tunnelConfigJson: tunnelConfigJson,
                providerOptionsConfigJson: providerOptionsConfigJson
            )
            return nil
        } catch {
            dispatch(NativeActions.disconnectVpn())
            let message = error.localizedDescription
            debugLog("debug probe start failed: \(message)")
            return message
        }
    }

    private func fetchDebugProbe(urlString: String) async -> [String: Any] {
        var result: [String: Any] = [:]
        guard let url = URL(string: urlString) else {
            result["fetchError"] = "Invalid URL"
            return result
        }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 20
        configuration.timeoutIntervalForResource = 25
        let session = URLSession(configuration: configuration)
        do {
            let (data, response) = try await session.data(from: url)
            if let http = response as? HTTPURLResponse {
                result["statusCode"] = http.statusCode
            }
            if let body = String(data: data, encoding: .utf8) {
                result["body"] = String(body.prefix(4096))
            } else {
                result["byteCount"] = data.count
            }
        } catch {
            result["fetchError"] = String(describing: error)
        }
        return result
    }

    private func runDebugTunPacketProbe() async -> [String: Any] {
        let target = "10.44.255.254"
        let port: UInt16 = 9
        let packetCount = 4
        let waitSeconds = 6.0
        let pollIntervalNanoseconds: UInt64 = 100_000_000
        var result: [String: Any] = [
            "tunPacketProbeTarget": target,
            "tunPacketProbePort": Int(port),
            "tunPacketProbeExpectedPackets": packetCount,
            "tunPacketProbePollIntervalMs": Int(pollIntervalNanoseconds / 1_000_000),
            "tunPacketProbeReadIncreased": false,
        ]

        let baselineRuntimeJson = await vpnController.runtimeStateJson()
        guard let baselineRead = Self.runtimeCounter(
            "tunPacketsRead",
            from: baselineRuntimeJson
        ),
            let baselineBytesRead = Self.runtimeCounter(
                "tunBytesRead",
                from: baselineRuntimeJson
            ),
            let baselineDropped = Self.runtimeCounter(
                "tunPacketsDropped",
                from: baselineRuntimeJson
            )
        else {
            result["tunPacketProbeError"] = "baseline TUN counters missing"
            return result
        }
        result["tunPacketProbeBaselineRead"] = Self.jsonCounterValue(baselineRead)
        result["tunPacketProbeBaselineBytesRead"] = Self.jsonCounterValue(baselineBytesRead)
        result["tunPacketProbeBaselineDropped"] = Self.jsonCounterValue(baselineDropped)

        var sentPackets = 0
        var sendErrors: [String] = []
        for _ in 0..<packetCount {
            if let sendError = Self.sendDebugUdpPacket(target: target, port: port) {
                sendErrors.append(sendError)
            } else {
                sentPackets += 1
            }
        }
        result["tunPacketProbeSentPackets"] = sentPackets
        let requiredRead = Self.saturatingAdd(baselineRead, UInt64(sentPackets))
        result["tunPacketProbeRequiredRead"] = Self.jsonCounterValue(requiredRead)
        if !sendErrors.isEmpty {
            result["tunPacketProbeSendError"] = sendErrors.joined(separator: "; ")
        }
        guard sentPackets > 0 else {
            result["tunPacketProbeError"] = "no UDP probe packets sent"
            return result
        }

        let deadline = Date().addingTimeInterval(waitSeconds)
        let probeStartedAt = Date()
        var finalRead = baselineRead
        var finalBytesRead = baselineBytesRead
        var finalDropped = baselineDropped
        var pollCount = 0
        var firstObservedAt: Date?
        while Date() < deadline {
            pollCount += 1
            let runtimeJson = await vpnController.runtimeStateJson()
            if let currentRead = Self.runtimeCounter(
                "tunPacketsRead",
                from: runtimeJson
            ),
                let currentBytesRead = Self.runtimeCounter(
                    "tunBytesRead",
                    from: runtimeJson
                ),
                let currentDropped = Self.runtimeCounter(
                    "tunPacketsDropped",
                    from: runtimeJson
                )
            {
                finalRead = currentRead
                finalBytesRead = currentBytesRead
                finalDropped = currentDropped
                if currentRead > baselineRead && firstObservedAt == nil {
                    firstObservedAt = Date()
                }
                if currentRead >= requiredRead {
                    Self.finishTunPacketProbeResult(
                        &result,
                        sentPackets: sentPackets,
                        baselineRead: baselineRead,
                        baselineBytesRead: baselineBytesRead,
                        baselineDropped: baselineDropped,
                        finalRead: currentRead,
                        finalBytesRead: currentBytesRead,
                        finalDropped: currentDropped,
                        probeStartedAt: probeStartedAt,
                        firstObservedAt: firstObservedAt,
                        pollCount: pollCount
                    )
                    result["tunPacketProbeReadIncreased"] = true
                    return result
                }
            }
            try? await Task.sleep(nanoseconds: pollIntervalNanoseconds)
        }

        Self.finishTunPacketProbeResult(
            &result,
            sentPackets: sentPackets,
            baselineRead: baselineRead,
            baselineBytesRead: baselineBytesRead,
            baselineDropped: baselineDropped,
            finalRead: finalRead,
            finalBytesRead: finalBytesRead,
            finalDropped: finalDropped,
            probeStartedAt: probeStartedAt,
            firstObservedAt: firstObservedAt,
            pollCount: pollCount
        )
        return result
    }

    nonisolated private static func finishTunPacketProbeResult(
        _ result: inout [String: Any],
        sentPackets: Int,
        baselineRead: UInt64,
        baselineBytesRead: UInt64,
        baselineDropped: UInt64,
        finalRead: UInt64,
        finalBytesRead: UInt64,
        finalDropped: UInt64,
        probeStartedAt: Date,
        firstObservedAt: Date?,
        pollCount: Int
    ) {
        let observedPackets = saturatingSubtract(finalRead, baselineRead)
        let observedBytes = saturatingSubtract(finalBytesRead, baselineBytesRead)
        let droppedDelta = saturatingSubtract(finalDropped, baselineDropped)
        let missingPackets = saturatingSubtract(UInt64(sentPackets), observedPackets)
        result["tunPacketProbeFinalRead"] = jsonCounterValue(finalRead)
        result["tunPacketProbeObservedPackets"] = jsonCounterValue(observedPackets)
        result["tunPacketProbeMissingPackets"] = jsonCounterValue(missingPackets)
        result["tunPacketProbeFinalBytesRead"] = jsonCounterValue(finalBytesRead)
        result["tunPacketProbeObservedBytesRead"] = jsonCounterValue(observedBytes)
        result["tunPacketProbeBytesReadIncreased"] = observedBytes > 0
        result["tunPacketProbeFinalDropped"] = jsonCounterValue(finalDropped)
        result["tunPacketProbeDroppedDelta"] = jsonCounterValue(droppedDelta)
        result["tunPacketProbeDroppedIncreased"] = droppedDelta > 0
        result["tunPacketProbeElapsedMs"] = Int(Date().timeIntervalSince(probeStartedAt) * 1000)
        result["tunPacketProbePolls"] = pollCount
        if let firstObservedAt {
            result["tunPacketProbeFirstObservedMs"] = Int(
                firstObservedAt.timeIntervalSince(probeStartedAt) * 1000
            )
        }
    }

    nonisolated private static func sendDebugUdpPacket(target: String, port: UInt16) -> String? {
        let fd = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard fd >= 0 else {
            return String(cString: strerror(errno))
        }
        defer { close(fd) }

        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        guard inet_pton(AF_INET, target, &addr.sin_addr) == 1 else {
            return "invalid IPv4 target"
        }

        let payload = [UInt8]("nvpn".utf8)
        let sent = payload.withUnsafeBytes { bytes in
            withUnsafePointer(to: &addr) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockAddr in
                    sendto(
                        fd,
                        bytes.baseAddress,
                        bytes.count,
                        0,
                        sockAddr,
                        socklen_t(MemoryLayout<sockaddr_in>.size)
                    )
                }
            }
        }
        return sent < 0 ? String(cString: strerror(errno)) : nil
    }

    nonisolated private static func saturatingAdd(_ left: UInt64, _ right: UInt64) -> UInt64 {
        let (value, overflow) = left.addingReportingOverflow(right)
        return overflow ? UInt64.max : value
    }

    nonisolated private static func saturatingSubtract(_ left: UInt64, _ right: UInt64) -> UInt64 {
        left >= right ? left - right : 0
    }

    nonisolated private static func runtimeCounter(_ key: String, from json: String?) -> UInt64? {
        guard let json,
              let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let value = object[key]
        else {
            return nil
        }
        if let number = value as? NSNumber {
            return number.uint64Value
        }
        if let string = value as? String {
            return UInt64(string)
        }
        return nil
    }

    nonisolated private static func jsonCounterValue(_ value: UInt64) -> Any {
        if value <= UInt64(Int.max) {
            return Int(value)
        }
        return String(value)
    }

    private func writeDebugProbeResult(_ result: [String: Any], name: String) {
        guard let supportDir else {
            return
        }
        let safeName = name
            .split(separator: "/")
            .last
            .map(String.init) ?? "debug-exit-probe.json"
        let url = supportDir.appendingPathComponent(safeName)
        guard JSONSerialization.isValidJSONObject(result),
              let data = try? JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
        else {
            return
        }
        try? data.write(to: url, options: .atomic)
        debugLog("debug probe wrote \(url.path)")
    }

    nonisolated private static func argumentValue(after name: String, in arguments: [String]) -> String? {
        guard let index = arguments.firstIndex(of: name) else {
            return nil
        }
        let valueIndex = arguments.index(after: index)
        guard valueIndex < arguments.endIndex else {
            return nil
        }
        return arguments[valueIndex]
    }

    nonisolated private static func redactedDebugArguments(_ arguments: [String]) -> [String] {
        let sensitiveFlags = [
            "--nvpn-debug-import-invite",
            "--nvpn-debug-exit-node",
            "--nvpn-debug-fetch-url",
            "--nvpn-debug-result",
            "--nvpn-debug-wireguard-config-base64",
            "--nvpn-debug-wireguard-config-file",
        ]
        var output: [String] = []
        var redactNext = false
        for argument in arguments {
            if redactNext {
                output.append("<redacted>")
                redactNext = false
                continue
            }
            if let flag = sensitiveFlags.first(where: { argument.hasPrefix($0 + "=") }) {
                output.append("\(flag)=<redacted>")
                continue
            }
            output.append(argument)
            if sensitiveFlags.contains(argument) {
                redactNext = true
            }
        }
        return output
    }

    nonisolated private static func wireGuardConfig(from arguments: [String], supportDir: URL?) -> String? {
        if let encoded = argumentValue(after: "--nvpn-debug-wireguard-config-base64", in: arguments),
           let data = Data(base64Encoded: encoded),
           let config = String(data: data, encoding: .utf8) {
            return config
        }
        guard let path = argumentValue(after: "--nvpn-debug-wireguard-config-file", in: arguments) else {
            return nil
        }
        let url: URL
        if path.hasPrefix("/") {
            url = URL(fileURLWithPath: path)
        } else if let supportDir {
            url = supportDir.appendingPathComponent(path)
        } else {
            return nil
        }
        return try? String(contentsOf: url)
    }

    nonisolated private static func resolveDebugHost(_ host: String) -> (addresses: [String], error: String?) {
        var hints = addrinfo()
        hints.ai_family = AF_UNSPEC
        hints.ai_socktype = SOCK_STREAM

        var result: UnsafeMutablePointer<addrinfo>?
        let status = getaddrinfo(host, nil, &hints, &result)
        guard status == 0 else {
            let message = gai_strerror(status).map { String(cString: $0) } ?? "getaddrinfo failed"
            return ([], message)
        }
        defer { freeaddrinfo(result) }

        var addresses = Set<String>()
        var cursor = result
        while let current = cursor {
            var buffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            let info = current.pointee
            if getnameinfo(
                info.ai_addr,
                info.ai_addrlen,
                &buffer,
                socklen_t(buffer.count),
                nil,
                0,
                NI_NUMERICHOST
            ) == 0 {
                addresses.insert(String(cString: buffer))
            }
            cursor = info.ai_next
        }

        return (Array(addresses).sorted(), nil)
    }

    func qrMatrix(for invite: String) -> QrMatrix {
        if fixtureMode {
            return ScreenshotFixtures.qrMatrix()
        }
        return core?.qrMatrix(invite: invite) ?? QrMatrix()
    }

    func copy(_ value: String) {
        guard !value.isEmpty else {
            return
        }
        UIPasteboard.general.string = value
        copiedValue = value
        copyClearTask?.cancel()
        copyClearTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            await MainActor.run {
                if self?.copiedValue == value {
                    self?.copiedValue = ""
                }
            }
        }
    }

    private static func seedMobileConfig(in supportDir: URL, deviceName: String) {
        let name = deviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else {
            return
        }

        let config = supportDir.appendingPathComponent(configFileName)
        guard !FileManager.default.fileExists(atPath: config.path) else {
            return
        }

        let escaped = name
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        try? "node_name = \"\(escaped)\"\n".write(to: config, atomically: true, encoding: .utf8)
    }

    private func refreshTunnelSidecarState() {
        guard state.vpnEnabled, !tunnelStateRefreshInFlight else {
            return
        }
        tunnelStateRefreshInFlight = true
        Task { [weak self] in
            let runtimeJson = await self?.vpnController.runtimeStateJson()
            let appConfigToml = await self?.vpnController.takeAppConfigToml()
            await MainActor.run {
                guard let self else {
                    return
                }
                self.tunnelStateRefreshInFlight = false
                var wrote = false
                if let appConfigToml, self.writeTunnelAppConfigIfNeeded(appConfigToml) {
                    wrote = true
                }
                if let runtimeJson, self.writeTunnelRuntimeStateIfNeeded(runtimeJson) {
                    wrote = true
                }
                if wrote {
                    self.state = self.core?.refresh() ?? self.state
                }
            }
        }
    }

    private func writeTunnelRuntimeStateIfNeeded(_ json: String) -> Bool {
        let trimmed = json.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let data = trimmed.data(using: .utf8),
              (try? JSONSerialization.jsonObject(with: data)) != nil
        else {
            return false
        }
        return writeSupportFileIfChanged(data, name: Self.mobileRuntimeStateFileName)
    }

    private func writeTunnelAppConfigIfNeeded(_ toml: String) -> Bool {
        let trimmed = toml.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.hasPrefix("# failed") else {
            return false
        }
        guard let data = toml.data(using: .utf8) else {
            return false
        }
        return writeSupportFileIfChanged(data, name: Self.configFileName)
    }

    private func writeSupportFileIfChanged(_ data: Data, name: String) -> Bool {
        guard let supportDir else {
            return false
        }
        try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
        let url = supportDir.appendingPathComponent(name)
        if let existing = try? Data(contentsOf: url), existing == data {
            return false
        }
        do {
            try data.write(to: url, options: .atomic)
            debugLog("wrote tunnel sidecar file \(name)")
            return true
        } catch {
            debugLog("failed to write tunnel sidecar file \(name): \(String(describing: error))")
            return false
        }
    }

    nonisolated static func supportDirectory() -> URL? {
        let legacy = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent("Nostr VPN", isDirectory: true)
        guard let shared = FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier)?
            .appendingPathComponent("Nostr VPN", isDirectory: true)
        else {
            return legacy
        }
        migrateLegacySupportDirectory(from: legacy, to: shared)
        return shared
    }

    nonisolated private static func migrateLegacySupportDirectory(from legacy: URL?, to shared: URL) {
        guard let legacy, legacy.path != shared.path else {
            return
        }
        let manager = FileManager.default
        guard manager.fileExists(atPath: legacy.path) else {
            return
        }
        try? manager.createDirectory(at: shared, withIntermediateDirectories: true)
        guard let items = try? manager.contentsOfDirectory(at: legacy, includingPropertiesForKeys: nil)
        else {
            return
        }
        for item in items {
            let destination = shared.appendingPathComponent(item.lastPathComponent)
            guard !manager.fileExists(atPath: destination.path) else {
                continue
            }
            try? manager.copyItem(at: item, to: destination)
        }
    }

    private static func deviceName() -> String {
        let preferred = UIDevice.current.name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !preferred.isEmpty {
            return preferred
        }

        let model = UIDevice.current.model.trimmingCharacters(in: .whitespacesAndNewlines)
        return model.isEmpty ? "iOS device" : model
    }

    nonisolated static func screenshotTabArgument() -> String? {
        argumentValue(after: "--nvpn-screenshot-tab", in: ProcessInfo.processInfo.arguments)
    }

    nonisolated private static func fixtureModeRequested() -> Bool {
        #if DEBUG
        let arguments = ProcessInfo.processInfo.arguments
        if arguments.contains("--nvpn-fixture-mode") {
            return true
        }
        let raw = ProcessInfo.processInfo.environment["NVPN_IOS_FIXTURE_MODE"] ?? ""
        return ["1", "true", "yes", "on"].contains(raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased())
        #else
        return false
        #endif
    }

    private func debugLog(_ message: String) {
        #if DEBUG
        guard let supportDir else {
            return
        }
        let line = "[\(Date())] \(message)\n"
        guard let data = line.data(using: .utf8) else {
            return
        }
        let logUrl = supportDir.appendingPathComponent("app-debug.log")
        if FileManager.default.fileExists(atPath: logUrl.path),
           let handle = try? FileHandle(forWritingTo: logUrl)
        {
            handle.seekToEndOfFile()
            handle.write(data)
            try? handle.close()
        } else {
            try? data.write(to: logUrl)
        }
        #endif
    }
}
