import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct DevicesPage: View {
    @ObservedObject var model: AppModel
    let network: NetworkState?
    @State private var addDevicePresented = false
    @State private var pendingNetworkRemoval: NetworkState?

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                if !model.state.error.isEmpty || shouldShowStatusNotice(model.statusMessage) {
                    NoticeCard(
                        text: model.state.error.isEmpty ? model.statusMessage : model.state.error,
                        actionTitle: nil,
                        action: {}
                    )
                }
                if let network {
                    if !network.enabled {
                        Button {
                            model.dispatch(
                                NativeActions.setNetworkEnabled(network.id, true),
                                status: "Activating network"
                            )
                        } label: {
                            Label("Activate Network", systemImage: "checkmark.circle.fill")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(model.actionInFlight)
                    }
                    if network.localIsAdmin {
                        Button {
                            addDevicePresented = true
                        } label: {
                            Label("Link device", systemImage: "person.badge.plus")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                        .disabled(!network.enabled)
                    }
                    ForEach(sortedParticipants(network.participants, state: model.state)) { participant in
                        ParticipantRow(model: model, network: network, participant: participant)
                    }
                    Button(role: .destructive) {
                        pendingNetworkRemoval = network
                    } label: {
                        Label("Delete network", systemImage: "trash")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .padding(.top, 8)
                } else {
                    NoticeCard(text: "No network")
                }
            }
            .padding()
        }
        .safeAreaPadding(.bottom, 92)
        .background(AppColors.background)
        .sheet(isPresented: $addDevicePresented) {
            if let network {
                NavigationStack {
                    AddDeviceSheet(model: model, network: network)
                        .navigationTitle("Link Device")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("Done") {
                                    addDevicePresented = false
                                }
                            }
                        }
                }
            }
        }
        .confirmationDialog(
            "Delete \(pendingNetworkRemoval?.displayName ?? "network")?",
            isPresented: Binding(
                get: { pendingNetworkRemoval != nil },
                set: { if !$0 { pendingNetworkRemoval = nil } }
            ),
            titleVisibility: .visible,
            presenting: pendingNetworkRemoval
        ) { network in
            Button("Delete", role: .destructive) {
                model.dispatch(NativeActions.removeNetwork(network.id), status: "Deleting network")
                pendingNetworkRemoval = nil
            }
            Button("Cancel", role: .cancel) { pendingNetworkRemoval = nil }
        } message: { _ in
            Text("Removes the network from this device. You can link it again later.")
        }
    }

    private func shouldShowStatusNotice(_ message: String) -> Bool {
        !message.isEmpty && message != AppModel.vpnDisclosurePromptMessage
    }

}
struct ToolbarVpnSwitch: View {
    @ObservedObject var model: AppModel
    let vpnDisclosureAccepted: Bool
    let onReviewVpnDisclosure: () -> Void

    private var enabled: Bool {
        !model.actionInFlight && model.state.vpnControlSupported && model.activeNetwork != nil
    }

    var body: some View {
        Button {
            if !model.state.vpnEnabled && !vpnDisclosureAccepted {
                model.requireVpnDisclosureReview()
                onReviewVpnDisclosure()
            } else {
                model.toggleVpn()
            }
        } label: {
            ZStack(alignment: model.state.vpnEnabled ? .trailing : .leading) {
                Capsule()
                    .fill(model.state.vpnEnabled ? AppColors.accent : Color.gray.opacity(0.24))
                    .frame(width: 48, height: 28)
                Circle()
                    .fill(Color.white)
                    .frame(width: 24, height: 24)
                    .shadow(color: .black.opacity(0.22), radius: 1, y: 1)
                    .padding(2)
            }
            .frame(width: 48, height: 28)
            .contentShape(Capsule())
            .opacity(enabled ? 1 : 0.55)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityLabel(model.state.vpnEnabled ? "Turn VPN off" : "Turn VPN on")
        .accessibilityValue(model.state.vpnEnabled ? "On" : "Off")
    }
}

struct VpnDisclosureSheet: View {
    let acknowledge: () -> Void

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 14) {
                Text("Before Turning VPN On")
                    .font(.title2.weight(.semibold))
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text("Nostr VPN is a private VPN and generic WireGuard exit-node utility. It is not a public VPN, anonymity, stealth, or consumer proxy service.")
                Text("The app uses VPN data only to operate networks you configure: device identity, peer lists, internet-sharing settings, endpoints, join request metadata, traffic counters, and connection health.")
                Text("Packet traffic is encrypted. User-selected peers, relays, bridge paths, and internet providers receive only the data needed to provide the connection you asked them to provide.")
                Text("The developer does not sell VPN data, use it for ads or tracking, or disclose it to third parties.")
                Spacer()
            }
            .font(.body)
            .foregroundStyle(.primary)
            .padding()
            .navigationTitle("VPN Data Use")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Continue", action: acknowledge)
                        .fontWeight(.semibold)
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

struct CreateNetworkCard: View {
    @ObservedObject var model: AppModel
    var onCreated: (() -> Void)? = nil
    @State private var networkName = "My Network"

