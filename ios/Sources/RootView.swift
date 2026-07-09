import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

private enum PaidInternetFeature {
    static var enabled: Bool {
        #if DEBUG
        let arguments = Set(ProcessInfo.processInfo.arguments)
        if arguments.contains("--nvpn-enable-paid-internet") {
            return true
        }
        return enabledFlag(ProcessInfo.processInfo.environment["NVPN_ENABLE_PAID_INTERNET"])
        #else
        return false
        #endif
    }

    private static func enabledFlag(_ value: String?) -> Bool {
        switch value?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "1", "true", "yes", "on":
            return true
        default:
            return false
        }
    }
}

struct RootView: View {
    @ObservedObject var model: AppModel
    @AppStorage(AppModel.vpnDisclosureAcceptedKey) private var vpnDisclosureAccepted = false
    @State private var addNetworkPresented = false
    @State private var vpnDisclosurePresented = false
    @State private var startVpnAfterDisclosure = false
    @State private var shownNetworkId: String?
    @State private var selectedTab = Self.initialTab()

    private var shownNetwork: NetworkState? {
        if let shownNetworkId,
           let network = model.state.networks.first(where: { $0.id == shownNetworkId }) {
            return network
        }
        return model.activeNetwork ?? model.state.networks.first
    }

    private var paidRouteMarketAvailable: Bool {
        PaidInternetFeature.enabled && model.state.paidRouteMarket.supported
    }

    var body: some View {
        Group {
            if model.state.networks.isEmpty {
                NavigationStack {
                    AddNetworkPage(
                        model: model,
                        showsWelcomeHeader: true,
                        onReviewVpnDisclosure: {
                            presentVpnDisclosure(startVpnAfterAccept: true)
                        }
                    )
                        .toolbar(.hidden, for: .navigationBar)
                }
            } else {
                TabView(selection: $selectedTab) {
                    NavigationStack {
                        DevicesPage(model: model, network: shownNetwork)
                            .toolbar { networkSwitcherToolbar }
                    }
                    .tabItem { Label("Devices", systemImage: "circle.grid.2x2.fill") }
                    .tag(AppTab.devices)

                    NavigationStack {
                        InternetPage(model: model, network: shownNetwork)
                            .navigationTitle("Internet")
                            .toolbar { networkSwitcherToolbar }
                    }
                    .tabItem { Label("Internet", systemImage: "network") }
                    .tag(AppTab.internet)

                    if paidRouteMarketAvailable {
                        NavigationStack {
                            PublicExitsPage(model: model)
                                .navigationTitle("Buy Internet")
                                .toolbar { networkSwitcherToolbar }
                        }
                        .tabItem { Label("Buy Internet", systemImage: "cart.fill") }
                        .tag(AppTab.publicExits)

                        NavigationStack {
                            PaidRouteWalletPage(model: model)
                                .navigationTitle("Wallet")
                                .toolbar { networkSwitcherToolbar }
                        }
                        .tabItem { Label("Wallet", systemImage: "creditcard.fill") }
                        .tag(AppTab.wallet)
                    }

                    NavigationStack {
                        SettingsPage(model: model)
                            .navigationTitle("Settings")
                            .toolbar { networkSwitcherToolbar }
                    }
                    .tabItem { Label("Settings", systemImage: "gearshape") }
                    .tag(AppTab.settings)
                }
            }
        }
        .tint(.purple)
        .sheet(isPresented: $addNetworkPresented) {
            NavigationStack {
                AddNetworkPage(
                    model: model,
                    onCreated: { addNetworkPresented = false },
                    onReviewVpnDisclosure: {
                        presentVpnDisclosure(startVpnAfterAccept: true)
                    }
                )
                    .navigationTitle("Add Network")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { addNetworkPresented = false }
                        }
                }
            }
        }
        .sheet(isPresented: $vpnDisclosurePresented) {
            VpnDisclosureSheet {
                let shouldStartVpn = startVpnAfterDisclosure
                vpnDisclosureAccepted = true
                startVpnAfterDisclosure = false
                vpnDisclosurePresented = false
                model.markVpnDisclosureAccepted()
                if shouldStartVpn, !model.state.vpnEnabled {
                    model.toggleVpn()
                }
            }
        }
        .onAppear {
            if model.vpnDisclosurePromptVisible && !vpnDisclosureAccepted {
                presentVpnDisclosure(startVpnAfterAccept: true)
            }
            normalizeSelectedTab()
        }
        .onChange(of: model.vpnDisclosurePromptVisible) { _, visible in
            if visible && !vpnDisclosureAccepted {
                presentVpnDisclosure(startVpnAfterAccept: true)
            }
        }
        .onChange(of: model.state.rev) { _, _ in
            if let shownNetworkId,
               !model.state.networks.contains(where: { $0.id == shownNetworkId }) {
                self.shownNetworkId = nil
            }
            normalizeSelectedTab()
        }
    }

    @ToolbarContentBuilder
    private var networkSwitcherToolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            NetworkSwitcher(
                model: model,
                shownNetwork: shownNetwork,
                shownNetworkId: $shownNetworkId,
                addNetworkPresented: $addNetworkPresented
            )
        }
        ToolbarItem(placement: .topBarTrailing) {
            ToolbarVpnSwitch(
                model: model,
                vpnDisclosureAccepted: vpnDisclosureAccepted,
                onReviewVpnDisclosure: {
                    presentVpnDisclosure(startVpnAfterAccept: true)
                }
            )
        }
    }

    private func presentVpnDisclosure(startVpnAfterAccept: Bool) {
        startVpnAfterDisclosure = startVpnAfterAccept
        vpnDisclosurePresented = true
    }

    private func normalizeSelectedTab() {
        if !paidRouteMarketAvailable && (selectedTab == .publicExits || selectedTab == .wallet) {
            selectedTab = .devices
        }
    }

    private static func initialTab() -> AppTab {
        switch AppModel.screenshotTabArgument()?.lowercased() {
        case "internet", "exit", "exit-node", "exit-nodes", "routes", "routing":
            return .internet
        case "public-exits", "paid-exits", "paid-market", "market":
            return PaidInternetFeature.enabled ? .publicExits : .devices
        case "wallet", "paid-wallet":
            return PaidInternetFeature.enabled ? .wallet : .devices
        case "settings", "diagnostics":
            return .settings
        default:
            return .devices
        }
    }
}

private enum AppTab: Hashable {
    case devices
    case internet
    case publicExits
    case wallet
    case settings
}

/// Header dropdown that shows the active network's name and lets the user
/// switch to any saved network or jump to the Add Network page. Single
/// "Add network" button when there's only one saved network and nothing
/// to switch to.
private struct NetworkSwitcher: View {
    @ObservedObject var model: AppModel
    let shownNetwork: NetworkState?
    @Binding var shownNetworkId: String?
    @Binding var addNetworkPresented: Bool

    var body: some View {
        Menu {
            ForEach(model.state.networks) { network in
                Button {
                    shownNetworkId = network.id
                } label: {
                    HStack {
                        if model.state.networks.count > 1 {
                            NetworkStatusDot(network: network)
                        }
                        Text(network.displayName)
                    }
                }
            }
            if !model.state.networks.isEmpty {
                Divider()
            }
            Button {
                addNetworkPresented = true
            } label: {
                Label("Add network", systemImage: "plus")
            }
        } label: {
            HStack(spacing: 4) {
                if let shownNetwork, model.state.networks.count > 1 {
                    NetworkStatusDot(network: shownNetwork)
                }
                Text(shownNetwork?.displayName ?? "Nostr VPN")
                    .font(.headline)
                    .lineLimit(1)
                Image(systemName: "chevron.down")
                    .font(.caption2)
            }
            .foregroundStyle(.primary)
        }
    }
}

private struct NetworkStatusDot: View {
    let network: NetworkState

    var body: some View {
        Circle()
            .fill(network.enabled ? Color.green : Color.secondary.opacity(0.55))
            .frame(width: 7, height: 7)
    }
}

private enum AddNetworkMode {
    case create
    case join
}

/// First screen on a fresh install AND the screen reachable from the
/// header switcher's "Add network" item. Same content in both contexts:
/// choose create or join first, then show only the selected path.
private struct AddNetworkPage: View {
    @ObservedObject var model: AppModel
    /// Called once the user lands on a network — used by the sheet
    /// presentation to dismiss itself so the underlying Devices tab is
    /// visible. The setup case (no active network) doesn't pass this:
    /// the root view's `if activeNetwork == nil` flips on its own.
    var onCreated: (() -> Void)? = nil
    var showsWelcomeHeader = false
    var onReviewVpnDisclosure: () -> Void = {}
    @State private var mode: AddNetworkMode?

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                if showsWelcomeHeader && mode == nil {
                    NostrVpnWelcomeHeader()
                }
                if !model.state.error.isEmpty || !model.statusMessage.isEmpty {
                    NoticeCard(
                        text: model.state.error.isEmpty ? model.statusMessage : model.state.error,
                        actionTitle: model.state.error.isEmpty && model.vpnDisclosurePromptVisible ? "Review" : nil,
                        action: onReviewVpnDisclosure
                    )
                }
                switch mode {
                case nil:
                    AddNetworkChoiceButtons(mode: $mode)
                case .create:
                    AddNetworkBackButton(mode: $mode)
                    CreateNetworkCard(model: model, onCreated: onCreated)
                case .join:
                    AddNetworkBackButton(mode: $mode)
                    JoinNetworkCard(model: model)
                    AdvertiseJoinRequestCard(model: model)
                }
            }
            .padding()
        }
        .safeAreaPadding(.bottom, 92)
        .background(AppColors.background)
    }
}

private struct AddNetworkChoiceButtons: View {
    @Binding var mode: AddNetworkMode?

    var body: some View {
        VStack(spacing: 18) {
            Button {
                mode = .create
            } label: {
                Label("Create Network", systemImage: "plus.circle.fill")
                    .font(.headline)
                    .frame(maxWidth: .infinity, minHeight: 58)
            }
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.roundedRectangle(radius: 16))
            .controlSize(.large)
            .tint(AppColors.create)

            Button {
                mode = .join
            } label: {
                Label("Join Network", systemImage: "qrcode.viewfinder")
                    .font(.headline)
                    .frame(maxWidth: .infinity, minHeight: 58)
            }
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.roundedRectangle(radius: 16))
            .controlSize(.large)
            .tint(AppColors.join)
        }
    }
}

