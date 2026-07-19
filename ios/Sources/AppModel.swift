import Foundation
import Darwin
import SwiftUI
import UIKit

@MainActor
final class AppModel: ObservableObject {
    nonisolated static let appGroupIdentifier = Bundle.main.object(
        forInfoDictionaryKey: "NVPNAppGroupIdentifier"
    ) as? String ?? "group.fi.siriusbusiness.nvpn"
    static let configFileName = "config.toml"
    static let mobileRuntimeStateFileName = "mobile-runtime-state.json"
    static let vpnDisclosureAcceptedKey = "vpnDisclosureAccepted"
    static let vpnDisclosurePromptMessage = "Review VPN data use before turning VPN on."

    @Published var state: AppState
    @Published var actionInFlight = false
    @Published var statusMessage = ""
    @Published var copiedValue = ""
    @Published var vpnDisclosurePromptVisible = false

    let core: NativeCoreClient?
    let vpnController = PacketTunnelController()
    let supportDir: URL?
    let fixtureMode: Bool
    private var refreshTask: Task<Void, Never>?
    var copyClearTask: Task<Void, Never>?
    private var tunnelConfigSyncTask: Task<Void, Never>?
    var launchAutomationHandled = false
    var tunnelStateRefreshInFlight = false

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
        if !launchAutomationHandled {
            // A running unjoined tunnel may already hold a completed approval
            // that the app has not copied back yet. Do not restart it from the
            // stale QR-side config on every UI-process launch; the sidecar poll
            // below consumes the completed config first. A disconnected tunnel
            // is still started normally.
            ensureAutoconnectPacketTunnel(reason: "startup")
        }
    }

    func refresh() {
        guard let core else {
            state.rev += 1
            return
        }
        let hadActiveNetwork = activeNetwork != nil
        state = core.refresh()
        refreshTunnelSidecarState()
        if !hadActiveNetwork, activeNetwork != nil {
            ensureAutoconnectPacketTunnel(reason: "network joined")
        }
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
            let force = actionType == "remove_network"
                && activeNetwork == nil
                && !state.joinRequestQrCodeOrLink.isEmpty
            schedulePacketTunnelConfigSync(reason: actionType, force: force)
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

    private func ensureAutoconnectPacketTunnel(reason: String) {
        let canReceiveDeviceApproval = !state.joinRequestQrCodeOrLink.isEmpty
        guard state.autoconnect, activeNetwork != nil || canReceiveDeviceApproval else {
            return
        }
        guard UserDefaults.standard.bool(forKey: Self.vpnDisclosureAcceptedKey) else {
            requireVpnDisclosureReview()
            return
        }
        Task { [weak self] in
            guard let self else { return }
            let status = await vpnController.statusRawValue()
            guard Self.packetTunnelNeedsStart(statusRawValue: status) else {
                debugLog("autoconnect skipped reason=\(reason) tunnelStatus=\(status ?? -1)")
                return
            }
            debugLog("autoconnect starting PacketTunnel reason=\(reason) tunnelStatus=\(status ?? -1)")
            setVpnEnabled(true, force: true)
        }
    }

    static func packetTunnelNeedsStart(statusRawValue: Int?) -> Bool {
        guard let statusRawValue else {
            return true
        }
        // NEVPNStatus: invalid=0, disconnected=1, connecting=2,
        // connected=3, reasserting=4, disconnecting=5.
        return statusRawValue <= 1 || statusRawValue == 5
    }

    func setVpnEnabled(_ enabled: Bool, force: Bool = false) {
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
                    if statusMessage == "Turning VPN on" {
                        statusMessage = state.error
                    }
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

    private func schedulePacketTunnelConfigSync(reason: String, force: Bool = false) {
        guard !fixtureMode else {
            return
        }
        guard !force || UserDefaults.standard.bool(forKey: Self.vpnDisclosureAcceptedKey) else {
            debugLog("PacketTunnel config sync skipped reason=\(reason) disclosure pending")
            return
        }
        guard force || state.vpnEnabled || state.vpnActive else {
            debugLog("PacketTunnel config sync skipped reason=\(reason) vpn off")
            return
        }
        tunnelConfigSyncTask?.cancel()
        tunnelConfigSyncTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 250_000_000)
            await self?.syncPacketTunnelConfig(reason: reason, force: force)
        }
    }

    private func syncPacketTunnelConfig(reason: String, force: Bool) async {
        guard let core else {
            statusMessage = "Native core unavailable"
            return
        }
        guard force || state.vpnEnabled || state.vpnActive else {
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
             "add_network",
             "manual_add_network",
             "remove_network",
             "set_network_enabled",
             "set_network_mesh_id",
             "add_participant",
             "set_participant_endpoint_hints",
             "add_admin",
             "remove_participant",
             "remove_admin":
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
        if raw.lowercased().hasPrefix("nvpn://join-request") {
            dispatch(NativeActions.importJoinRequest(raw), status: "Adding device")
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

}