    var body: some View {
        SetupCard(title: "Create Network", systemImage: "plus.circle.fill", tint: AppColors.create) {
            VStack(alignment: .leading, spacing: 10) {
                TextField("Network name", text: $networkName)
                    .textFieldStyle(.roundedBorder)
                Button {
                    let name = networkName.trimmingCharacters(in: .whitespacesAndNewlines)
                    model.dispatch(
                        NativeActions.addNetwork(name.isEmpty ? "My Network" : name),
                        status: "Creating network"
                    )
                    networkName = "My Network"
                    onCreated?()
                } label: {
                    Label("Create", systemImage: "plus")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.actionInFlight)
            }
        }
    }
}

struct JoinNetworkCard: View {
    @ObservedObject var model: AppModel
    @State private var inviteInput = ""
    @State private var inviteExpanded = false
    @State private var manualExpanded = false
    @State private var manualAdminId = ""
    @State private var manualNetworkId = ""

    private var manualAdminInvalid: Bool {
        let trimmed = manualAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
        return !trimmed.isEmpty && !isValidDeviceId(trimmed)
    }

    private var canSubmitManual: Bool {
        let admin = manualAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
        let mesh = normalizeNetworkIdInput(manualNetworkId)
        return !admin.isEmpty && !mesh.isEmpty && isValidDeviceId(admin)
    }

    private var requestNetwork: NetworkState? {
        model.activeNetwork ?? model.state.networks.first { network in
            !network.joinRequestQrCodeOrLink.isEmpty
        }
    }

    private var joinRequestQrCodeOrLink: String {
        if !model.state.joinRequestQrCodeOrLink.isEmpty {
            return model.state.joinRequestQrCodeOrLink
        }
        return requestNetwork?.joinRequestQrCodeOrLink ?? ""
    }