private struct NostrVpnWelcomeHeader: View {
    var body: some View {
        VStack(spacing: 12) {
            NostrVpnAppIcon()
            Text("Nostr VPN")
                .font(.largeTitle.weight(.bold))
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
        }
        .padding(.top, 26)
        .padding(.bottom, 10)
    }
}

private struct NostrVpnAppIcon: View {
    var body: some View {
        Group {
            if let icon = UIImage.appIcon {
                Image(uiImage: icon)
                    .resizable()
            } else {
                NostrVpnIconFallback()
            }
        }
        .aspectRatio(1, contentMode: .fit)
        .frame(width: 82, height: 82)
        .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        .shadow(color: .black.opacity(0.16), radius: 12, y: 6)
        .accessibilityHidden(true)
    }
}

private struct NostrVpnIconFallback: View {
    var body: some View {
        GeometryReader { geometry in
            let side = min(geometry.size.width, geometry.size.height)
            let scale = side / 108
            ZStack {
                RoundedRectangle(cornerRadius: 22 * scale, style: .continuous)
                    .fill(Color(red: 0.07, green: 0.03, blue: 0.23))
                Path { path in
                    path.move(to: CGPoint(x: 32 * scale, y: 75 * scale))
                    path.addLine(to: CGPoint(x: 32 * scale, y: 30 * scale))
                    path.addLine(to: CGPoint(x: 76 * scale, y: 75 * scale))
                    path.addLine(to: CGPoint(x: 76 * scale, y: 30 * scale))
                }
                .stroke(
                    Color(red: 0.95, green: 0.36, blue: 0.85),
                    style: StrokeStyle(lineWidth: 6.8 * scale, lineCap: .round, lineJoin: .round)
                )
                ForEach(Array(iconNodePoints(scale: scale).enumerated()), id: \.offset) { _, point in
                    Circle()
                        .fill(Color(red: 0.43, green: 0.91, blue: 1.0))
                        .frame(width: 10 * scale, height: 10 * scale)
                        .position(point)
                }
            }
            .frame(width: side, height: side)
        }
    }

    private func iconNodePoints(scale: CGFloat) -> [CGPoint] {
        [
            CGPoint(x: 32 * scale, y: 30 * scale),
            CGPoint(x: 32 * scale, y: 75 * scale),
            CGPoint(x: 76 * scale, y: 30 * scale),
            CGPoint(x: 76 * scale, y: 75 * scale),
        ]
    }
}

private extension UIImage {
    static var appIcon: UIImage? {
        guard
            let icons = Bundle.main.infoDictionary?["CFBundleIcons"] as? [String: Any],
            let primaryIcon = icons["CFBundlePrimaryIcon"] as? [String: Any],
            let iconFiles = primaryIcon["CFBundleIconFiles"] as? [String],
            let iconName = iconFiles.last
        else {
            return nil
        }
        return UIImage(named: iconName)
    }
}

private struct AddNetworkBackButton: View {
    @Binding var mode: AddNetworkMode?

    var body: some View {
        HStack {
            Button {
                mode = nil
            } label: {
                Label("Back", systemImage: "chevron.left")
            }
            .buttonStyle(.bordered)
            Spacer()
        }
    }
}

private struct DevicesPage: View {
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

private struct ToolbarVpnSwitch: View {
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

private struct VpnDisclosureSheet: View {
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

private struct CreateNetworkCard: View {
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

private struct JoinNetworkCard: View {
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
private struct AddDeviceSheet: View {
    @ObservedObject var model: AppModel
    let network: NetworkState
    @Environment(\.dismiss) private var dismiss
    @State private var qrScannerPresented = false
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
                    },
                    submit: {
                        importJoinerValue(joinRequestInput)
                        joinRequestInput = ""
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
        .sheet(isPresented: $qrScannerPresented) {
            QRCodeScannerSheet { code in
                handleScannedJoinerCode(code)
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

private struct ScanJoinerDeviceCard: View {
    @Binding var requestInput: String
    let scanError: String
    let scan: () -> Void
    let paste: () -> Void
    let inputChanged: (String) -> Void
    let submit: () -> Void

    var body: some View {
        AppCard {
            Text("Add join request")
                .font(.headline)
            Text("Scan or paste the joining device's join request or Device ID.")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextField("Join request or Device ID", text: $requestInput)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
                .onChange(of: requestInput) { _, value in
                    inputChanged(value)
                }
            HStack(spacing: 10) {
                Button(action: paste) {
                    Label("Paste", systemImage: "doc.on.clipboard")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                Button(action: submit) {
                    Label("Import", systemImage: "arrow.down.doc")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .disabled(requestInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
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
private struct ManualPairingInfoCard: View {
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

private struct InternetPage: View {
    @ObservedObject var model: AppModel
    let network: NetworkState?

    private var directSelected: Bool {
        !model.state.wireguardExitEnabled && model.state.exitNode.isEmpty
    }

    private var wgSelected: Bool {
        model.state.wireguardExitEnabled
    }

    private var wgSubtitle: String {
        if !model.state.wireguardExitConfigured {
            return "No WireGuard config saved yet"
        }
        let endpoint = model.state.wireguardExitEndpoint
        return endpoint.isEmpty ? "Configured" : endpoint
    }

    private var exitParticipants: [ParticipantState] {
        network?.participants.filter { participant in
            participant.offersExitNode && !isSelf(participant, state: model.state)
        } ?? []
    }

    // The daemon clears the *other* side automatically when there
    // would otherwise be both a peer exit AND WG upstream enabled
    // (see `settings_patch_enforces_exit_node_mutual_exclusion` in
    // ffi.rs). Using this device's normal internet needs to clear both
    // explicitly since there's no conflict in that case for the daemon to resolve.
    private func selectDirect() {
        model.dispatch(
            NativeActions.updateSettings(["exitNode": "", "wireguardExitEnabled": false]),
            status: "Saving internet"
        )
    }

    private func selectWireGuard() {
        model.dispatch(
            NativeActions.updateSettings(["wireguardExitEnabled": true]),
            status: "Saving internet"
        )
    }

    private func selectPeer(_ npub: String) {
        model.dispatch(
            NativeActions.updateSettings(["exitNode": npub]),
            status: "Saving internet"
        )
    }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                AppCard {
                    Text("Internet")
                        .font(.headline)
                    ExitNodeRow(
                        title: "This device",
                        subtitle: "Use this device's normal internet",
                        selected: directSelected,
                        enabled: true,
                        action: selectDirect
                    )
                    ExitNodeRow(
                        title: "WireGuard upstream",
                        subtitle: wgSubtitle,
                        selected: wgSelected,
                        enabled: model.state.wireguardExitConfigured,
                        action: selectWireGuard
                    )
                    if exitParticipants.isEmpty {
                        Text("No trusted devices sharing internet")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    } else {
                        ForEach(exitParticipants) { participant in
                            ExitNodeRow(
                                title: participant.displayName,
                                subtitle: deviceSubtitle(participant, state: model.state),
                                selected: !model.state.wireguardExitEnabled
                                    && model.state.exitNode == participant.npub,
                                enabled: true,
                                action: { selectPeer(participant.npub) }
                            )
                        }
                    }
                }

                AppCard {
                    Toggle("Share internet with this network", isOn: Binding(
                        get: { model.state.advertiseExitNode },
                        set: { value in
                            model.dispatch(
                                NativeActions.updateSettings(["advertiseExitNode": value]),
                                status: "Saving internet"
                            )
                        }
                    ))
                    Toggle("Block internet if selected source disconnects", isOn: Binding(
                        get: { model.state.exitNodeLeakProtection },
                        set: { value in
                            model.dispatch(
                                NativeActions.updateSettings(["exitNodeLeakProtection": value]),
                                status: "Saving internet"
                            )
                        }
                    ))
                }
                if PaidInternetFeature.enabled {
                    PaidExitSellerStatusCard(state: model.state)
                }
                WireGuardSettingsCard(model: model)
            }
            .padding()
        }
        .safeAreaPadding(.bottom, 92)
        .background(AppColors.background)
    }
}

private struct PublicExitsPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                PaidRouteMarketCard(model: model, mode: .market)
            }
            .padding()
        }
        .safeAreaPadding(.bottom, 92)
        .background(AppColors.background)
    }
}

private struct PaidRouteWalletPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                PaidRouteMarketCard(model: model, mode: .wallet)
            }
            .padding()
        }
        .safeAreaPadding(.bottom, 92)
        .background(AppColors.background)
    }
}

private enum PaidRouteCardMode {
    case market
    case wallet
}

private struct PaidRouteMarketCard: View {
    @ObservedObject var model: AppModel
    let mode: PaidRouteCardMode
    @State private var mintUrl = ""
    @State private var token = ""
    @State private var topUpAmount = ""
    @State private var sendAmount = ""
    @State private var withdrawInvoice = ""
    @State private var filterCountry = ""
    @State private var filterNetworkClass = ""
    @State private var filterRequireIpv4 = false
    @State private var filterRequireIpv6 = false
    @State private var filterSort = "quality"

    private var market: PaidRouteMarketState {
        model.state.paidRouteMarket
    }

    var body: some View {
        AppCard {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(mode == .wallet ? "Cashu Wallet" : "Buy Internet")
                        .font(.headline)
                    Text("Wallet \(fallbackText(market.wallet.totalBalanceText, formatPaidRouteMsat(market.wallet.totalBalanceMsat)))")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if mode == .market {
                    Button {
                        model.dispatch(
                            NativeActions.discoverPaidRouteOffers(),
                            status: "Finding sellers"
                        )
                    } label: {
                        Label("Find", systemImage: "magnifyingglass")
                    }
                    .disabled(model.actionInFlight || !market.supported)
                }
            }
            if mode == .market && !market.statusText.isEmpty {
                Text(market.statusText)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            if !market.supported {
                Text(mode == .wallet ? "Cashu wallet is not supported on this platform" : "Buying internet is not supported on this platform")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            } else {
                switch mode {
                case .market:
                    marketFilterControls
                    paymentActionResult(market.lastPaymentAction)
                    Divider()
                    offerList
                    Divider()
                    sessionList
                case .wallet:
                    walletControls
                    walletMintList
                    walletActionResult(market.wallet.lastAction)
                }
            }
        }
        .onAppear {
            if mintUrl.isEmpty {
                mintUrl = market.wallet.defaultMint
            }
            filterCountry = market.filter.countryCode
            filterNetworkClass = market.filter.networkClass
            filterRequireIpv4 = market.filter.requireIpv4
            filterRequireIpv6 = market.filter.requireIpv6
            filterSort = market.filter.sort.isEmpty ? "quality" : market.filter.sort
        }
    }

