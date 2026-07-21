import Foundation
import UIKit

extension AppModel {
    func qrMatrix(for text: String) -> QrMatrix {
        if fixtureMode {
            return ScreenshotFixtures.qrMatrix()
        }
        return core?.qrMatrix(text: text) ?? QrMatrix()
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

    static func seedMobileConfig(in supportDir: URL, deviceName: String) {
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

    func refreshTunnelSidecarState() {
        guard state.vpnEnabled, !tunnelStateRefreshInFlight else {
            return
        }
        tunnelStateRefreshInFlight = true
        Task { [weak self] in
            // Approval completion is transactional state. Read it before the
            // observational runtime snapshot so a slow snapshot cannot leave
            // the UI displaying a stale QR after the tunnel already joined.
            let appConfigToml = await self?.vpnController.takeAppConfigToml()
            let runtimeJson = await self?.vpnController.runtimeStateJson()
            let acceptance = await MainActor.run {
                guard let self else {
                    return (accepted: false, networkChanged: false)
                }
                let previousNetworkId = self.activeNetwork?.networkId ?? ""
                var wrote = false
                var appConfigAccepted = false
                if let appConfigToml {
                    let result = self.acceptTunnelAppConfig(appConfigToml)
                    appConfigAccepted = result.accepted
                    wrote = result.wrote
                }
                if let runtimeJson, self.writeTunnelRuntimeStateIfNeeded(runtimeJson) {
                    wrote = true
                }
                if wrote {
                    self.state = self.core?.refresh() ?? self.state
                }
                let currentNetworkId = self.activeNetwork?.networkId ?? ""
                return (
                    accepted: appConfigAccepted,
                    networkChanged: wrote && previousNetworkId != currentNetworkId
                )
            }
            if acceptance.accepted {
                _ = await self?.vpnController.acknowledgeAppConfigToml()
            }
            await MainActor.run {
                self?.tunnelStateRefreshInFlight = false
                if acceptance.networkChanged {
                    // A FIPS endpoint's discovery scope is bound to its network
                    // id. Restart exactly once after a join so the accepted
                    // roster does not remain on its QR/onboarding scope.
                    self?.schedulePacketTunnelConfigSync(reason: "network joined", force: true)
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

    private func acceptTunnelAppConfig(_ toml: String) -> (accepted: Bool, wrote: Bool) {
        let trimmed = toml.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !trimmed.hasPrefix("# failed") else {
            return (false, false)
        }
        guard let supportDir, let data = toml.data(using: .utf8) else {
            return (false, false)
        }
        try? FileManager.default.createDirectory(at: supportDir, withIntermediateDirectories: true)
        let url = supportDir.appendingPathComponent(Self.configFileName)
        if let existing = try? Data(contentsOf: url), existing == data {
            return (true, false)
        }
        do {
            try data.write(to: url, options: .atomic)
            debugLog("wrote tunnel sidecar file \(Self.configFileName)")
            return (true, true)
        } catch {
            debugLog("failed to write tunnel sidecar file \(Self.configFileName): \(String(describing: error))")
            return (false, false)
        }
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

    static func deviceName() -> String {
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

    nonisolated static func fixtureModeRequested() -> Bool {
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

    func debugLog(_ message: String) {
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