    var body: some View {
        SetupCard(title: "Join Network", systemImage: "arrow.down.circle.fill", tint: AppColors.join) {
            if !joinRequestQrCodeOrLink.isEmpty {
                Pill("Join request", tint: .orange)
                VStack(alignment: .leading, spacing: 8) {
                    QrCodeView(matrix: model.qrMatrix(for: joinRequestQrCodeOrLink))
                        .aspectRatio(1, contentMode: .fit)
                        .frame(maxWidth: .infinity, alignment: .center)
                    HStack(spacing: 10) {
                        Button {
                            model.copy(joinRequestQrCodeOrLink)
                        } label: {
                            Label("Copy Request", systemImage: model.copiedValue == joinRequestQrCodeOrLink ? "checkmark" : "doc.on.doc")
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                        if let requestUrl = URL(string: joinRequestQrCodeOrLink) {
                            ShareLink(item: requestUrl) {
                                Label("Share", systemImage: "square.and.arrow.up")
                                    .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(.bordered)
                        }
                    }
                }
            }

            DisclosureGroup("Invite link", isExpanded: $inviteExpanded) {
                VStack(alignment: .leading, spacing: 8) {
                    TextField("nvpn://invite/…", text: $inviteInput)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.roundedBorder)
                        .onChange(of: inviteInput) { _, newValue in
                            let trimmed = newValue.trimmingCharacters(in: .whitespacesAndNewlines)
                            if trimmed.lowercased().hasPrefix("nvpn://invite/") {
                                model.linkNetwork(trimmed)
                                inviteInput = ""
                            }
                        }
                    Button {
                        if let text = UIPasteboard.general.string {
                            inviteInput = text.trimmingCharacters(in: .whitespacesAndNewlines)
                        }
                    } label: {
                        Label("Paste", systemImage: "doc.on.clipboard")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                }
                .padding(.top, 6)
            }
            .font(.subheadline)

            DisclosureGroup("Manual join", isExpanded: $manualExpanded) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Give the admin your Device ID, then enter their Device ID and network ID.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Your Device ID")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        CopyLine(value: model.state.ownNpub, model: model)
                    }
                    TextField("Admin Device ID", text: $manualAdminId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.roundedBorder)
                        .overlay(
                            RoundedRectangle(cornerRadius: 6)
                                .stroke(Color.red, lineWidth: manualAdminInvalid ? 1 : 0)
                        )
                    if manualAdminInvalid {
                        Text("Not a valid device ID")
                            .font(.caption)
                            .foregroundStyle(.red)
                    }
                    TextField("Network ID", text: $manualNetworkId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.roundedBorder)
                    Button("Add") {
                        let admin = manualAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
                        let mesh = normalizeNetworkIdInput(manualNetworkId)
                        model.dispatch(
                            NativeActions.manualAddNetwork(adminNpub: admin, meshNetworkId: mesh),
                            status: "Adding network"
                        )
                        manualAdminId = ""
                        manualNetworkId = ""
                        manualExpanded = false
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(!canSubmitManual)
                }
                .padding(.top, 6)
            }
            .font(.subheadline)
        }
    }
}

/// Admin-only sheet for linking a device to YOUR network. The preferred path
/// is scanning or pasting the joining device's join request; direct Device ID
/// entry remains for compatible signed-roster clients.
struct AddDeviceSheet: View {
    @ObservedObject var model: AppModel
    let network: NetworkState
    @Environment(\.dismiss) private var dismiss
    @State private var qrScannerPresented = false
    @State private var scannedQrCode: String?
    @State private var scanError = ""
    @State private var joinRequestInput = ""
    @State private var pendingJoinRequest: PendingJoinRequest?

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                ScanJoinerDeviceCard(
                    requestInput: $joinRequestInput,
                    scanError: scanError,
                    scan: { qrScannerPresented = true },
                    paste: {
                        if let text = UIPasteboard.general.string {
                            importJoinerValue(text)
                        }
                    },
                    inputChanged: { value in
                        stageJoinRequest(value)
                    }
                )
                NearbyCard(model: model)
                ManualPairingInfoCard(model: model, network: network)
                AddDeviceCard(network: network) { npub, alias in
                    model.dispatch(
                        NativeActions.addParticipant(networkId: network.id, npub: npub, alias: alias),
                        status: "Adding device"
                    )
                }
            }
            .padding()
        }
        .safeAreaPadding(.bottom, 92)
        .background(AppColors.background)
        .sheet(isPresented: $qrScannerPresented, onDismiss: qrScannerDismissed) {
            QRCodeScannerSheet { code in
                scannedQrCode = code
                qrScannerPresented = false
            }
        }
        .alert("Add device?", isPresented: pendingJoinRequestPresented, presenting: pendingJoinRequest) { pending in
            Button("Cancel", role: .cancel) {
                pendingJoinRequest = nil
            }
            Button("Add") {
                model.dispatch(NativeActions.importJoinRequest(pending.request), status: "Adding device")
                joinRequestInput = ""
                pendingJoinRequest = nil
                dismiss()
            }
        } message: { pending in
            Text("Add the device from this join request to \(pending.networkName)?")
        }
    }

    private var pendingJoinRequestPresented: Binding<Bool> {
        Binding(
            get: { pendingJoinRequest != nil },
            set: { presented in
                if !presented {
                    pendingJoinRequest = nil
                }
            }
        )
    }

    private func qrScannerDismissed() {
        guard let code = scannedQrCode else {
            return
        }
        scannedQrCode = nil
        DispatchQueue.main.async {
            handleScannedJoinerCode(code)
        }
    }

    private func importJoinRequest(_ value: String) {
        let request = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !request.isEmpty else { return }
        scanError = ""
        model.dispatch(
            NativeActions.importJoinRequest(request),
            status: "Adding device"
        )
    }

    private func importJoinerValue(_ value: String) {
        let request = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !request.isEmpty else { return }
        if looksLikeJoinRequestQrOrLink(request) {
            stageJoinRequest(request)
            return
        }
        if let scanned = parseScannedDeviceLinkQr(request) {
            addScannedJoiner(scanned)
            return
        }
        importJoinRequest(request)
    }

    private func handleScannedJoinerCode(_ value: String) {
        if looksLikeJoinRequestQrOrLink(value) {
            stageJoinRequest(value)
            return
        }
        guard let scanned = parseScannedDeviceLinkQr(value) else {
            scanError = "Not a Nostr VPN joiner QR."
            return
        }
        addScannedJoiner(scanned)
    }

    private func addScannedJoiner(_ scanned: ScannedDeviceLink) {
        scanError = ""
        model.dispatch(
            NativeActions.addParticipant(
                networkId: network.id,
                npub: scanned.deviceId,
                alias: scanned.alias ?? ""
            ),
            status: "Adding device"
        )
    }

    private func stageJoinRequest(_ value: String) {
        let request = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard looksLikeJoinRequestQrOrLink(request) else { return }
        scanError = ""
        pendingJoinRequest = PendingJoinRequest(
            networkName: network.name.isEmpty ? "this network" : network.name,
            request: request
        )
    }
}

struct ScanJoinerDeviceCard: View {
    @Binding var requestInput: String
    let scanError: String
    let scan: () -> Void
    let paste: () -> Void
    let inputChanged: (String) -> Void

    var body: some View {
        AppCard {
            Text("Add join request")
                .font(.headline)
            Text("Scan or paste the joining device's join request. Valid links open confirmation automatically.")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextField("Join request", text: $requestInput)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
                .onChange(of: requestInput) { _, value in
                    inputChanged(value)
                }
            Button(action: paste) {
                Label("Paste", systemImage: "doc.on.clipboard")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
            Button(action: scan) {
                Label("Scan QR", systemImage: "camera.viewfinder")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            if !scanError.isEmpty {
                Text(scanError)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }
}

/// Manual pairing path for directly sharing signed-roster values.
struct ManualPairingInfoCard: View {
    @ObservedObject var model: AppModel
    let network: NetworkState

    var body: some View {
        AppCard {
            Text("Manual pairing")
                .font(.headline)
            Text("Share these values with the other device, then add its Device ID below to keep the signed roster in sync.")
                .font(.caption)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 4) {
                Text("Your Device ID")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                CopyLine(value: model.state.ownNpub, model: model)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("Network ID")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                CopyLine(value: network.networkId, displayValue: displayNetworkId(network.networkId), model: model)
            }
        }
    }
}