    private var marketFilterControls: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                TextField("Country", text: $filterCountry)
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
                TextField("Class", text: $filterNetworkClass)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            }
            HStack {
                Button("Quality") {
                    setMarketFilterSort("quality")
                }
                .buttonStyle(.bordered)
                .disabled(model.actionInFlight || filterSort == "quality")
                Button("Price") {
                    setMarketFilterSort("price")
                }
                .buttonStyle(.bordered)
                .disabled(model.actionInFlight || filterSort == "price")
                Button("Newest") {
                    setMarketFilterSort("newest")
                }
                .buttonStyle(.bordered)
                .disabled(model.actionInFlight || filterSort == "newest")
            }
            HStack {
                Toggle("IPv4", isOn: $filterRequireIpv4)
                    .toggleStyle(.button)
                Toggle("IPv6", isOn: $filterRequireIpv6)
                    .toggleStyle(.button)
                Spacer()
                Button("Clear") {
                    filterCountry = ""
                    filterNetworkClass = ""
                    filterRequireIpv4 = false
                    filterRequireIpv6 = false
                    filterSort = "quality"
                    applyMarketFilter()
                }
                .disabled(model.actionInFlight || market.offers.isEmpty)
                Button("Apply") {
                    applyMarketFilter()
                }
                .disabled(model.actionInFlight || market.offers.isEmpty)
            }
        }
    }

    private func setMarketFilterSort(_ sort: String) {
        filterSort = sort
        applyMarketFilter(sort: sort)
    }

    private func applyMarketFilter(sort: String? = nil) {
        model.dispatch(
            NativeActions.setPaidRouteMarketFilter(
                countryCode: filterCountry.trimmingCharacters(in: .whitespacesAndNewlines),
                networkClass: filterNetworkClass.trimmingCharacters(in: .whitespacesAndNewlines),
                requireIpv4: filterRequireIpv4,
                requireIpv6: filterRequireIpv6,
                sort: sort ?? filterSort
            ),
            status: "Filtering sellers"
        )
    }

