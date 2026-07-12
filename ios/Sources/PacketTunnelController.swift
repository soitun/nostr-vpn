import Foundation
import NetworkExtension

enum PacketTunnelControllerError: LocalizedError {
    case managerUnavailable
    case preferencesTimedOut(String)

    var errorDescription: String? {
        switch self {
        case .managerUnavailable:
            return "VPN manager unavailable"
        case .preferencesTimedOut(let operation):
            return "\(operation) VPN preferences timed out; approve any iOS VPN configuration prompt and retry"
        }
    }
}

final class PacketTunnelController {
    private static let preferencesOperationTimeoutSeconds: TimeInterval = 10
    private let providerBundleIdentifier = Bundle.main.object(
        forInfoDictionaryKey: "NVPNPacketTunnelBundleIdentifier"
    ) as? String ?? "fi.siriusbusiness.nvpn.PacketTunnel"
    private var activeManager: NETunnelProviderManager?

    func start(
        state: AppState,
        network: NetworkState?,
        tunnelConfigJson: String,
        providerOptionsConfigJson: String
    ) async throws {
        debugLog("PacketTunnelController.start begin")
        let manager = try await loadOrCreateManager()
        activeManager = manager
        let proto = (manager.protocolConfiguration as? NETunnelProviderProtocol) ?? NETunnelProviderProtocol()
        proto.providerBundleIdentifier = providerBundleIdentifier
        proto.serverAddress = network?.displayName ?? "Nostr VPN"
        proto.providerConfiguration = [
            "networkName": network?.displayName ?? "Nostr VPN",
            "tunnelIp": state.tunnelIp.isEmpty ? "10.44.0.1/32" : state.tunnelIp,
            "mtu": 1150,
            "mobileTunnelConfigJson": tunnelConfigJson,
        ]
        // Tell iOS to actually use the includedRoutes we install
        // (without this iOS sometimes lets system services bypass the
        // tunnel, which is also the only condition under which the
        // VPN status badge stays hidden).
        proto.enforceRoutes = true
        if #available(iOS 14.0, *) {
            proto.includeAllNetworks = Self.hasDefaultRoute(in: providerOptionsConfigJson)
        }
        // Don't tear the tunnel down when the screen locks — for a
        // utility VPN we want it to keep running.
        proto.disconnectOnSleep = false
        manager.protocolConfiguration = proto
        manager.localizedDescription = "Nostr VPN"
        manager.isEnabled = true
        debugLog("saving preferences")
        try await save(manager)
        debugLog("reloading preferences")
        try await reload(manager)
        debugLog("calling startVPNTunnel status=\(manager.connection.status.rawValue)")
        // Keep providerConfiguration redacted in VPN preferences; the full
        // config is delivered only to this start attempt.
        let options: [String: NSObject] = [
            "mobileTunnelConfigJson": providerOptionsConfigJson as NSString,
        ]
        try manager.connection.startVPNTunnel(options: options)
        debugLog("startVPNTunnel returned status=\(manager.connection.status.rawValue)")
    }

    private static func hasDefaultRoute(in configJson: String) -> Bool {
        guard let data = configJson.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let routes = object["routeTargets"] as? [String]
        else {
            return false
        }
        return routes.contains("0.0.0.0/0")
    }

    func stop() async throws {
        debugLog("PacketTunnelController.stop begin")
        guard let manager = try await loadExistingManager() else {
            debugLog("stop skipped: no existing manager")
            return
        }
        activeManager = manager
        manager.connection.stopVPNTunnel()
        debugLog("stopVPNTunnel returned status=\(manager.connection.status.rawValue)")
    }

    func statusRawValue() async -> Int? {
        do {
            guard let manager = try await loadExistingManager() else {
                return nil
            }
            return manager.connection.status.rawValue
        } catch {
            debugLog("status failed: \(String(describing: error))")
            return nil
        }
    }

    func runtimeStateJson() async -> String? {
        guard let sizeData = await providerMessageData("runtimeStateBegin"),
              let sizeText = String(data: sizeData, encoding: .utf8),
              let expectedSize = Int(sizeText),
              expectedSize >= 0,
              expectedSize <= 1_048_576
        else {
            return await providerMessage("runtimeState")
        }
        var response = Data()
        response.reserveCapacity(expectedSize)
        while response.count < expectedSize {
            guard let chunk = await providerMessageData("runtimeStateChunk:\(response.count)"),
                  !chunk.isEmpty
            else {
                return nil
            }
            response.append(chunk)
        }
        guard response.count == expectedSize else {
            return nil
        }
        return String(data: response, encoding: .utf8)
    }

    func takeAppConfigToml() async -> String? {
        await providerMessage("takeAppConfig")
    }

    private func providerMessage(_ message: String) async -> String? {
        guard let response = await providerMessageData(message) else {
            return nil
        }
        return String(data: response, encoding: .utf8)
    }

    private func providerMessageData(_ message: String) async -> Data? {
        do {
            guard let manager = try await loadExistingManager() else {
                debugLog("providerMessage \(message) skipped: no existing manager")
                return nil
            }
            guard manager.connection.status == .connected else {
                debugLog("providerMessage \(message) skipped status=\(manager.connection.status.rawValue)")
                return nil
            }
            guard let session = manager.connection as? NETunnelProviderSession else {
                return nil
            }
            let data = message.data(using: .utf8) ?? Data()
            return try await withCheckedThrowingContinuation { continuation in
                do {
                    try session.sendProviderMessage(data) { response in
                        continuation.resume(returning: response)
                    }
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        } catch {
            debugLog("providerMessage \(message) failed: \(String(describing: error))")
            return nil
        }
    }

    private func loadOrCreateManager() async throws -> NETunnelProviderManager {
        if let existing = try await loadExistingManager() {
            debugLog("using existing manager status=\(existing.connection.status.rawValue)")
            return existing
        }
        debugLog("creating new manager")
        return NETunnelProviderManager()
    }

    private func loadExistingManager() async throws -> NETunnelProviderManager? {
        let managers = try await loadAllManagers()
        debugLog("loaded managers count=\(managers.count)")
        return managers.first(where: { manager in
            (manager.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier
                == providerBundleIdentifier
        })
    }

    private func loadAllManagers() async throws -> [NETunnelProviderManager] {
        try await withCheckedThrowingContinuation { continuation in
            NETunnelProviderManager.loadAllFromPreferences { managers, error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: managers ?? [])
                }
            }
        }
    }

    private func save(_ manager: NETunnelProviderManager) async throws {
        try await withPreferencesTimeout(operation: "save") { finish in
            manager.saveToPreferences { error in
                finish(error)
            }
        }
    }

    private func reload(_ manager: NETunnelProviderManager) async throws {
        try await withPreferencesTimeout(operation: "reload") { finish in
            manager.loadFromPreferences { error in
                finish(error)
            }
        }
    }

    private func withPreferencesTimeout(
        operation: String,
        start: (@escaping (Error?) -> Void) -> Void
    ) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            let completion = PreferenceOperationCompletion(continuation)
            let timeoutSeconds = Self.preferencesOperationTimeoutSeconds
            start { error in
                if let error {
                    _ = completion.resume(throwing: error)
                } else {
                    _ = completion.resume(returning: ())
                }
            }
            Task.detached(priority: .utility) {
                try? await Task.sleep(nanoseconds: UInt64(timeoutSeconds * 1_000_000_000))
                _ = completion.resume(
                    throwing: PacketTunnelControllerError.preferencesTimedOut(operation)
                )
            }
        }
    }

    private func debugLog(_ message: String) {
        #if DEBUG
        guard let supportDir = AppModel.supportDirectory() else {
            return
        }
        try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
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

private final class PreferenceOperationCompletion: @unchecked Sendable {
    private let lock = NSLock()
    private var completed = false
    private let continuation: CheckedContinuation<Void, Error>

    init(_ continuation: CheckedContinuation<Void, Error>) {
        self.continuation = continuation
    }

    @discardableResult
    func resume(returning value: Void) -> Bool {
        guard markCompleted() else {
            return false
        }
        continuation.resume(returning: value)
        return true
    }

    @discardableResult
    func resume(throwing error: Error) -> Bool {
        guard markCompleted() else {
            return false
        }
        continuation.resume(throwing: error)
        return true
    }

    private func markCompleted() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard !completed else {
            return false
        }
        completed = true
        return true
    }
}
