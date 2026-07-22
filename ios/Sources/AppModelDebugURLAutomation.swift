import Foundation

extension AppModel {
    nonisolated static func debugArguments(fromBase64URL encoded: String) -> [String]? {
        var padded = encoded.replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let remainder = padded.count % 4
        if remainder != 0 {
            padded += String(repeating: "=", count: 4 - remainder)
        }
        guard let data = Data(base64Encoded: padded),
              let value = try? JSONSerialization.jsonObject(with: data),
              let arguments = value as? [String],
              !arguments.isEmpty,
              arguments.count <= 64,
              arguments.allSatisfy({ !$0.contains("\0") }),
              arguments[0].hasPrefix("--nvpn-debug-") || arguments[0].hasPrefix("--nvpn-")
        else {
            return nil
        }
        return arguments
    }

    nonisolated static func redactedDebugArguments(_ arguments: [String]) -> [String] {
        let sensitiveFlags = [
            "--nvpn-debug-exit-node",
            "--nvpn-debug-fetch-url",
            "--nvpn-debug-direct-fetch-url",
            "--nvpn-debug-direct-resolve-host",
            "--nvpn-debug-result",
            "--nvpn-debug-idle-cpu-result",
            "--nvpn-debug-tun-probe-target",
            "--nvpn-debug-wireguard-config-base64",
            "--nvpn-debug-wireguard-config-file",
            "--nvpn-debug-connect-result",
            "--nvpn-debug-runtime-result",
            "--nvpn-debug-import-join-request-base64",
            "--nvpn-debug-export-join-request",
            "--nvpn-debug-manual-join-admin-base64",
            "--nvpn-debug-manual-join-network-base64",
            "--nvpn-debug-add-participant-base64",
            "--nvpn-debug-remove-participant-base64",
            "--nvpn-debug-select-network-base64",
            "--nvpn-debug-select-network-result",
            "--nvpn-debug-wait-for-joined-network-base64",
            "--nvpn-debug-wait-for-joined-network-result",
            "--nvpn-debug-wait-for-joined-network-timeout-seconds",
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
}