    private var walletControls: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                TextField("Mint URL", text: $mintUrl)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("Add") {
                    model.dispatch(
                        NativeActions.addPaidRouteWalletMint(
                            url: mintUrl.trimmingCharacters(in: .whitespacesAndNewlines),
                            label: nil
                        ),
                        status: "Saving wallet"
                    )
                }
                .disabled(model.actionInFlight || mintUrl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            HStack {
                TextField("Top-up sats", text: $topUpAmount)
                    .keyboardType(.numberPad)
                Button("Top Up") {
                    guard let amount = parsePositivePaidRouteAmount(topUpAmount) else { return }
                    model.dispatch(
                        NativeActions.topUpPaidRouteWallet(mintUrl: optionalPaidRouteMintUrl(mintUrl), amountSat: amount),
                        status: "Creating invoice"
                    )
                }
                .disabled(model.actionInFlight || parsePositivePaidRouteAmount(topUpAmount) == nil)
            }
            HStack {
                TextField("Send sats", text: $sendAmount)
                    .keyboardType(.numberPad)
                Button("Export") {
                    guard let amount = parsePositivePaidRouteAmount(sendAmount) else { return }
                    model.dispatch(
                        NativeActions.sendPaidRouteWalletToken(mintUrl: optionalPaidRouteMintUrl(mintUrl), amountSat: amount),
                        status: "Creating token"
                    )
                }
                .disabled(model.actionInFlight || parsePositivePaidRouteAmount(sendAmount) == nil)
            }
            HStack {
                TextField("Cashu token", text: $token)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("Import") {
                    let trimmed = token.trimmingCharacters(in: .whitespacesAndNewlines)
                    model.dispatch(
                        NativeActions.receivePaidRouteWalletToken(token: trimmed),
                        status: "Receiving token"
                    )
                    token = ""
                }
                .disabled(model.actionInFlight || token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            HStack {
                TextField("Lightning invoice", text: $withdrawInvoice)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("Withdraw") {
                    let trimmed = withdrawInvoice.trimmingCharacters(in: .whitespacesAndNewlines)
                    model.dispatch(
                        NativeActions.withdrawPaidRouteWalletLightning(mintUrl: optionalPaidRouteMintUrl(mintUrl), invoice: trimmed),
                        status: "Paying invoice"
                    )
                    withdrawInvoice = ""
                }
                .disabled(model.actionInFlight || withdrawInvoice.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            HStack {
                Button("Refresh Wallet") {
                    model.dispatch(
                        NativeActions.refreshPaidRouteWallet(),
                        status: "Refreshing wallet"
                    )
                }
            }
        }
    }

    private var walletMintList: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Mints")
                .font(.subheadline)
                .fontWeight(.semibold)
            if market.wallet.mints.isEmpty {
                Text("No wallet mints")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(market.wallet.mints) { mint in
                    HStack(alignment: .center) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(mint.label.isEmpty ? mint.url : mint.label)
                                .fontWeight(.semibold)
                                .lineLimit(1)
                            Text(fallbackText(mint.balanceText, formatPaidRouteMsat(mint.balanceMsat)))
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        if mint.url == market.wallet.defaultMint {
                            Pill("Default", tint: AppColors.accent)
                        } else {
                            Button("Default") {
                                model.dispatch(
                                    NativeActions.setPaidRouteDefaultMint(url: mint.url),
                                    status: "Saving wallet"
                                )
                            }
                            .disabled(model.actionInFlight)
                        }
                        Button(role: .destructive) {
                            model.dispatch(
                                NativeActions.removePaidRouteWalletMint(url: mint.url),
                                status: "Saving wallet"
                            )
                        } label: {
                            Image(systemName: "trash")
                        }
                        .disabled(model.actionInFlight)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func walletActionResult(_ action: PaidRouteWalletActionState) -> some View {
        if !action.kind.isEmpty || !action.statusText.isEmpty {
            Text(action.statusText.isEmpty ? paidRouteWalletActionTitle(action.kind) : action.statusText)
                .font(.footnote)
                .foregroundStyle(.secondary)
            if !action.paymentRequest.isEmpty {
                Text("Lightning invoice ready")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            if !action.token.isEmpty {
                Text("Cashu token ready")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            if !action.preimage.isEmpty {
                Text("Lightning preimage ready")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func paymentActionResult(_ action: PaidRoutePaymentActionState) -> some View {
        if !action.kind.isEmpty || !action.statusText.isEmpty || !action.envelopeJson.isEmpty {
            HStack {
                Text(action.statusText.isEmpty ? paidRoutePaymentActionTitle(action.kind) : action.statusText)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Spacer()
                if !action.envelopeJson.isEmpty {
                    Button("Send payment") {
                        model.dispatch(
                            NativeActions.sendPaidRoutePaymentEnvelope(envelopeJson: action.envelopeJson),
                            status: "Sending payment"
                        )
                    }
                    .disabled(model.actionInFlight)
                }
            }
        }
    }

    private var offerList: some View {
        let visibleOffers = (market.hiddenOfferCount > 0 || !market.visibleOffers.isEmpty)
            ? market.visibleOffers
            : market.offers
        return VStack(alignment: .leading, spacing: 8) {
            Text("Offers")
                .font(.subheadline)
                .fontWeight(.semibold)
            if market.offers.isEmpty {
                Text("No internet sellers found")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            } else if visibleOffers.isEmpty {
                Text("No matching sellers")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            } else {
                if market.hiddenOfferCount > 0 {
                    Text("\(market.hiddenOfferCount) hidden by filters")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                ForEach(visibleOffers.prefix(6)) { offer in
                    PaidRouteOfferRow(model: model, offer: offer)
                }
            }
        }
    }

    private var sessionList: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Your Paid Internet")
                .font(.subheadline)
                .fontWeight(.semibold)
            if market.sessions.isEmpty {
                Text("No seller selected")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(market.sessions) { session in
                    PaidRouteSessionRow(
                        model: model,
                        session: session,
                        envelopeJson: market.lastPaymentAction.envelopeJson
                    )
                }
            }
        }
    }
}

private struct PaidRouteOfferRow: View {
    @ObservedObject var model: AppModel
    let offer: PaidRouteOfferState

    var body: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 3) {
                Text(paidRouteOfferTitle(offer))
                    .fontWeight(.semibold)
                Text(offer.statusText.isEmpty ? offer.sellerNpub : offer.statusText)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                let metricText = paidRouteMetricText(
                    fallbackText(
                        offer.qualityText,
                        paidRouteQualityText(offer.latencyMs, offer.jitterMs, offer.packetLossPpm)
                    ),
                    offer.bandwidthText
                )
                if !metricText.isEmpty {
                    Text(metricText)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            Button("Connect") {
                model.dispatch(
                    NativeActions.buyPaidRouteOffer(offerKey: offer.key),
                    status: "Connecting"
                )
            }
            .disabled(model.actionInFlight || offer.key.isEmpty)
        }
    }
}

private struct PaidRouteSessionRow: View {
    @ObservedObject var model: AppModel
    let session: PaidRouteSessionState
    let envelopeJson: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(paidRouteBuyerSessionTitle(session))
                        .fontWeight(.semibold)
                    Text(paidRouteSessionDetail(session))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    if !session.locationText.isEmpty {
                        Text(session.locationText)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    } else if !session.realizedExitIp.isEmpty {
                        Text("\(session.realizedExitIp) · \(paidRouteCountryClaimText(session))")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    let metricText = paidRouteMetricText(
                        fallbackText(
                            session.qualityText,
                            paidRouteQualityText(session.latencyMs, session.jitterMs, session.packetLossPpm)
                        ),
                        session.bandwidthText
                    )
                    if !metricText.isEmpty {
                        Text(metricText)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    if !session.settlementText.isEmpty {
                        Text(session.settlementText)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 3) {
                    Text(fallbackText(session.paidText, "\(formatPaidRouteMsat(session.paidMsat)) paid"))
                        .font(.footnote)
                    if session.unpaidMsat > 0 {
                        Text(fallbackText(session.unpaidText, "\(formatPaidRouteMsat(session.unpaidMsat)) behind"))
                            .font(.footnote)
                            .foregroundStyle(.orange)
                    }
                }
            }
            HStack {
                Button("Connect") {
                    model.dispatch(
                        NativeActions.selectPaidRouteSession(sessionId: session.sessionId, connect: true),
                        status: "Connecting"
                    )
                }
                Button("Probe") {
                    model.dispatch(
                        NativeActions.probePaidRouteSession(sessionId: session.sessionId),
                        status: "Checking connection"
                    )
                }
            }
            HStack {
                if paidRouteSessionCanOpenChannel(session) {
                    Button("Fund") {
                        model.dispatch(
                            NativeActions.openPaidRouteChannelFromWallet(sessionId: session.sessionId),
                            status: "Funding seller"
                        )
                    }
                }
                if paidRouteSessionCanSignPayment(session) {
                    Button("Pay") {
                        model.dispatch(
                            NativeActions.signPaidRoutePaymentEnvelopeFromWallet(sessionId: session.sessionId),
                            status: "Paying seller"
                        )
                    }
                }
                if paidRouteSessionCanCloseChannel(session) {
                    Button("Settle") {
                        model.dispatch(
                            NativeActions.closePaidRouteChannelFromWallet(sessionId: session.sessionId),
                            status: "Settling channel"
                        )
                    }
                }
                if !envelopeJson.isEmpty {
                    Button("Send") {
                        model.dispatch(
                            NativeActions.sendPaidRoutePaymentEnvelope(envelopeJson: envelopeJson),
                            status: "Sending payment"
                        )
                    }
                }
            }
            .disabled(model.actionInFlight)
        }
    }
}

private struct PaidExitSellerStatusCard: View {
    let state: AppState

    var body: some View {
        let seller = state.paidExitSeller
        AppCard {
            Text("Share My Internet")
                .font(.headline)
            Text(
                paidExitSellerStatusText(seller)
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
            if seller.supported {
                Text(paidExitSellerInternetText(seller))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                if !seller.publicIpText.isEmpty {
                    Text("Public IP \(seller.publicIpText)")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Text("Spendable wallet \(fallbackText(state.paidRouteMarket.wallet.totalBalanceText, formatPaidRouteMsat(state.paidRouteMarket.wallet.totalBalanceMsat)))")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Text("\(fallbackText(seller.channelCreditTitleText, "Pending buyer credit")) \(fallbackText(seller.channelCreditText, formatPaidRouteMsat(seller.channelCreditMsat)))")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                let creditHelp = fallbackText(seller.channelCreditHelpText, seller.channelCreditMsat > 0 ? "Collect to move it into wallet" : "")
                if !creditHelp.isEmpty {
                    Text(creditHelp)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                let paymentStatus = paidRoutePaymentStatusText(state.paidRouteMarket.lastPaymentAction)
                if !paymentStatus.isEmpty {
                    Text("Payments \(paymentStatus)")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Text("\(seller.countryCode.isEmpty ? "Country unset" : seller.countryCode) · \(paidRouteNetworkClassTitle(seller.networkClass)) · \(fallbackText(seller.priceText, paidRoutePriceText(priceMsat: seller.priceMsat, perUnits: seller.perUnits, meter: seller.meter, perUnitsText: seller.perUnitsText)))")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Text("Free test \(fallbackText(seller.freeProbeText, paidRouteTrafficUnitText(seller.freeProbeUnits, meter: seller.meter))) · Grace \(fallbackText(seller.graceText, paidRouteTrafficUnitText(seller.graceUnits, meter: seller.meter)))")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                if !seller.settlementText.isEmpty {
                    Text(seller.settlementText)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                if !seller.sessions.isEmpty {
                    Text("\(seller.sessions.count) active customer\(seller.sessions.count == 1 ? "" : "s")")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

private struct ExitNodeRow: View {
    let title: String
    let subtitle: String
    let selected: Bool
    let enabled: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .center, spacing: 12) {
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundColor(selected ? AppColors.accent : .secondary)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(.body)
                        .foregroundColor(.primary)
                    if !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.footnote)
                            .foregroundColor(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
            }
            .padding(.vertical, 6)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .opacity(enabled ? 1.0 : 0.5)
    }
}

private struct SettingsPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                DeviceSettingsCard(model: model)
                GeneralSettingsCard(model: model)
                FipsSettingsCard(model: model)
                PubsubSettingsCard(model: model)
                RelaySettingsCard(model: model)
                DiagnosticsCard(state: model.state)
            }
            .padding()
        }
        .background(AppColors.background)
    }
}

private struct ParticipantRow: View {
    @ObservedObject var model: AppModel
    let network: NetworkState
    let participant: ParticipantState
    @State private var detailPresented = false

    var body: some View {
        Button {
            detailPresented = true
        } label: {
            AppCard {
                HStack(spacing: 12) {
                    Circle()
                        .fill(connectivityTint(participant, state: model.state))
                        .frame(width: 12, height: 12)
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 8) {
                            Text(deviceName(participant, state: model.state))
                                .font(.headline)
                                .lineLimit(nil)
                                .fixedSize(horizontal: false, vertical: true)
                            if participant.isAdmin {
                                Pill("Admin", tint: AppColors.accent)
                            }
                            if isSelf(participant, state: model.state) {
                                Pill("This device", tint: AppColors.ok)
                            }
                            if participant.offersExitNode {
                                Pill(
                                    exitNodeBadgeText(participant, state: model.state),
                                    tint: exitNodeBadgeTint(participant, state: model.state)
                                )
                            }
                            if isFipsRouted(participant, state: model.state) {
                                Pill("via mesh", tint: .secondary)
                            }
                        }
                        Text(deviceSubtitle(participant, state: model.state))
                            .foregroundStyle(.secondary)
                        Text(deviceStatus(participant, state: model.state))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .buttonStyle(.plain)
        .sheet(isPresented: $detailPresented) {
            NavigationStack {
                DeviceDetailSheet(model: model, network: network, participant: participant)
                    .navigationTitle(deviceName(participant, state: model.state))
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Done") { detailPresented = false }
                        }
                    }
            }
        }
    }
}

private struct DeviceDetailSheet: View {
    @ObservedObject var model: AppModel
    let network: NetworkState
    let participant: ParticipantState
    @State private var aliasDraft: String = ""
    @State private var endpointHintsDraft: String = ""
    @State private var pendingRemove = false

    private var isMe: Bool { isSelf(participant, state: model.state) }
    private var localIsAdmin: Bool { network.localIsAdmin }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                AppCard {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(connectivityTint(participant, state: model.state))
                            .frame(width: 12, height: 12)
                        Text(deviceDetailStatus(participant, state: model.state))
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                        Spacer()
                        if participant.isAdmin {
                            Pill("Admin", tint: AppColors.accent)
                        }
                        if isMe {
                            Pill("This device", tint: AppColors.ok)
                        }
                        if participant.offersExitNode {
                            Pill(
                                exitNodeBadgeText(participant, state: model.state),
                                tint: exitNodeBadgeTint(participant, state: model.state)
                            )
                        }
                    }
                }

                AppCard {
                    Text("Identity")
                        .font(.headline)
                    if !participant.magicDnsName.isEmpty {
                        labelValueRow("Magic DNS", participant.magicDnsName)
                    }
                    labelValueRow("Device ID", participant.npub, copyable: true)
                    if !participant.tunnelIp.isEmpty {
                        labelValueRow("Tunnel IP", participant.tunnelIp, copyable: true)
                    }
                    labelValueRow("FIPS path", fipsPath(participant, state: model.state))
                    if participant.fipsSrttAgeMs > 0 {
                        labelValueRow("Latency age", formatDurationMs(participant.fipsSrttAgeMs))
                    }
                    if !participant.lastFipsControlSeenText.isEmpty {
                        labelValueRow("Control seen", participant.lastFipsControlSeenText)
                    }
                    if !participant.lastFipsDataSeenText.isEmpty {
                        labelValueRow("Data seen", participant.lastFipsDataSeenText)
                    }
                    if !participant.fipsTransportAddr.isEmpty {
                        labelValueRow("Endpoint", participant.fipsTransportAddr)
                    }
                    if !participant.fipsEndpointHints.isEmpty {
                        labelValueRow("Address hints", participant.fipsEndpointHints.joined(separator: ", "))
                    }
                }

                if localIsAdmin {
                    AppCard {
                        Text("Manage")
                            .font(.headline)
                        TextField("Alias", text: $aliasDraft)
                            .textFieldStyle(.roundedBorder)
                            .onAppear { aliasDraft = participant.magicDnsAlias }
                        Button {
                            model.dispatch(
                                NativeActions.setParticipantAlias(npub: participant.npub, alias: aliasDraft),
                                status: "Saving alias"
                            )
                        } label: {
                            Label("Save alias", systemImage: "checkmark")
                        }
                        .buttonStyle(.bordered)

                        if !isMe {
                            TextField("host or host:port", text: $endpointHintsDraft)
                                .textFieldStyle(.roundedBorder)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .onAppear { endpointHintsDraft = participant.fipsEndpointHints.joined(separator: ", ") }
                            Button {
                                model.dispatch(
                                    NativeActions.setParticipantEndpointHints(
                                        npub: participant.npub,
                                        endpointHints: endpointHints(from: endpointHintsDraft)
                                    ),
                                    status: "Saving address hints"
                                )
                            } label: {
                                Label("Save hints", systemImage: "network")
                            }
                            .buttonStyle(.bordered)

                            Button {
                                if participant.isAdmin {
                                    model.dispatch(
                                        NativeActions.removeAdmin(networkId: network.id, npub: participant.npub),
                                        status: "Removing admin"
                                    )
                                } else {
                                    model.dispatch(
                                        NativeActions.addAdmin(networkId: network.id, npub: participant.npub),
                                        status: "Granting admin"
                                    )
                                }
                            } label: {
                                Label(participant.isAdmin ? "Remove admin" : "Make admin", systemImage: participant.isAdmin ? "person.fill.badge.minus" : "person.fill.badge.plus")
                            }
                            .buttonStyle(.bordered)

                            Button(role: .destructive) {
                                pendingRemove = true
                            } label: {
                                Label("Remove from network", systemImage: "trash")
                            }
                            .buttonStyle(.bordered)
                        }
                    }
                    .confirmationDialog(
                        "Remove \(deviceName(participant, state: model.state))?",
                        isPresented: $pendingRemove,
                        titleVisibility: .visible
                    ) {
                        Button("Remove", role: .destructive) {
                            model.dispatch(
                                NativeActions.removeParticipant(networkId: network.id, npub: participant.npub),
                                status: "Removing device"
                            )
                            pendingRemove = false
                        }
                        Button("Cancel", role: .cancel) { pendingRemove = false }
                    } message: {
                        Text("This removes the device from the network's roster. They keep the network locally but won't be in this roster anymore.")
                    }
                }
            }
            .padding()
        }
        .background(AppColors.background)
    }

    private func endpointHints(from value: String) -> [String] {
        value
            .components(separatedBy: CharacterSet(charactersIn: ", \n\r\t"))
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    private func labelValueRow(_ label: String, _ value: String, copyable: Bool = false) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Text(label)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .frame(width: 90, alignment: .leading)
            if label == "Device ID" {
                WrappingIdentifierText(
                    value: value,
                    font: .preferredFont(forTextStyle: .callout),
                    color: .label
                )
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text(value)
                    .font(.callout)
                    .lineLimit(nil)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if copyable {
                Button {
                    model.copy(value)
                } label: {
                    Image(systemName: "doc.on.doc")
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
            }
        }
    }
}

private struct AddDeviceCard: View {
    let network: NetworkState
    let add: (String, String) -> Void
    @State private var deviceId = ""
    @State private var alias = ""

    private var trimmedDeviceId: String {
        deviceId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var deviceIdInvalid: Bool {
        !trimmedDeviceId.isEmpty && !isValidDeviceId(trimmedDeviceId)
    }

    var body: some View {
        AppCard {
            Text("Add by Device ID")
                .font(.headline)
            Text("Add the other device directly to this signed roster.")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextField("Device ID", text: $deviceId)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Color.red, lineWidth: deviceIdInvalid ? 1 : 0)
                )
            if deviceIdInvalid {
                Text("Not a valid device ID")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
            TextField("Name", text: $alias)
                .textFieldStyle(.roundedBorder)
            Button("Add") {
                add(trimmedDeviceId, alias)
                deviceId = ""
                alias = ""
            }
            .buttonStyle(.borderedProminent)
            .disabled(trimmedDeviceId.isEmpty || deviceIdInvalid)
        }
    }
}

private struct NearbyCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        AppCard {
            HStack {
                Text("Nearby join requests")
                    .font(.headline)
                Spacer()
                Button {
                    model.dispatch(
                        model.state.nearbyDiscoveryActive ? NativeActions.stopNearbyDiscovery() : NativeActions.startNearbyDiscovery(),
                        status: "Finding nearby"
                    )
                } label: {
                    Label(
                        model.state.nearbyDiscoveryActive
                            ? "Finding nearby · \(formatRemaining(model.state.nearbyDiscoveryRemainingSecs))"
                            : "Find nearby",
                        systemImage: model.state.nearbyDiscoveryActive ? "stop.circle" : "dot.radiowaves.left.and.right"
                    )
                }
                .buttonStyle(.bordered)
            }
            if model.state.lanPeers.isEmpty {
                Text(model.state.nearbyDiscoveryActive ? "No nearby join requests yet" : "Tap above to find nearby join requests")
                    .foregroundStyle(.secondary)
                    .font(.footnote)
            } else {
                ForEach(model.state.lanPeers) { peer in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(peer.nodeName.isEmpty ? peer.networkName : peer.nodeName)
                                .font(.subheadline.weight(.semibold))
                            Text(peer.lastSeenText)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Add") {
                            model.dispatch(NativeActions.importJoinRequest(peer.invite), status: "Adding device")
                        }
                    }
                }
            }
        }
    }
}

private func formatRemaining(_ seconds: UInt64) -> String {
    if seconds == 0 { return "off" }
    let minutes = seconds / 60
    if minutes == 0 { return "\(seconds)s" }
    let secs = seconds % 60
    return secs == 0 ? "\(minutes)m" : String(format: "%dm%02ds", minutes, secs)
}

private struct AdvertiseJoinRequestCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        AppCard {
            HStack {
                Text("Nearby join request")
                    .font(.headline)
                Spacer()
                Button {
                    model.dispatch(
                        model.state.inviteBroadcastActive ? NativeActions.stopJoinRequestBroadcast() : NativeActions.startJoinRequestBroadcast(),
                        status: model.state.inviteBroadcastActive ? "Stopping nearby" : "Advertising nearby"
                    )
                } label: {
                    Label(
                        model.state.inviteBroadcastActive
                            ? "Advertising · \(formatRemaining(model.state.inviteBroadcastRemainingSecs))"
                            : "Advertise nearby",
                        systemImage: model.state.inviteBroadcastActive ? "stop.circle" : "dot.radiowaves.left.and.right"
                    )
                }
                .buttonStyle(.bordered)
            }
            Text(model.state.inviteBroadcastActive ? "Admins nearby can add this device from its join request." : "Advertise this device's join request to nearby admins.")
                .foregroundStyle(.secondary)
                .font(.footnote)
        }
    }
}

private struct DeviceSettingsCard: View {
    @ObservedObject var model: AppModel
    @State private var nodeName = ""
    @State private var tunnelIp = ""
    @State private var endpoint = ""
    @State private var port = ""

    var body: some View {
        AppCard {
            Text("This Device")
                .font(.headline)
            VStack(alignment: .leading, spacing: 4) {
                Text("Device ID")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                CopyLine(value: model.state.ownNpub, model: model)
            }
            TextField("Name", text: $nodeName)
                .textFieldStyle(.roundedBorder)
            TextField("Tunnel IP", text: $tunnelIp)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
            TextField("Endpoint", text: $endpoint)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
            TextField("Listen Port", text: $port)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)
            Button("Save") {
                var patch: [String: Any] = [
                    "nodeName": nodeName,
                    "tunnelIp": tunnelIp,
                    "endpoint": endpoint,
                ]
                if let listenPort = Int(port) {
                    patch["listenPort"] = listenPort
                }
                model.dispatch(NativeActions.updateSettings(patch), status: "Saving")
            }
            .buttonStyle(.borderedProminent)
        }
        .onAppear {
            nodeName = model.state.nodeName
            tunnelIp = model.state.tunnelIp
            endpoint = model.state.endpoint
            port = String(model.state.listenPort)
        }
        .onChange(of: model.state.rev) { _, _ in
            nodeName = model.state.nodeName
            tunnelIp = model.state.tunnelIp
            endpoint = model.state.endpoint
            port = String(model.state.listenPort)
        }
    }
}

private struct GeneralSettingsCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        AppCard {
            Text("General")
                .font(.headline)
            Toggle("Start VPN automatically", isOn: Binding(
                get: { model.state.autoconnect },
                set: { value in
                    model.dispatch(NativeActions.updateSettings(["autoconnect": value]), status: "Saving")
                }
            ))
        }
    }
}

private struct FipsSettingsCard: View {
    @ObservedObject var model: AppModel

    var body: some View {
        AppCard {
            Text("FIPS")
                .font(.headline)
            Toggle("Connect to non-roster FIPS peers", isOn: Binding(
                get: { model.state.connectToNonRosterFipsPeers },
                set: { value in
                    model.dispatch(NativeActions.updateSettings(["connectToNonRosterFipsPeers": value]), status: "Saving")
                }
            ))
            Toggle("Find peers over Nostr relays", isOn: Binding(
                get: { model.state.fipsNostrDiscoveryEnabled },
                set: { value in
                    model.dispatch(NativeActions.updateSettings(["fipsNostrDiscoveryEnabled": value]), status: "Saving")
                }
            ))
            Toggle("Use bootstrap servers", isOn: Binding(
                get: { model.state.fipsBootstrapEnabled },
                set: { value in
                    model.dispatch(NativeActions.updateSettings(["fipsBootstrapEnabled": value]), status: "Saving")
                }
            ))
        }
    }
}

private struct PubsubSettingsCard: View {
    @ObservedObject var model: AppModel
    @State private var mode = "relay"
    @State private var fanout = ""
    @State private var maxHops = ""
    @State private var maxEventBytes = ""

    var body: some View {
        AppCard {
            Text("Nostr Pubsub")
                .font(.headline)
            Picker("Mode", selection: $mode) {
                Text("Off").tag("off")
                Text("Client").tag("client")
                Text("Relay").tag("relay")
            }
            .pickerStyle(.segmented)
            TextField("Fanout", text: $fanout)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)
            TextField("Hops", text: $maxHops)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)
            TextField("Max event bytes", text: $maxEventBytes)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)
            Button("Save") {
                var patch: [String: Any] = ["nostrPubsubMode": mode]
                if let fanout = Int(fanout) {
                    patch["nostrPubsubFanout"] = fanout
                }
                if let maxHops = Int(maxHops) {
                    patch["nostrPubsubMaxHops"] = maxHops
                }
                if let maxEventBytes = Int(maxEventBytes) {
                    patch["nostrPubsubMaxEventBytes"] = maxEventBytes
                }
                model.dispatch(NativeActions.updateSettings(patch), status: "Saving")
            }
            .buttonStyle(.borderedProminent)
        }
        .onAppear { sync() }
        .onChange(of: model.state.rev) { _, _ in sync() }
    }

    private func sync() {
        mode = model.state.nostrPubsubMode
        fanout = String(model.state.nostrPubsubFanout)
        maxHops = String(model.state.nostrPubsubMaxHops)
        maxEventBytes = String(model.state.nostrPubsubMaxEventBytes)
    }
}

private struct RelaySettingsCard: View {
    @ObservedObject var model: AppModel
    @State private var relayInput = ""

    var body: some View {
        AppCard {
            Text("Relays")
                .font(.headline)
            HStack(spacing: 8) {
                TextField("wss://relay.example.com", text: $relayInput)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .textFieldStyle(.roundedBorder)
                    .submitLabel(.done)
                    .onSubmit { addRelayFromInput() }
                Button {
                    addRelayFromInput()
                } label: {
                    Image(systemName: "plus")
                }
                .buttonStyle(.borderedProminent)
                .disabled(relayInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            VStack(alignment: .leading, spacing: 8) {
                ForEach(model.state.relays) { relay in
                    HStack(spacing: 8) {
                        Circle()
                            .fill(relay.connected ? AppColors.ok : Color.secondary.opacity(0.65))
                            .frame(width: 10, height: 10)
                        Text(relay.url)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .foregroundStyle(relay.enabled ? .primary : .secondary)
                        Spacer(minLength: 8)
                        Toggle("", isOn: Binding(
                            get: { relay.enabled },
                            set: { setRelay(relay.url, enabled: $0) }
                        ))
                        .labelsHidden()
                        Button(role: .destructive) {
                            deleteRelay(relay.url)
                        } label: {
                            Image(systemName: "trash")
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
        }
    }

    private func addRelayFromInput() {
        guard let url = normalizedRelayUrl(relayInput) else {
            return
        }
        var lists = relayLists()
        lists.disabled.removeAll { $0 == url }
        if !lists.enabled.contains(url) {
            lists.enabled.append(url)
        }
        saveRelayLists(lists)
        relayInput = ""
    }

    private func setRelay(_ url: String, enabled: Bool) {
        guard let url = normalizedRelayUrl(url) else {
            return
        }
        var lists = relayLists()
        lists.enabled.removeAll { $0 == url }
        lists.disabled.removeAll { $0 == url }
        if enabled {
            lists.enabled.append(url)
        } else {
            lists.disabled.append(url)
        }
        saveRelayLists(lists)
    }

    private func deleteRelay(_ url: String) {
        guard let url = normalizedRelayUrl(url) else {
            return
        }
        var lists = relayLists()
        lists.enabled.removeAll { $0 == url }
        lists.disabled.removeAll { $0 == url }
        saveRelayLists(lists)
    }

    private func relayLists() -> (enabled: [String], disabled: [String]) {
        let enabled = uniqueRelayList(model.state.relays.filter(\.enabled).compactMap { normalizedRelayUrl($0.url) })
        let disabled = uniqueRelayList(model.state.relays.filter { !$0.enabled }.compactMap { normalizedRelayUrl($0.url) })
            .filter { !enabled.contains($0) }
        return (enabled, disabled)
    }

    private func saveRelayLists(_ lists: (enabled: [String], disabled: [String])) {
        let enabled = uniqueRelayList(lists.enabled)
        let disabled = uniqueRelayList(lists.disabled).filter { !enabled.contains($0) }
        model.dispatch(
            NativeActions.updateSettings(["relays": enabled, "disabledRelays": disabled]),
            status: "Saving"
        )
    }

    private func normalizedRelayUrl(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return nil
        }
        if trimmed.hasPrefix("ws://") || trimmed.hasPrefix("wss://") {
            return trimmed
        }
        return "wss://\(trimmed)"
    }

    private func uniqueRelayList(_ values: [String]) -> [String] {
        var seen = Set<String>()
        return values.filter { seen.insert($0).inserted }
    }
}

private struct WireGuardSettingsCard: View {
    @ObservedObject var model: AppModel
    @State private var config = ""
    @State private var lastSyncedConfig: String?
    @State private var importingConfig = false

    var body: some View {
        AppCard {
            Text("WireGuard Upstream")
                .font(.headline)
            Text("Paste a WireGuard config from an upstream VPN provider such as Mullvad or Proton VPN.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            Toggle("Enabled", isOn: Binding(
                get: { model.state.wireguardExitEnabled },
                set: { value in
                    model.dispatch(NativeActions.updateSettings(["wireguardExitEnabled": value]), status: "Saving")
                }
            ))
            TextEditor(text: $config)
                .font(.system(.body, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .frame(minHeight: 180)
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Color.secondary.opacity(0.25))
                )
            HStack {
                Button("Import File") {
                    importingConfig = true
                }
                .buttonStyle(.bordered)
                .disabled(model.actionInFlight)

                Button("Save") {
                    model.dispatch(NativeActions.updateSettings(["wireguardExitConfig": config]), status: "Saving")
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.actionInFlight)
            }
        }
        .fileImporter(
            isPresented: $importingConfig,
            allowedContentTypes: [.plainText, .data, .item],
            allowsMultipleSelection: false,
            onCompletion: importConfigFile
        )
        .onAppear {
            syncSavedConfigIfNeeded(force: true)
        }
        .onChange(of: model.state.rev) { _, _ in
            syncSavedConfigIfNeeded()
        }
    }

    private func syncSavedConfigIfNeeded(force: Bool = false) {
        let savedConfig = model.state.wireguardExitConfig
        guard force || savedConfig != lastSyncedConfig else {
            return
        }
        config = savedConfig
        lastSyncedConfig = savedConfig
    }

    private func importConfigFile(_ result: Result<[URL], Error>) {
        do {
            guard let url = try result.get().first else {
                return
            }
            let didStartAccess = url.startAccessingSecurityScopedResource()
            defer {
                if didStartAccess {
                    url.stopAccessingSecurityScopedResource()
                }
            }
            let imported = try String(contentsOf: url, encoding: .utf8)
            guard !imported.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                model.statusMessage = "Selected WireGuard config is empty."
                return
            }
            config = imported
            model.dispatch(
                NativeActions.updateSettings(["wireguardExitConfig": imported]),
                status: "Importing"
            )
        } catch {
            model.statusMessage = error.localizedDescription
        }
    }
}

private struct DiagnosticsCard: View {
    let state: AppState

    var body: some View {
        AppCard {
            Text("Diagnostics")
                .font(.headline)
            Metric("Runtime", state.runtimeStatusDetail.isEmpty ? state.platform : state.runtimeStatusDetail)
            Metric("Peers", "\(state.connectedPeerCount)/\(state.expectedPeerCount)")
            Metric("Roster FIPS", "\(state.fipsConnectedPeerCount)/\(state.fipsRosterPeerCount) direct")
            Metric("Other FIPS", "\(state.nonFipsRosterPeerCount)")
            Metric("MagicDNS", state.magicDnsStatus)
            Metric("Version", state.appVersion)
            Metric("Config", state.configPath)
            ForEach(state.health) { issue in
                VStack(alignment: .leading, spacing: 3) {
                    Text(issue.severity)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.orange)
                    Text(issue.summary)
                    if !issue.detail.isEmpty {
                        Text(issue.detail)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }
}

private struct AppCard<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

private struct SetupCard<Content: View>: View {
    let title: String
    let systemImage: String
    let tint: Color
    let content: Content

    init(title: String, systemImage: String, tint: Color, @ViewBuilder content: () -> Content) {
        self.title = title
        self.systemImage = systemImage
        self.tint = tint
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(title, systemImage: systemImage)
                .font(.headline)
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(tint)
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(alignment: .leading) {
            RoundedRectangle(cornerRadius: 2, style: .continuous)
                .fill(tint)
                .frame(width: 4)
                .padding(.vertical, 14)
        }
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(tint.opacity(0.24), lineWidth: 1)
        }
        .tint(tint)
    }
}

private struct NoticeCard: View {
    let text: String
    var actionTitle: String? = nil
    var action: () -> Void = {}

    var body: some View {
        AppCard {
            Text(text)
                .foregroundStyle(.brown)
            if let actionTitle {
                Button(actionTitle, action: action)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.regular)
            }
        }
    }
}

private struct WrappingIdentifierText: UIViewRepresentable {
    let value: String
    let font: UIFont
    let color: UIColor

    func makeUIView(context: Context) -> UITextView {
        let textView = UITextView()
        textView.backgroundColor = .clear
        textView.isEditable = false
        textView.isSelectable = true
        textView.isScrollEnabled = false
        textView.textContainerInset = .zero
        textView.textContainer.lineFragmentPadding = 0
        textView.textContainer.lineBreakMode = .byCharWrapping
        textView.adjustsFontForContentSizeCategory = true
        textView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return textView
    }

    func updateUIView(_ textView: UITextView, context: Context) {
        textView.attributedText = attributedText
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        guard let width = proposal.width else {
            return nil
        }
        let fittingSize = uiView.sizeThatFits(
            CGSize(width: width, height: CGFloat.greatestFiniteMagnitude)
        )
        return CGSize(width: width, height: fittingSize.height)
    }

    private var attributedText: NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.hyphenationFactor = 0
        paragraph.lineBreakMode = .byCharWrapping
        return NSAttributedString(
            string: value.isEmpty ? "-" : value,
            attributes: [
                .font: font,
                .foregroundColor: color,
                .paragraphStyle: paragraph,
            ]
        )
    }
}

private struct CopyLine: View {
    let value: String
    var displayValue: String? = nil
    @ObservedObject var model: AppModel

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            if value.hasPrefix("npub1") {
                WrappingIdentifierText(
                    value: (displayValue ?? value).isEmpty ? "-" : (displayValue ?? value),
                    font: .preferredFont(forTextStyle: .footnote),
                    color: .secondaryLabel
                )
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text((displayValue ?? value).isEmpty ? "-" : (displayValue ?? value))
                    .font(.footnote)
                    .textSelection(.enabled)
                    .foregroundStyle(.secondary)
                    .lineLimit(nil)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            Button {
                model.copy(value)
            } label: {
                Label("Copy", systemImage: model.copiedValue == value ? "checkmark" : "doc.on.doc")
            }
            .disabled(value.isEmpty)
        }
    }
}

private func displayNetworkId(_ value: String) -> String {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.count > 4, isHexString(trimmed) else {
        return trimmed
    }
    return stride(from: 0, to: trimmed.count, by: 4)
        .map { offset -> String in
            let start = trimmed.index(trimmed.startIndex, offsetBy: offset)
            let end = trimmed.index(start, offsetBy: min(4, trimmed.distance(from: start, to: trimmed.endIndex)))
            return String(trimmed[start..<end])
        }
        .joined(separator: "-")
}

private func normalizeNetworkIdInput(_ value: String) -> String {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    let compactScalars = trimmed.unicodeScalars.filter {
        !$0.properties.isWhitespace && $0 != "-"
    }
    let compact = String(String.UnicodeScalarView(compactScalars))
    if compact.isEmpty && trimmed.unicodeScalars.allSatisfy({ $0.properties.isWhitespace || $0 == "-" }) {
        return ""
    }
    return !compact.isEmpty && isHexString(compact) ? compact.lowercased() : trimmed
}

private func isHexString(_ value: String) -> Bool {
    !value.isEmpty && value.unicodeScalars.allSatisfy { scalar in
        (48...57).contains(Int(scalar.value))
            || (65...70).contains(Int(scalar.value))
            || (97...102).contains(Int(scalar.value))
    }
}

private struct ScannedDeviceLink {
    let deviceId: String
    let alias: String?
}

private struct PendingJoinRequest: Identifiable {
    let id = UUID()
    let networkName: String
    let request: String
}

private func looksLikeJoinRequestQrOrLink(_ value: String) -> Bool {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.lowercased().hasPrefix("nvpn://join-request?")
}

private func parseScannedDeviceLinkQr(_ value: String) -> ScannedDeviceLink? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    if let deviceId = normalizedDeviceIdCandidate(trimmed) {
        return ScannedDeviceLink(deviceId: deviceId, alias: nil)
    }
    if let parsed = parseScannedDeviceJson(trimmed) {
        return parsed
    }
    return parseScannedDeviceUrl(trimmed)
}

private func parseScannedDeviceJson(_ value: String) -> ScannedDeviceLink? {
    guard value.hasPrefix("{"),
          let data = value.data(using: .utf8),
          let object = try? JSONSerialization.jsonObject(with: data),
          let json = object as? [String: Any]
    else {
        return nil
    }
    guard let deviceId = firstValidDeviceId(
        jsonString(json["deviceId"]),
        jsonString(json["device"]),
        jsonString(json["npub"]),
        jsonString(json["requesterNpub"])
    ) else {
        return nil
    }
    return ScannedDeviceLink(
        deviceId: deviceId,
        alias: firstNonBlank(
            jsonString(json["name"]),
            jsonString(json["nodeName"]),
            jsonString(json["label"])
        )
    )
}

private func parseScannedDeviceUrl(_ value: String) -> ScannedDeviceLink? {
    guard let components = URLComponents(string: value),
          components.scheme?.lowercased() == "nvpn"
    else {
        return nil
    }
    var query: [String: String] = [:]
    for item in components.queryItems ?? [] where query[item.name] == nil {
        query[item.name] = item.value ?? ""
    }
    let pathCandidate = components.path
        .split(separator: "/")
        .last
        .map(String.init)
    guard let deviceId = firstValidDeviceId(
        query["deviceId"],
        query["device"],
        query["npub"],
        query["requesterNpub"],
        components.host,
        pathCandidate
    ) else {
        return nil
    }
    return ScannedDeviceLink(
        deviceId: deviceId,
        alias: firstNonBlank(query["name"], query["nodeName"], query["label"])
    )
}

private func jsonString(_ value: Any?) -> String? {
    value as? String
}

private func firstValidDeviceId(_ values: String?...) -> String? {
    values.compactMap { normalizedDeviceIdCandidate($0 ?? "") }.first
}

private func normalizedDeviceIdCandidate(_ value: String) -> String? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else {
        return nil
    }
    let withoutNostrPrefix: String
    if trimmed.lowercased().hasPrefix("nostr:") {
        withoutNostrPrefix = String(trimmed.dropFirst(6)).trimmingCharacters(in: .whitespacesAndNewlines)
    } else {
        withoutNostrPrefix = trimmed
    }
    return isValidDeviceId(withoutNostrPrefix) ? withoutNostrPrefix : nil
}

private func firstNonBlank(_ values: String?...) -> String? {
    values
        .map { ($0 ?? "").trimmingCharacters(in: .whitespacesAndNewlines) }
        .first { !$0.isEmpty }
}

private struct Metric: View {
    let label: String
    let value: String

    init(_ label: String, _ value: String) {
        self.label = label
        self.value = value
    }

    var body: some View {
        HStack(alignment: .top) {
            Text(label)
                .foregroundStyle(.secondary)
                .frame(width: 80, alignment: .leading)
            Text(value.isEmpty ? "-" : value)
                .lineLimit(2)
                .truncationMode(.middle)
        }
        .font(.footnote)
    }
}

private struct Pill: View {
    let text: String
    let tint: Color

    init(_ text: String, tint: Color) {
        self.text = text
        self.tint = tint
    }

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(tint.opacity(0.12))
            .clipShape(Capsule())
    }
}

private struct QrCodeView: View {
    let matrix: QrMatrix

    var body: some View {
        Canvas { context, size in
            let side = min(size.width, size.height)
            let origin = CGPoint(
                x: (size.width - side) / 2,
                y: (size.height - side) / 2
            )
            context.fill(
                Path(CGRect(origin: origin, size: CGSize(width: side, height: side))),
                with: .color(.white)
            )
            guard matrix.width > 0, matrix.cells.count == matrix.width * matrix.width else {
                return
            }
            let quiet = 3
            let modules = matrix.width + quiet * 2
            let cell = side / CGFloat(modules)
            for y in 0..<matrix.width {
                for x in 0..<matrix.width where matrix.cells[y * matrix.width + x] {
                    let rect = CGRect(
                        x: origin.x + CGFloat(x + quiet) * cell,
                        y: origin.y + CGFloat(y + quiet) * cell,
                        width: cell,
                        height: cell
                    )
                    context.fill(Path(rect), with: .color(.black))
                }
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

private enum AppColors {
    static let background = Color(uiColor: .systemGroupedBackground)
    static let accent = Color.purple
    static let create = Color.green
    static let join = Color.blue
    static let ok = Color.green
}

private func paidRouteOfferTitle(_ offer: PaidRouteOfferState) -> String {
    let location = offer.countryCode.isEmpty ? "Unknown country" : offer.countryCode.uppercased()
    let network = paidRouteNetworkClassTitle(offer.networkClass)
    let price = offer.priceText.isEmpty
        ? paidRoutePriceText(priceMsat: offer.priceMsat, perUnits: offer.perUnits, meter: offer.meter, perUnitsText: offer.perUnitsText)
        : offer.priceText
    return "\(location) · \(network) · \(price)"
}

private func paidRouteSessionDetail(_ session: PaidRouteSessionState) -> String {
    if !session.detailText.isEmpty {
        return session.detailText
    }
    let access = paidRouteAccessTitle(
        session.accessState,
        fallback: session.lifecycleStatus.isEmpty ? "session" : session.lifecycleStatus
    )
    let units: String
    if session.bytes > 0 {
        units = "\(formatBytes(session.bytes)) used"
    } else if session.packets > 0 {
        units = "\(session.packets) packets"
    } else {
        units = "\(session.deliveredUnits) units"
    }
    return "\(access), \(units), \(formatPaidRouteMsat(session.amountDueMsat)) due"
}

private func paidRouteBuyerSessionTitle(_ session: PaidRouteSessionState) -> String {
    if !session.titleText.isEmpty {
        return session.titleText
    }
    if session.allowRouting {
        return "Ready"
    }
    if session.unpaidMsat > 0 {
        return "Payment needed"
    }
    if !session.paymentChannelReady {
        return "Needs funds"
    }
    return paidRoutePlainStatus(
        session.statusText.isEmpty ? session.lifecycleStatus : session.statusText,
        fallback: "Session"
    )
}

private func paidRouteAccessTitle(_ value: String, fallback: String) -> String {
    switch value {
    case "paid": return "Paid"
    case "free_probe": return "Free test"
    case "grace": return "Grace"
    case "suspended": return "Paused"
    default:
        return paidRoutePlainStatus(value, fallback: fallback)
    }
}

private func paidRoutePlainStatus(_ value: String, fallback: String) -> String {
    let raw = value.isEmpty ? fallback : value
    switch raw {
    case "opening": return "Opening"
    case "probing": return "Checking quality"
    case "active": return "Active"
    case "paused": return "Paused"
    case "closed": return "Closed"
    case "session": return "Session"
    default:
        return raw.replacingOccurrences(of: "_", with: " ").capitalized
    }
}

private func paidRoutePaymentActionTitle(_ kind: String) -> String {
    switch kind {
    case "send": return "Payment sent"
    case "receive": return "Payment received"
    case "apply": return "Payment applied"
    case "create": return "Payment ready"
    case "open_channel": return "Exit funded"
    case "sign": return "Payment ready"
    case "close": return "Channel settled"
    case "stream": return "Payments sent"
    case "probe": return "Quality checked"
    default:
        return kind.isEmpty ? "Payment" : kind.replacingOccurrences(of: "_", with: " ").capitalized
    }
}

private func paidRoutePaymentStatusText(_ action: PaidRoutePaymentActionState) -> String {
    if action.kind.isEmpty && action.statusText.isEmpty {
        return ""
    }
    return action.statusText.isEmpty ? paidRoutePaymentActionTitle(action.kind) : action.statusText
}

private func paidRouteWalletActionTitle(_ kind: String) -> String {
    switch kind {
    case "topup": return "Invoice ready"
    case "receive": return "Token imported"
    case "send": return "Token ready"
    case "withdraw": return "Invoice paid"
    case "refresh": return "Wallet refreshed"
    case "open_channel": return "Exit funded"
    default:
        return kind.isEmpty ? "Wallet updated" : kind.replacingOccurrences(of: "_", with: " ").capitalized
    }
}

private func paidRouteNetworkClassTitle(_ value: String) -> String {
    switch value {
    case "datacenter": return "Datacenter"
    case "residential": return "Residential"
    case "mobile": return "Mobile"
    case "satellite": return "Satellite"
    case "community_mesh": return "Community mesh"
    case "unknown", "": return "Unknown"
    default:
        return value.replacingOccurrences(of: "_", with: " ").capitalized
    }
}

private func paidRouteCountryClaimText(_ session: PaidRouteSessionState) -> String {
    switch session.countryClaimStatus {
    case "match":
        let observed = session.observedCountryCode.isEmpty ? session.claimedCountryCode : session.observedCountryCode
        return "\(observed) matches claim"
    case "mismatch":
        let observed = session.observedCountryCode.isEmpty ? "Observed country" : session.observedCountryCode
        return "\(observed) differs from \(session.claimedCountryCode)"
    default:
        if !session.observedCountryCode.isEmpty {
            return session.observedCountryCode
        }
        return session.claimedCountryCode.isEmpty ? "country unknown" : session.claimedCountryCode
    }
}

private func paidRouteQualityText(_ latencyMs: UInt32, _ jitterMs: UInt32, _ packetLossPpm: UInt32) -> String {
    if latencyMs == 0, jitterMs == 0, packetLossPpm == 0 {
        return "Quality unmeasured"
    }
    let loss = Double(packetLossPpm) / 10_000.0
    return String(format: "%u ms · %u ms jitter · %.2f%% loss", latencyMs, jitterMs, loss)
}

private func paidRouteMetricText(_ qualityText: String, _ bandwidthText: String) -> String {
    [qualityText, bandwidthText]
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty && $0 != "Quality unmeasured" }
        .joined(separator: " · ")
}

private func paidExitSellerStatusText(_ seller: PaidExitSellerState) -> String {
    if seller.statusText.isEmpty {
        return seller.supported
            ? "People can pay to use my internet"
            : "This platform cannot sell public internet access"
    }
    return seller.statusText
        .replacingOccurrences(of: "Paid exit selling", with: "Selling internet")
        .replacingOccurrences(of: "paid exit selling", with: "selling internet")
}

private func paidExitSellerInternetText(_ seller: PaidExitSellerState) -> String {
    if !seller.internetText.isEmpty {
        return seller.internetText
    }
    switch seller.upstream {
    case "wireguard_exit", "wireguard", "wg", "upstream_vpn", "vpn":
        return "My internet through WireGuard"
    default:
        return "My internet"
    }
}

private func paidRouteSessionCanOpenChannel(_ session: PaidRouteSessionState) -> Bool {
    !session.sessionId.isEmpty && !session.paymentChannelReady
}

private func paidRouteSessionCanSignPayment(_ session: PaidRouteSessionState) -> Bool {
    !session.sessionId.isEmpty && session.paymentChannelReady && session.unpaidMsat > 0
}

private func paidRouteSessionCanCloseChannel(_ session: PaidRouteSessionState) -> Bool {
    !session.sessionId.isEmpty
        && session.paymentChannelReady
        && ["closed", "expired"].contains(session.lifecycleStatus) == false
}

private func parsePositivePaidRouteAmount(_ value: String) -> UInt64? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard let amount = UInt64(trimmed), amount > 0 else { return nil }
    return amount
}

private func optionalPaidRouteMintUrl(_ value: String) -> String? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? nil : trimmed
}

private func formatPaidRouteMsat(_ msat: UInt64) -> String {
    if msat == 0 {
        return "0 sat"
    }
    let whole = msat / 1_000
    let remainder = msat % 1_000
    if remainder == 0 {
        return "\(whole) sat"
    }
    return String(format: "%llu.%03llu sat", whole, remainder)
}

private func fallbackText(_ value: String, _ fallback: String) -> String {
    value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? fallback : value
}

private func formatBytes(_ bytes: UInt64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"]
    var value = Double(bytes)
    var index = 0
    while value >= 1_024, index < units.count - 1 {
        value /= 1_024
        index += 1
    }
    if index == 0 {
        return "\(bytes) B"
    }
    if value.rounded() == value {
        return String(format: "%.0f %@", value, units[index])
    }
    return String(format: "%.1f %@", value, units[index])
}

private func paidRoutePriceText(priceMsat: UInt64, perUnits: UInt64, meter: String, perUnitsText: String = "") -> String {
    "\(formatPaidRouteMsat(priceMsat)) / \(fallbackText(perUnitsText, paidRouteMeterUnitText(perUnits, meter: meter)))"
}

private func paidRouteMeterUnitText(_ units: UInt64, meter: String) -> String {
    switch meter {
    case "bytes":
        return formatDecimalBytes(units)
    case "milliseconds", "millisecond", "ms":
        return "\(units) ms"
    case "packets", "packet":
        return units == 1 ? "1 packet" : "\(units) packets"
    case "":
        return "\(units) units"
    default:
        return "\(units) \(meter)"
    }
}

private func paidRouteTrafficUnitText(_ units: UInt64, meter: String) -> String {
    meter == "bytes" ? formatBytes(units) : paidRouteMeterUnitText(units, meter: meter)
}

private func formatDecimalBytes(_ bytes: UInt64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"]
    var value = Double(bytes)
    var index = 0
    while value >= 1_000, index < units.count - 1 {
        value /= 1_000
        index += 1
    }
    if index == 0 {
        return "\(bytes) B"
    }
    if value.rounded() == value {
        return String(format: "%.0f %@", value, units[index])
    }
    return String(format: "%.1f %@", value, units[index])
}

private func cleanIp(_ value: String) -> String {
    value.split(separator: "/").first.map(String.init) ?? value
}

private func sortedParticipants(_ participants: [ParticipantState], state: AppState) -> [ParticipantState] {
    participants.sorted { lhs, rhs in
        let lhsSelf = isSelf(lhs, state: state)
        let rhsSelf = isSelf(rhs, state: state)
        if lhsSelf != rhsSelf {
            return lhsSelf
        }
        if lhs.reachable != rhs.reachable {
            return lhs.reachable && !rhs.reachable
        }
        return deviceName(lhs, state: state).localizedCaseInsensitiveCompare(deviceName(rhs, state: state)) == .orderedAscending
    }
}

private func isSelf(_ participant: ParticipantState, state: AppState) -> Bool {
    (!state.ownNpub.isEmpty && participant.npub == state.ownNpub) || participant.meshState == "local"
}

private func isActiveExitParticipant(_ participant: ParticipantState, state: AppState) -> Bool {
    state.exitNodeActive && !state.exitNode.isEmpty && participant.npub == state.exitNode
}

private func exitNodeBadgeText(_ participant: ParticipantState, state: AppState) -> String {
    isActiveExitParticipant(participant, state: state) ? "Exit active" : "Exit offered"
}

private func exitNodeBadgeTint(_ participant: ParticipantState, state: AppState) -> Color {
    isActiveExitParticipant(participant, state: state) ? AppColors.ok : .orange
}

private func deviceName(_ participant: ParticipantState, state: AppState) -> String {
    if !participant.magicDnsName.isEmpty {
        return participant.magicDnsName
    }
    if isSelf(participant, state: state), !state.selfMagicDnsName.isEmpty {
        return state.selfMagicDnsName
    }
    if !participant.alias.isEmpty {
        return participant.alias
    }
    if !participant.magicDnsAlias.isEmpty {
        return participant.magicDnsAlias
    }
    return "Device"
}

private func deviceSubtitle(_ participant: ParticipantState, state: AppState) -> String {
    let ip = cleanIp(participant.tunnelIp)
    if isSelf(participant, state: state) {
        return ip.isEmpty ? "This device" : "This device - \(ip)"
    }
    return ip
}

private func deviceStatus(_ participant: ParticipantState, state: AppState) -> String {
    if isSelf(participant, state: state) {
        return state.vpnEnabled ? "This device" : "Off"
    }
    switch participant.state {
    case "local", "online", "present":
        return "Online"
    case "pending":
        return "Connecting"
    case "offline", "absent", "off":
        return "Offline"
    default:
        return participant.reachable ? "Online" : "Unknown"
    }
}

private func deviceDetailStatus(_ participant: ParticipantState, state: AppState) -> String {
    if isSelf(participant, state: state) {
        return deviceStatus(participant, state: state)
    }
    if !participant.statusText.isEmpty {
        return participant.statusText
    }
    return deviceStatus(participant, state: state)
}

private func fipsPath(_ participant: ParticipantState, state: AppState) -> String {
    if isSelf(participant, state: state) {
        return "This device"
    }
    if participant.reachable
        && !participant.fipsTransportAddr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    {
        let transport = participant.fipsTransportType.isEmpty ? "" : " (\(participant.fipsTransportType.uppercased()))"
        if participant.fipsSrttMs > 0 {
            return "Direct connection\(transport), \(participant.fipsSrttMs) ms"
        }
        return "Direct connection\(transport)"
    }
    if participant.reachable {
        if participant.fipsSrttMs > 0 {
            return "Via mesh, \(participant.fipsSrttMs) ms"
        }
        return "Via mesh"
    }
    if participant.state == "pending" {
        return "Connecting"
    }
    return "Offline"
}

private func formatDurationMs(_ ms: UInt64) -> String {
    if ms == 0 { return "-" }
    if ms < 1_000 { return "\(ms) ms" }
    let seconds = ms / 1_000
    if seconds < 60 { return "\(seconds)s" }
    let minutes = seconds / 60
    if minutes < 60 { return "\(minutes)m" }
    return "\(minutes / 60)h"
}

private func connectivityTint(_ participant: ParticipantState, state: AppState) -> Color {
    if isSelf(participant, state: state) {
        return state.vpnActive ? AppColors.ok : Color.gray.opacity(0.35)
    }
    switch participant.state {
    case "local", "online", "present":
        return AppColors.ok
    case "pending":
        return .orange
    default:
        return Color.gray.opacity(0.35)
    }
}

private func isFipsRouted(_ participant: ParticipantState, state: AppState) -> Bool {
    !isSelf(participant, state: state)
        && participant.reachable
        && participant.fipsTransportAddr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
}

private let bech32BodyCharset: Set<Character> = Set("qpzry9x8gf2tvdw0s3jn54khce6mua7l")

/// A valid device ID is a bech32-encoded npub: `npub1` + 58 bech32 chars.
func isValidDeviceId(_ value: String) -> Bool {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.count == 63, trimmed.hasPrefix("npub1") else { return false }
    return trimmed.dropFirst(5).allSatisfy { bech32BodyCharset.contains($0) }
}
