import AppKit
import CoreImage
import SwiftUI

private let searchVisibilityThreshold = 7

private enum PaidInternetFeature {
    static var enabled: Bool {
        #if DEBUG
        let arguments = Set(CommandLine.arguments)
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

private enum AddNetworkMode {
    case create
    case join
}

struct RootView: View {
    @ObservedObject var manager: AppManager

    @State private var nodeName = ""
    @State private var endpoint = ""
    @State private var tunnelIp = ""
    @State private var listenPort = ""
    @State private var relayInput = ""
    @State private var fipsHostInboundTcpPorts = ""
    @State private var nostrPubsubMode = "relay"
    @State private var nostrPubsubFanout = ""
    @State private var nostrPubsubMaxHops = ""
    @State private var nostrPubsubMaxEventBytes = ""
    @State private var wireguardExitConfig = ""
    @State private var paidExitMeter = "bytes"
    @State private var paidExitPriceMsat = ""
    @State private var paidExitPerUnits = ""
    @State private var paidExitAcceptedMints = ""
    @State private var paidExitMaxChannelCapacitySat = ""
    @State private var paidExitChannelExpirySecs = ""
    @State private var paidExitFreeProbeUnits = ""
    @State private var paidExitGraceUnits = ""
    @State private var paidExitCountryCode = ""
    @State private var paidExitRegion = ""
    @State private var paidExitAsn = ""
    @State private var paidExitNetworkClass = "unknown"
    @State private var paidExitIpv4 = true
    @State private var paidExitIpv6 = false
    @State private var paidRouteMintUrl = "https://mint.minibits.cash/Bitcoin"
    @State private var paidRouteMintLabel = "Minibits"
    @State private var paidRouteTopupAmount = "1000"
    @State private var paidRouteReceiveToken = ""
    @State private var paidRouteSendAmount = "1000"
    @State private var paidRouteWithdrawInvoice = ""
    @State private var paidExitAdvancedTermsExpanded = RootView.initialPaidExitAdvancedTermsExpanded()
    @State private var paidExitListingAdvancedExpanded = false
    @State private var wireGuardUpstreamExpanded = RootView.initialWireGuardUpstreamExpanded()
    @State private var paidRouteOfferCountryFilter = "all"
    @State private var paidRouteOfferNetworkFilter = "all"
    @State private var paidRouteOfferSort = "quality"
    @State private var networkNameInput = ""
    @State private var selectedDevicePubkeyHex: String?
    @State private var networkNameDrafts: [String: String] = [:]
    @State private var savedNetworksExpanded = false
    @State private var pendingNetworkRemoval: NativeNetworkState?
    @State private var pendingParticipantRemoval: PendingParticipantRemoval?
    @State private var pendingJoinRequest: PendingJoinRequest?
    @State private var addByDeviceIdInput = ""
    @State private var addByDeviceIdAlias = ""
    @State private var diagnosticsExpanded = false
    @State private var showingQrScanner = false
    @State private var scanningJoinRequest = false
    @State private var scannedQrCode: String?
    @State private var selectedSidebarItem: SidebarItem? = RootView.initialSidebarItem()
    @State private var shownNetworkId: String?
    @State private var addNetworkPresented = false
    @State private var addDevicePresented = false
    @State private var addNetworkMode: AddNetworkMode?
    @State private var legacyInviteExpanded = false
    @State private var joinRequestInput = ""
    @State private var manualJoinExpanded = false
    @State private var manualJoinAdminId = ""
    @State private var manualJoinMeshId = ""
    @State private var lastSyncedNodeName = ""
    @State private var lastSyncedEndpoint = ""
    @State private var lastSyncedTunnelIp = ""
    @State private var lastSyncedListenPort: UInt32 = 0
    @State private var lastSyncedFipsHostInboundTcpPorts = ""
    @State private var lastSyncedNostrPubsubMode = ""
    @State private var lastSyncedNostrPubsubFanout: UInt32 = 0
    @State private var lastSyncedNostrPubsubMaxHops: UInt8 = 0
    @State private var lastSyncedNostrPubsubMaxEventBytes: UInt32 = 0
    @State private var lastSyncedWireguardExitConfig: String? = nil
    @State private var lastSyncedPaidExitSeller: NativePaidExitSellerState? = nil

    private var state: NativeAppState {
        manager.state
    }

    private var activeNetwork: NativeNetworkState? {
        manager.activeNetwork
    }

    private static func initialSidebarItem() -> SidebarItem {
        let arguments = Set(CommandLine.arguments)
        if PaidInternetFeature.enabled {
            if arguments.contains("--nvpn-screenshot-paid-seller") {
                return .sellExit
            }
            if arguments.contains("--nvpn-screenshot-paid-market") {
                return .publicExits
            }
            if arguments.contains("--nvpn-screenshot-paid-wallet") {
                return .wallet
            }
        }
        if arguments.contains("--nvpn-screenshot-exit-nodes")
            || arguments.contains("--nvpn-screenshot-upstream") {
            return .internet
        }
        if arguments.contains("--nvpn-screenshot-settings") {
            return .settings
        }
        return .devices
    }

    private static func initialWireGuardUpstreamExpanded() -> Bool {
        Set(CommandLine.arguments).contains("--nvpn-screenshot-upstream")
    }

    private static func initialPaidExitAdvancedTermsExpanded() -> Bool {
        false
    }

    private var shownNetwork: NativeNetworkState? {
        if let shownNetworkId,
           let network = state.networks.first(where: { $0.id == shownNetworkId }) {
            return network
        }
        return activeNetwork ?? state.networks.first
    }

    private var paidRouteMarketAvailable: Bool {
        PaidInternetFeature.enabled && state.paidRouteMarket.supported
    }

    private var paidExitSellerAvailable: Bool {
        PaidInternetFeature.enabled && state.paidExitSeller.supported
    }

    private var paidInternetAvailable: Bool {
        paidRouteMarketAvailable || paidExitSellerAvailable
    }

    private var visibleSidebarItem: SidebarItem {
        let item = selectedSidebarItem ?? .devices
        return sidebarItemVisible(item) ? item : .devices
    }

    var body: some View {
        VStack(spacing: 0) {
            headerBar
            if manager.serviceUpdateRecommended {
                serviceUpdateStripe
            }
            if manager.updateAvailable {
                updateStripe
            }
            Divider()
            HStack(spacing: 0) {
                sidebar
                    .frame(width: 250)
                Divider()
                detailPane
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
        .ignoresSafeArea(.container, edges: .top)
        .onAppear {
            syncDrafts()
            normalizeSidebarSelection()
        }
        .onChange(of: state.rev) { _, _ in
            syncDrafts()
            normalizeSidebarSelection()
        }
        .onChange(of: shownNetwork?.enabled) { _, enabled in
            if addDevicePresented && enabled != true {
                addDevicePresented = false
            }
        }
        .onChange(of: addNetworkPresented) { _, presented in
            if !presented {
                addNetworkMode = nil
            }
        }
        .sheet(isPresented: $showingQrScanner, onDismiss: qrScannerDismissed) {
            QRCodeScannerSheet { code in
                scannedQrCode = code
                showingQrScanner = false
            }
        }
        .sheet(isPresented: $addNetworkPresented) {
            addNetworkSheetContent
        }
        .sheet(isPresented: $addDevicePresented) {
            if let network = shownNetwork, network.enabled {
                addDeviceSheetContent(network)
                    .alert("Add device?", isPresented: pendingJoinRequestPresented, presenting: pendingJoinRequest) { pending in
                        Button("Cancel", role: .cancel) {
                            pendingJoinRequest = nil
                        }
                        Button("Add") {
                            manager.importJoinRequest(pending.request)
                            joinRequestInput = ""
                            pendingJoinRequest = nil
                            addDevicePresented = false
                        }
                    } message: { pending in
                        Text("Add the device from this join request to \(pending.networkName)?")
                    }
            }
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
        let code = scannedQrCode
        let network = shownNetwork
        let shouldImport = scanningJoinRequest
        scannedQrCode = nil
        scanningJoinRequest = false

        guard shouldImport, let code, let network else {
            return
        }
        DispatchQueue.main.async {
            importJoinRequestOrAddDevice(code, network: network)
        }
    }

    private var addNetworkSheetContent: some View {
        VStack(alignment: .leading, spacing: 0) {
            sheetTitleBar("Add Network", systemImage: "plus.circle") {
                addNetworkPresented = false
            }
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    switch addNetworkMode {
                    case nil:
                        addNetworkChoiceSection
                    case .create:
                        addNetworkBackButton
                        createNetworkSection
                    case .join:
                        addNetworkBackButton
                        joinNetworkSection(activeNetwork)
                    }
                }
                .padding(18)
            }
        }
        .frame(width: 560, height: 620)
    }

    private func addDeviceSheetContent(_ network: NativeNetworkState) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            sheetTitleBar("Link Device", systemImage: "person.badge.plus") {
                addDevicePresented = false
            }
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    joinRequestInputSection(network)
                    nearbyJoinRequestsSection
                    manualPairingInfoSection(network)
                    addByDeviceIdSection(network)
                }
                .padding(18)
            }
        }
        .frame(width: 560, height: 620)
    }

    /// Manual pairing path for directly sharing the signed-roster values.
    private func manualPairingInfoSection(_ network: NativeNetworkState) -> some View {
        surface {
            sectionHeader("Manual Pairing", systemImage: "keyboard")
            Text("Share these values with the other device, then add its Device ID below to keep the signed roster in sync.")
                .font(.caption)
                .foregroundStyle(.secondary)
            detailValueRow("Your Device ID", state.ownNpub)
            detailValueRow("Network ID", network.networkId, displayValue: displayNetworkId(network.networkId))
        }
    }

    private func sheetTitleBar(_ title: String, systemImage: String, close: @escaping () -> Void) -> some View {
        HStack(spacing: 10) {
            Label(title, systemImage: systemImage)
                .font(.title3.weight(.semibold))
            Spacer()
            Button("Done", action: close)
                .keyboardShortcut(.cancelAction)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
    }

    private var headerBar: some View {
        HStack(spacing: 18) {
            headerIdentity
            Spacer(minLength: 0)
            headerVpnControl
        }
        .padding(.leading, 104)
        .padding(.trailing, 18)
        .frame(height: 44)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var serviceUpdateStripe: some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color.orange)
            Text(serviceUpdateStripeText)
                .font(.callout)
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 12)
            Button {
                manager.installService()
            } label: {
                Text(manager.serviceSettling ? "Updating…" : "Update")
            }
            .controlSize(.small)
            .disabled(!state.serviceSupported || manager.actionInFlight || manager.serviceSettling)
        }
        .padding(.leading, 104)
        .padding(.trailing, 18)
        .padding(.vertical, 6)
        .background(Color(nsColor: .underPageBackgroundColor))
        .overlay(alignment: .bottom) {
            Divider()
        }
    }

    private var serviceUpdateStripeText: String {
        let installed = state.serviceBinaryVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        let expected = state.expectedServiceBinaryVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        if installed.isEmpty || expected.isEmpty {
            return "Background service needs update to match the app"
        }
        return "Background service is on v\(installed); update to match app v\(expected)"
    }

    private var updateStripe: some View {
        HStack(spacing: 10) {
            Image(systemName: "arrow.down.circle.fill")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.secondary)
            Text(updateStripeText)
                .font(.callout)
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 12)
            Toggle("Install automatically", isOn: $manager.autoInstallUpdates)
                .toggleStyle(.checkbox)
                .font(.caption)
                .foregroundStyle(.secondary)
            Button {
                manager.installUpdate()
            } label: {
                Text(manager.updateInstalling ? "Installing…" : "Install")
            }
            .controlSize(.small)
            .disabled(!manager.updateInstallEnabled)
        }
        .padding(.leading, 104)
        .padding(.trailing, 18)
        .padding(.vertical, 6)
        .background(Color(nsColor: .underPageBackgroundColor))
        .overlay(alignment: .bottom) {
            Divider()
        }
    }

    private var updateStripeText: String {
        let current = state.appVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        if current.isEmpty {
            return "Update available: \(manager.updateVersion)"
        }
        return "Update available: \(manager.updateVersion) (you're on \(current))"
    }

    private var systemVersionLabel: String {
        let app = state.appVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        let daemon = state.daemonBinaryVersion.trimmingCharacters(in: .whitespacesAndNewlines)
        switch (app.isEmpty, daemon.isEmpty) {
        case (true, true): return ""
        case (false, true): return "gui v\(app)"
        case (true, false): return "daemon v\(daemon)"
        case (false, false) where app == daemon: return "v\(app)"
        case (false, false): return "gui v\(app) · daemon v\(daemon)"
        }
    }

    private var headerIdentity: some View {
        HStack(spacing: 6) {
            if let shownNetwork, state.networks.count > 1 {
                networkStatusDot(shownNetwork)
            }
            Picker("", selection: headerNetworkSelection) {
                ForEach(state.networks, id: \.id) { network in
                    Text(displayName(network))
                        .tag(network.id)
                }
            }
            .labelsHidden()
            .pickerStyle(.menu)
            .disabled(state.networks.isEmpty)
            .frame(maxWidth: 160, alignment: .leading)

            Button {
                addNetworkPresented = true
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 11, weight: .semibold))
            }
            .buttonStyle(.borderless)
            .help("Add network")
        }
        .frame(width: 220, alignment: .leading)
    }

    private var headerNetworkSelection: Binding<String> {
        Binding(
            get: { shownNetwork?.id ?? state.networks.first?.id ?? "" },
            set: { networkId in
                guard let network = state.networks.first(where: { $0.id == networkId }) else {
                    return
                }
                shownNetworkId = network.id
                selectedSidebarItem = .devices
                if !network.enabled {
                    activateNetwork(network)
                }
            }
        )
    }

    private func networkStatusDot(_ network: NativeNetworkState) -> some View {
        Circle()
            .fill(network.enabled ? Color.green : Color.secondary.opacity(0.55))
            .frame(width: 7, height: 7)
    }

    private var headerVpnControl: some View {
        HStack(spacing: 8) {
            Text(headerVpnStatusText)
                .font(.caption2)
                .foregroundStyle(headerStatusTextColor)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: 150, alignment: .trailing)
                .layoutPriority(1)
            if headerStatusDotVisible {
                Circle()
                    .fill(headerStatusColor)
                    .frame(width: 7, height: 7)
            }
            headerVpnSwitch
        }
        .help(manager.vpnSwitchEnabled ? "Turn VPN off" : "Turn VPN on")
    }

    private var headerVpnSwitch: some View {
        let disabled = manager.actionInFlight || !state.vpnControlSupported || activeNetwork == nil
        return Button {
            manager.toggleVpn()
        } label: {
            ZStack(alignment: manager.vpnSwitchEnabled ? .trailing : .leading) {
                Capsule()
                    .fill(manager.vpnSwitchEnabled ? Color.accentColor : Color(nsColor: .tertiaryLabelColor).opacity(0.45))
                    .frame(width: 52, height: 26)
                Circle()
                    .fill(Color.white)
                    .frame(width: 22, height: 22)
                    .shadow(color: .black.opacity(0.22), radius: 1, y: 1)
                    .padding(2)
            }
            .frame(width: 52, height: 26)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .accessibilityLabel(manager.vpnSwitchEnabled ? "Turn VPN off" : "Turn VPN on")
        .accessibilityValue(manager.vpnSwitchEnabled ? "On" : "Off")
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 5) {
            sidebarButton(.devices, "Devices", "circle.grid.2x2.fill")
            sidebarButton(.internet, "Internet", "network")
            if paidInternetAvailable {
                sidebarGroupLabel("Internet market")
                if paidRouteMarketAvailable {
                    sidebarButton(.publicExits, "Buy Internet", "cart.fill")
                }
                if paidExitSellerAvailable {
                    sidebarButton(.sellExit, "Share Internet", "bitcoinsign.circle.fill")
                }
                if paidRouteMarketAvailable {
                    sidebarButton(.wallet, "Wallet", "creditcard.fill")
                }
            }
            sidebarGroupLabel("App")
            sidebarButton(.settings, "Settings", "gearshape")
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .padding(.top, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(nsColor: .controlBackgroundColor))
    }

    private func sidebarGroupLabel(_ title: String) -> some View {
        Text(title.uppercased())
            .font(.caption2.weight(.semibold))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 12)
            .padding(.top, 14)
            .padding(.bottom, 2)
    }

    private func sidebarButton(_ item: SidebarItem, _ title: String, _ systemImage: String) -> some View {
        let selected = visibleSidebarItem == item
        return Button {
            selectedSidebarItem = item
        } label: {
            HStack(spacing: 8) {
                Label(title, systemImage: systemImage)
                    .labelStyle(.titleAndIcon)
                Spacer(minLength: 0)
            }
            .font(.subheadline.weight(.semibold))
            .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .frame(height: 32)
                .background(selected ? Color.accentColor : Color.clear, in: RoundedRectangle(cornerRadius: 7))
                .foregroundStyle(selected ? Color.white : Color.primary)
                .contentShape(RoundedRectangle(cornerRadius: 7))
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var detailPane: some View {
        switch visibleSidebarItem {
        case .devices:
            if let shownNetwork {
                devicesPane(shownNetwork)
            } else {
                setupPane
            }
        case .internet:
            pageScroll {
                pageTitle("Internet", "network")
                if let shownNetwork {
                    internetSection(shownNetwork)
                } else {
                    wireGuardExitSettings
                }
            }
        case .publicExits:
            pageScroll {
                pageTitle("Buy Internet", "cart.fill")
                paidRouteMarketSettings
            }
        case .sellExit:
            pageScroll {
                pageTitle("Share Internet", "bitcoinsign.circle.fill")
                paidExitSellerSettings
            }
        case .wallet:
            pageScroll {
                pageTitle("Wallet", "creditcard.fill")
                paidRouteWalletSettings
            }
        case .settings:
            pageScroll {
                pageTitle("Settings", "gearshape")
                settingsSection
            }
        }
    }

    private func sidebarItemVisible(_ item: SidebarItem) -> Bool {
        switch item {
        case .publicExits, .wallet:
            return paidRouteMarketAvailable
        case .sellExit:
            return paidExitSellerAvailable
        case .devices, .internet, .settings:
            return true
        }
    }

    private func normalizeSidebarSelection() {
        if let selectedSidebarItem, !sidebarItemVisible(selectedSidebarItem) {
            self.selectedSidebarItem = .devices
        }
    }

    private func pageScroll<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                content()
            }
            .padding(.horizontal, 28)
            .padding(.top, 28)
            .padding(.bottom, 32)
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private func pageTitle(_ title: String, _ systemImage: String) -> some View {
        Label(title, systemImage: systemImage)
            .font(.system(size: 24, weight: .semibold))
    }

    private func devicesPane(_ network: NativeNetworkState) -> some View {
        HStack(spacing: 0) {
            deviceListColumn(network)
                .frame(minWidth: 290, idealWidth: 330, maxWidth: 360)
            Divider()
            deviceDetailColumn(network)
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var setupPane: some View {
        pageScroll {
            pageTitle("Add Network", "plus.circle")
            switch addNetworkMode {
            case nil:
                addNetworkChoiceSection
            case .create:
                addNetworkBackButton
                createNetworkSection
            case .join:
                addNetworkBackButton
                joinNetworkSection(nil)
            }
        }
    }

    private var addNetworkChoiceSection: some View {
        surface {
            VStack(alignment: .leading, spacing: 10) {
                Button {
                    addNetworkMode = .create
                } label: {
                    Label("Create Network", systemImage: "plus.circle.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)

                Button {
                    addNetworkMode = .join
                } label: {
                    Label("Join Network", systemImage: "arrow.down.circle.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
            }
        }
    }

    private var addNetworkBackButton: some View {
        HStack {
            Button {
                addNetworkMode = nil
            } label: {
                Label("Back", systemImage: "chevron.left")
            }
            Spacer()
        }
    }

    private func deviceListColumn(_ network: NativeNetworkState) -> some View {
        LocalSearchScope { search in
            deviceListColumn(network, search: search)
        }
    }

    private func deviceListColumn(_ network: NativeNetworkState, search: Binding<String>) -> some View {
        let showSearch = sortedParticipants(network).count > searchVisibilityThreshold
        return VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text("Devices")
                            .font(.system(size: 24, weight: .semibold))
                        Text(deviceAvailabilityText(network))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer()
                    deviceHeaderActions(network)
                }
                if showSearch {
                    TextField("Search", text: search)
                        .textFieldStyle(.roundedBorder)
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 24)
            .padding(.bottom, 4)

            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    let activeSearch = showSearch ? search.wrappedValue : ""
                    let participants = visibleParticipants(network, search: activeSearch)
                    let selectedPubkeyHex = selectedParticipant(in: network)?.pubkeyHex
                    if participants.isEmpty {
                        emptyRow(
                            activeSearch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                                ? "No devices"
                                : "No matching devices",
                            systemImage: "circle.dotted"
                        )
                    } else {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(displayName(network))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .padding(.horizontal, 10)
                            ForEach(participants, id: \.pubkeyHex) { participant in
                                deviceListRow(
                                    participant,
                                    network: network,
                                    selected: selectedPubkeyHex == participant.pubkeyHex
                                )
                            }
                        }
                    }

                }
                .padding(.horizontal, 12)
                .padding(.bottom, 24)
            }
        }
    }

    private func deviceHeaderActions(_ network: NativeNetworkState) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                activateNetworkButton(network)
                addDeviceButton(network)
            }
            HStack(spacing: 8) {
                activateNetworkButton(network, compact: true)
                addDeviceButton(network, compact: true)
            }
        }
    }

    @ViewBuilder
    private func activateNetworkButton(_ network: NativeNetworkState, compact: Bool = false) -> some View {
        if !network.enabled {
            Button {
                activateNetwork(network)
            } label: {
                if compact {
                    Image(systemName: "checkmark.circle.fill")
                } else {
                    Label("Activate", systemImage: "checkmark.circle.fill")
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.small)
            .disabled(manager.actionInFlight)
            .help("Activate this network")
        }
    }

    @ViewBuilder
    private func addDeviceButton(_ network: NativeNetworkState, compact: Bool = false) -> some View {
        if network.localIsAdmin {
            Button {
                addDevicePresented = true
            } label: {
                if compact {
                    Image(systemName: "person.badge.plus")
                } else {
                    Label("Link device", systemImage: "person.badge.plus")
                }
            }
            .controlSize(.small)
            .disabled(!network.enabled)
            .help(network.enabled ? "Link device to this network" : "Activate this network first")
        }
    }

    private func deviceListRow(
        _ participant: NativeParticipantState,
        network: NativeNetworkState,
        selected: Bool
    ) -> some View {
        Button {
            selectedDevicePubkeyHex = participant.pubkeyHex
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 7) {
                    connectivityDot(participant, size: 8)
                    Text(deviceName(participant))
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    if isSelf(participant) {
                        badge("This device", style: selected ? .selected : .ok)
                    }
                    if participant.isAdmin {
                        badge("Admin", style: selected ? .selected : .muted)
                    }
                    if participant.offersExitNode {
                        badge(exitNodeBadgeText(participant), style: selected ? .selected : exitNodeBadgeStyle(participant))
                    }
                    if isFipsRouted(participant) {
                        badge("via mesh", style: selected ? .selected : .muted)
                    }
                }
                HStack(spacing: 6) {
                    if !deviceSubtitle(participant).isEmpty {
                        Text(deviceSubtitle(participant))
                    }
                    if !cleanIp(participant.tunnelIp).isEmpty {
                        Text(cleanIp(participant.tunnelIp))
                    }
                }
                .font(.caption)
                .foregroundStyle(selected ? Color.white.opacity(0.78) : Color.secondary)
                .lineLimit(1)
            }
            .foregroundStyle(selected ? Color.white : Color.primary)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                selected ? Color.accentColor : Color.clear,
                in: RoundedRectangle(cornerRadius: 7)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func deviceDetailColumn(_ network: NativeNetworkState) -> some View {
        ScrollView {
            if let participant = selectedParticipant(in: network) {
                VStack(alignment: .leading, spacing: 22) {
                    deviceDetailHeader(participant, network: network)
                    if network.localIsAdmin {
                        deviceAdminSection(participant, network: network)
                    }
                    deviceAddressesSection(participant)
                    deviceConnectivitySection(participant)
                }
                .padding(.horizontal, 22)
                .padding(.top, 26)
                .padding(.bottom, 30)
                .frame(maxWidth: 640, alignment: .topLeading)
                .frame(maxWidth: .infinity, alignment: .topLeading)
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Devices")
                        .font(.system(size: 24, weight: .semibold))
                    emptyRow("No devices yet", systemImage: "circle.dotted")
                }
                .padding(28)
                .frame(maxWidth: .infinity, alignment: .topLeading)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private func deviceDetailHeader(_ participant: NativeParticipantState, network: NativeNetworkState) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(deviceName(participant))
                        .font(.system(size: 24, weight: .semibold))
                        .lineLimit(2)
                    if isSelf(participant) || participant.isAdmin || participant.offersExitNode || participant.reachable {
                        HStack(spacing: 6) {
                            if isSelf(participant) {
                                badge("This device", style: .ok)
                            }
                            if participant.isAdmin {
                                badge("Admin", style: .muted)
                            }
                            if participant.offersExitNode {
                                badge(exitNodeBadgeText(participant), style: exitNodeBadgeStyle(participant))
                            }
                            if isDirectFipsPeer(participant) {
                                badge("direct connection", style: .ok)
                            } else if isFipsRouted(participant) {
                                badge("via mesh", style: .muted)
                            }
                        }
                    }
                }
                Spacer()
                HStack(spacing: 7) {
                    connectivityDot(participant, size: 8)
                    Text(deviceStatusText(participant))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private func deviceAdminSection(_ participant: NativeParticipantState, network: NativeNetworkState) -> some View {
        surface {
            sectionHeader("Manage Device", systemImage: "person.badge.key")

            SyncedTextFieldRow(
                title: "Name",
                placeholder: "Name",
                identity: participant.pubkeyHex,
                value: participant.magicDnsAlias,
                systemImage: "checkmark",
                disabled: manager.actionInFlight
            ) { draft in
                manager.setParticipantAlias(
                    npub: participant.npub,
                    alias: draft
                )
            }

            if !isSelf(participant) {
                SyncedTextFieldRow(
                    title: "Hints",
                    placeholder: "host or host:port",
                    identity: participant.pubkeyHex,
                    value: participant.fipsEndpointHints.joined(separator: ", "),
                    systemImage: "network",
                    disabled: manager.actionInFlight
                ) { draft in
                    manager.setParticipantEndpointHints(
                        npub: participant.npub,
                        endpointHints: endpointHints(from: draft)
                    )
                }
                deviceActionButtons(participant, network: network)
            }
        }
    }

    private func deviceAddressesSection(_ participant: NativeParticipantState) -> some View {
        surface {
            Text("Addresses")
                .font(.headline)
            detailValueRow("MagicDNS", deviceMagicDnsName(participant))
            detailValueRow("VPN IP", cleanIp(participant.tunnelIp))
            detailValueRow("Device ID", participant.npub)
        }
    }

    private func deviceConnectivitySection(_ participant: NativeParticipantState) -> some View {
        surface {
            Text("Connectivity")
                .font(.headline)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 130), alignment: .leading)], alignment: .leading, spacing: 12) {
                metric("Role", deviceRoleText(participant))
                metric("State", deviceStatusText(participant))
                metric("FIPS path", fipsPathText(participant))
                metric("Latency age", participant.fipsSrttAgeMs == 0 ? "-" : formatDurationMs(participant.fipsSrttAgeMs))
                metric("Control seen", participant.lastFipsControlSeenText.isEmpty ? "-" : participant.lastFipsControlSeenText)
                metric("Data seen", participant.lastFipsDataSeenText.isEmpty ? "-" : participant.lastFipsDataSeenText)
                metric("Address hints", participant.fipsEndpointHints.isEmpty ? "-" : participant.fipsEndpointHints.joined(separator: ", "))
                metric("Last seen", participant.lastSeenText.isEmpty ? "-" : participant.lastSeenText)
                metric("Sent", formatBytes(participant.txBytes))
                metric("Received", formatBytes(participant.rxBytes))
            }
            if !participant.statusText.isEmpty {
                Text(participant.statusText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
        }
    }

    private func deviceActionButtons(_ participant: NativeParticipantState, network: NativeNetworkState) -> some View {
        HStack(spacing: 6) {
            Button {
                manager.toggleAdmin(networkId: network.id, participant: participant)
            } label: {
                Label(
                    participant.isAdmin ? "Remove admin" : "Make admin",
                    systemImage: participant.isAdmin ? "star.slash" : "star"
                )
            }
            .disabled(manager.actionInFlight)
            .help(participant.isAdmin ? "Remove admin" : "Make admin")
            Button(role: .destructive) {
                pendingParticipantRemoval = PendingParticipantRemoval(
                    networkId: network.id,
                    npub: participant.npub,
                    deviceName: deviceName(participant)
                )
            } label: {
                Label("Remove", systemImage: "trash")
            }
            .disabled(isSelf(participant) || manager.actionInFlight)
            .help("Remove device")
        }
        .controlSize(.small)
        .confirmationDialog(
            "Remove \(pendingParticipantRemoval?.deviceName ?? "device")?",
            isPresented: Binding(
                get: { pendingParticipantRemoval != nil },
                set: { if !$0 { pendingParticipantRemoval = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                if let target = pendingParticipantRemoval {
                    manager.removeParticipant(networkId: target.networkId, npub: target.npub)
                }
                pendingParticipantRemoval = nil
            }
            Button("Cancel", role: .cancel) { pendingParticipantRemoval = nil }
        } message: {
            Text("This removes the device from the network's roster. They keep the network locally but won't be in this roster anymore.")
        }
    }

    private func detailValueRow(_ title: String, _ value: String, displayValue customDisplayValue: String? = nil) -> some View {
        let displayValue = value.isEmpty ? "-" : customDisplayValue ?? value
        return VStack(alignment: .leading, spacing: 4) {
            if value.hasPrefix("npub1") {
                WrappingIdentifierText(
                    value: displayValue,
                    font: .preferredFont(forTextStyle: .subheadline),
                    color: .labelColor
                )
            } else {
                Text(displayValue)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(nil)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
            }
            HStack(spacing: 6) {
                Text(title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                if !value.isEmpty {
                    Button {
                        manager.copy(value)
                    } label: {
                        Image(systemName: "doc.on.doc")
                    }
                    .buttonStyle(.borderless)
                    .help("Copy")
                }
            }
        }
        .padding(.vertical, 3)
    }

    private func joinRequestInputSection(_ network: NativeNetworkState) -> some View {
        return surface {
            sectionHeader("Add Join Request", systemImage: "camera.viewfinder")
            Text("Scan the joining device's request QR, or paste its join request. Valid links open confirmation automatically.")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack(spacing: 8) {
                Button {
                    scanningJoinRequest = true
                    showingQrScanner = true
                } label: {
                    Label("Scan QR", systemImage: "camera.viewfinder")
                }
                .disabled(manager.actionInFlight)
                TextField("nvpn://join-request/…", text: $joinRequestInput)
                    .textFieldStyle(.roundedBorder)
                    .onChange(of: joinRequestInput) { _, value in
                        stageJoinRequest(value, network: network)
                    }
            }
        }
    }

    private func importJoinRequestOrAddDevice(_ value: String, network: NativeNetworkState) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if looksLikeJoinRequestQrOrLink(trimmed) {
            stageJoinRequest(trimmed, network: network)
            return
        }
        if isValidDeviceId(trimmed) {
            manager.addParticipant(networkId: network.id, npub: trimmed)
        } else {
            manager.importJoinRequest(trimmed)
        }
    }

    private func stageJoinRequest(_ value: String, network: NativeNetworkState) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard looksLikeJoinRequestQrOrLink(trimmed) else { return }
        pendingJoinRequest = PendingJoinRequest(
            networkId: network.id,
            networkName: network.name.isEmpty ? "this network" : network.name,
            request: trimmed
        )
    }

    private func addByDeviceIdSection(_ network: NativeNetworkState) -> some View {
        let trimmed = addByDeviceIdInput.trimmingCharacters(in: .whitespacesAndNewlines)
        let invalid = !trimmed.isEmpty && !isValidDeviceId(trimmed)
        return surface {
            sectionHeader("Add by Device ID", systemImage: "plus")
            Text("Paste the joining device's npub to add it directly. The joining device still needs your Device ID and this network ID for manual pairing.")
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack(spacing: 8) {
                TextField("Device ID", text: $addByDeviceIdInput)
                    .textFieldStyle(.roundedBorder)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color.red, lineWidth: invalid ? 1 : 0)
                    )
                TextField("Name (optional)", text: $addByDeviceIdAlias)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 200)
                Button {
                    manager.addParticipant(networkId: network.id, npub: trimmed, alias: addByDeviceIdAlias)
                    addByDeviceIdInput = ""
                    addByDeviceIdAlias = ""
                } label: {
                    Label("Add to Roster", systemImage: "plus")
                }
                .disabled(trimmed.isEmpty || invalid || manager.actionInFlight)
            }
            if invalid {
                Text("Not a valid device ID")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }

    private var createNetworkSection: some View {
        surface {
            sectionHeader("Create Network", systemImage: "plus.circle")
            HStack(spacing: 8) {
                TextField("Network name", text: $networkNameInput)
                    .onSubmit {
                        addNetwork()
                        finishCreateNetwork()
                    }
                Button {
                    addNetwork(defaultName: "My Network")
                    finishCreateNetwork()
                } label: {
                    Label("Create", systemImage: "plus")
                }
                .disabled(manager.actionInFlight)
            }
        }
    }

    /// Land on the new network's Devices view right after Create. The
    /// sidebar may have been on Routing/Settings; from the Add Network
    /// sheet we also have to dismiss. Both are no-ops if already in the
    /// target state.
    private func finishCreateNetwork() {
        addNetworkPresented = false
        selectedSidebarItem = .devices
    }

    private func joinNetworkSection(_ network: NativeNetworkState?) -> some View {
        let requestNetwork = network ?? state.networks.first { candidate in
            !candidate.joinRequestQrCodeOrLink.isEmpty
        }
        let joinRequestQrCodeOrLink: String
        if state.joinRequestQrCodeOrLink.isEmpty {
            joinRequestQrCodeOrLink = requestNetwork?.joinRequestQrCodeOrLink ?? ""
        } else {
            joinRequestQrCodeOrLink = state.joinRequestQrCodeOrLink
        }
        return surface {
            sectionHeader("Join Network", systemImage: "arrow.down.circle")
            if !joinRequestQrCodeOrLink.isEmpty {
                InviteQRCodeView(invite: joinRequestQrCodeOrLink)
                    .frame(width: 220, height: 220)
                    .frame(maxWidth: .infinity, alignment: .center)
                HStack(spacing: 8) {
                    Button {
                        manager.copy(joinRequestQrCodeOrLink)
                    } label: {
                        Label("Copy Request", systemImage: "doc.on.doc")
                    }
                    Button {
                        manager.share(joinRequestQrCodeOrLink)
                    } label: {
                        Label("Share", systemImage: "square.and.arrow.up")
                    }
                }
                .disabled(manager.actionInFlight)
                detailValueRow("Your Device ID", state.ownNpub)
            }

            DisclosureGroup("Legacy invite link", isExpanded: $legacyInviteExpanded) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        TextField("nvpn://invite/…", text: $manager.inviteInput)
                            .onChange(of: manager.inviteInput) { _, newValue in
                                // Auto-import when the field becomes a valid invite —
                                // saves the user a click. importInvite clears the
                                // field, which prevents re-firing.
                                let trimmed = newValue.trimmingCharacters(in: .whitespacesAndNewlines)
                                if trimmed.lowercased().hasPrefix("nvpn://invite/") {
                                    manager.linkNetwork(trimmed)
                                }
                            }
                            .onSubmit {
                                manager.linkNetwork(manager.inviteInput)
                            }
                        Button {
                            pasteInviteFromClipboard()
                        } label: {
                            Label("Paste", systemImage: "doc.on.clipboard")
                        }
                    }
                }
                .padding(.top, 6)
            }

            manualJoinDisclosure

            advertiseJoinRequestSection
        }
    }

    private var advertiseJoinRequestSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Divider()
            HStack {
                Text("Nearby join request")
                    .font(.subheadline.weight(.medium))
                Spacer()
                Button {
                    state.inviteBroadcastActive ? manager.stopJoinRequestBroadcast() : manager.startJoinRequestBroadcast()
                } label: {
                    Label(
                        state.inviteBroadcastActive
                            ? "Advertising · \(formatRemaining(state.inviteBroadcastRemainingSecs))"
                            : "Advertise nearby",
                        systemImage: state.inviteBroadcastActive ? "stop.circle" : "dot.radiowaves.left.and.right"
                    )
                }
                .disabled(manager.actionInFlight)
            }
            Text(state.inviteBroadcastActive ? "Admins nearby can add this device from its join request." : "Advertise this device's join request to nearby admins.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var nearbyJoinRequestsSection: some View {
        surface {
            HStack {
                sectionHeader("Nearby Join Requests", systemImage: "dot.radiowaves.left.and.right")
                Spacer()
                Button {
                    state.nearbyDiscoveryActive ? manager.stopNearbyDiscovery() : manager.startNearbyDiscovery()
                } label: {
                    Label(
                        state.nearbyDiscoveryActive
                            ? "Finding nearby · \(formatRemaining(state.nearbyDiscoveryRemainingSecs))"
                            : "Find nearby",
                        systemImage: state.nearbyDiscoveryActive ? "stop.circle" : "dot.radiowaves.left.and.right"
                    )
                }
                .disabled(manager.actionInFlight)
            }
            if state.nearbyDiscoveryActive && state.lanPeers.isEmpty {
                emptyRow("No nearby join requests yet", systemImage: "wifi")
            } else {
                ForEach(state.lanPeers, id: \.invite) { peer in
                    HStack {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(peer.nodeName.isEmpty ? peer.npub : peer.nodeName)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Text(peer.lastSeenText)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Add") {
                            manager.importJoinRequest(peer.invite)
                        }
                    }
                    .padding(.vertical, 4)
                }
            }
        }
    }

    private var manualJoinDisclosure: some View {
        let admin = manualJoinAdminId.trimmingCharacters(in: .whitespacesAndNewlines)
        let mesh = manualJoinMeshId.trimmingCharacters(in: .whitespacesAndNewlines)
        let adminInvalid = !admin.isEmpty && !isValidDeviceId(admin)
        let canSubmit = !admin.isEmpty && !mesh.isEmpty && !adminInvalid
        return DisclosureGroup("Manual join", isExpanded: $manualJoinExpanded) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Enter the admin's Device ID and network ID, then give the admin your Device ID shown above.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField("Admin Device ID", text: $manualJoinAdminId)
                    .textFieldStyle(.roundedBorder)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color.red, lineWidth: adminInvalid ? 1 : 0)
                    )
                if adminInvalid {
                    Text("Not a valid device ID")
                        .font(.caption)
                        .foregroundStyle(.red)
                }
                TextField("Network ID", text: $manualJoinMeshId)
                    .textFieldStyle(.roundedBorder)
                Button {
                    manager.manualAddNetwork(adminNpub: admin, meshNetworkId: mesh)
                    manualJoinAdminId = ""
                    manualJoinMeshId = ""
                    manualJoinExpanded = false
                } label: {
                    Label("Add", systemImage: "plus")
                }
                .disabled(!canSubmit || manager.actionInFlight)
            }
            .padding(.top, 6)
        }
        .font(.subheadline)
    }

    private func pasteInviteFromClipboard() {
        if let text = NSPasteboard.general.string(forType: .string) {
            manager.inviteInput = text.trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }

    private func formatRemaining(_ seconds: UInt64) -> String {
        if seconds == 0 { return "off" }
        let days = seconds / 86_400
        if days > 0 {
            let hours = (seconds % 86_400) / 3_600
            return hours == 0 ? "\(days)d" : "\(days)d \(hours)h"
        }
        let hours = seconds / 3_600
        if hours > 0 {
            let minutes = (seconds % 3_600) / 60
            return minutes == 0 ? "\(hours)h" : "\(hours)h \(minutes)m"
        }
        let minutes = seconds / 60
        if minutes == 0 { return "\(seconds)s" }
        let secs = seconds % 60
        return secs == 0 ? "\(minutes)m" : String(format: "%dm%02ds", minutes, secs)
    }

    private func internetSection(_ network: NativeNetworkState) -> some View {
        LocalSearchScope { search in
            internetSection(network, search: search)
        }
    }

    private func internetSection(_ network: NativeNetworkState, search: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            internetChoiceSettings
            trustedDeviceInternetSettings(network, search: search)
            shareInternetSettings
            wireGuardUpstreamSettings
        }
    }

    private var internetChoiceSettings: some View {
        return surface {
            sectionHeader("Use Internet", systemImage: "network")
            VStack(spacing: 8) {
                routeChoice(
                    title: "My internet",
                    subtitle: "Use my normal connection",
                    selected: !state.wireguardExitEnabled && state.exitNode.isEmpty,
                    enabled: true
                ) {
                    manager.selectDirectExit()
                }

                if paidRouteMarketAvailable {
                    let publicSession = state.paidRouteMarket.sessions.first { paidRouteSessionIsSelected($0) }
                    routeChoice(
                        title: "Bought internet",
                        subtitle: publicSession.map(paidPublicExitSubtitle) ?? "Connect in Buy Internet",
                        selected: publicSession != nil,
                        enabled: true
                    ) {
                        selectedSidebarItem = .publicExits
                    }
                }

                routeChoice(
                    title: "Upstream VPN",
                    subtitle: wireguardUpstreamSubtitle,
                    selected: state.wireguardExitEnabled,
                    enabled: state.wireguardExitConfigured
                ) {
                    manager.selectWireGuardUpstreamExit()
                }
            }
        }
    }

    private func trustedDeviceInternetSettings(_ network: NativeNetworkState, search: Binding<String>) -> some View {
        let allPeerExitCandidates = exitNodeCandidates(network, search: "")
        let showSearch = allPeerExitCandidates.count > searchVisibilityThreshold
        let activeSearch = showSearch ? search.wrappedValue : ""
        let peerExitCandidates = exitNodeCandidates(network, search: activeSearch)

        return surface {
            sectionHeader("Trusted Devices", systemImage: "lock.shield.fill")
            if showSearch {
                TextField("Search devices", text: search)
                    .textFieldStyle(.roundedBorder)
            }

            VStack(spacing: 8) {
                if peerExitCandidates.isEmpty {
                    emptyRow(
                        activeSearch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? "No trusted devices sharing internet"
                            : "No trusted devices found",
                        systemImage: "tray"
                    )
                } else {
                    ForEach(peerExitCandidates, id: \.pubkeyHex) { participant in
                        routeChoice(
                            title: deviceName(participant),
                            subtitle: participant.statusText.isEmpty ? "Trusted device" : participant.statusText,
                            selected: !state.wireguardExitEnabled && state.exitNode == participant.npub,
                            enabled: true
                        ) {
                            manager.selectPeerExit(participant.npub)
                        }
                    }
                }
            }
        }
    }

    private var shareInternetSettings: some View {
        surface {
            HStack(spacing: 12) {
                sectionHeader("Share with Trusted Devices", systemImage: "lock.shield.fill")
                Spacer(minLength: 16)
                Toggle("", isOn: Binding(
                    get: { state.advertiseExitNode },
                    set: { manager.setAdvertiseExitNode($0) }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .disabled(manager.actionInFlight)
            }
            Text("Only devices in \(shownNetworkLabel) can use it.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var shownNetworkLabel: String {
        shownNetwork.map(displayName) ?? "this network"
    }

    private var paidExitSellerSummaryText: String {
        if !state.paidExitSeller.enabled {
            return "People can pay to use this Mac's internet connection."
        }
        return "Sharing is on. Save changes before advertising a new listing."
    }

    private var wireguardUpstreamSubtitle: String {
        if !state.wireguardExitConfigured {
            return "Set up a WireGuard provider"
        }
        let endpoint = state.wireguardExitEndpoint
        if endpoint.isEmpty {
            return "Configured"
        }
        return "via \(endpoint)"
    }

    private var paidExitCurrentUpstream: String {
        state.wireguardExitEnabled ? "wireguard_exit" : "host_default"
    }

    private var paidExitCurrentInternetTitle: String {
        state.wireguardExitEnabled ? "My internet through WireGuard" : "My internet"
    }

    private var paidExitCurrentInternetDetail: String {
        if state.wireguardExitEnabled {
            return wireguardUpstreamSubtitle
        }
        return "The same connection this Mac already uses"
    }

    private var paidExitSellerSettings: some View {
        VStack(alignment: .leading, spacing: 14) {
            paidExitSellerStatusSettings
            if state.paidExitSeller.supported {
                paidExitSellerListingSettings
                paidExitSellerPaymentSettings
                paidExitSellerActivitySettings
                paidRouteWalletSection(state.paidRouteMarket.wallet)
                paidExitSellerTermsSettings
            }
        }
    }

    private var paidExitSellerStatusSettings: some View {
        surface {
            HStack(spacing: 12) {
                sectionHeader("Share My Internet", systemImage: "bitcoinsign.circle.fill")
                Spacer(minLength: 16)
                Toggle("", isOn: Binding(
                    get: { state.paidExitSeller.enabled },
                    set: { manager.setPaidExitEnabled($0) }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .disabled(manager.actionInFlight || !state.paidExitSeller.supported)
            }
            if !state.paidExitSeller.supported {
                Text("Selling internet is unavailable on this platform.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text(paidExitSellerSummaryText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if state.paidExitSeller.supported {
                paidExitSellerStatusBadges
                paidExitSellerInternetSummary
            }
        }
    }

    private var paidExitSellerStatusBadges: some View {
        HStack(spacing: 8) {
            badge(state.paidExitSeller.enabled ? "Selling" : "Off", style: state.paidExitSeller.enabled ? .ok : .muted)
            badge(fallbackText(state.paidExitSeller.internetText, paidExitCurrentInternetTitle), style: .muted)
            if !state.paidExitSeller.publicIpText.isEmpty {
                badge("Public IP \(state.paidExitSeller.publicIpText)", style: .muted)
            }
        }
    }

    private var paidExitSellerInternetSummary: some View {
        VStack(alignment: .leading, spacing: 8) {
            paidExitSummaryRow(
                title: "Internet",
                value: fallbackText(state.paidExitSeller.internetText, paidExitCurrentInternetTitle),
                detail: paidExitCurrentInternetDetail,
                systemImage: "network"
            )
            if !state.paidExitSeller.priceText.isEmpty {
                paidExitSummaryRow(
                    title: "Price",
                    value: state.paidExitSeller.priceText,
                    detail: state.paidExitSeller.acceptedMints.first ?? "",
                    systemImage: "creditcard.fill"
                )
            }
            paidExitSummaryRow(
                title: fallbackText(state.paidExitSeller.channelCreditTitleText, "Pending buyer credit"),
                value: fallbackText(
                    state.paidExitSeller.channelCreditText,
                    formatPaidRouteMsat(state.paidExitSeller.channelCreditMsat)
                ),
                detail: fallbackText(
                    state.paidExitSeller.channelCreditHelpText,
                    state.paidExitSeller.channelCreditMsat > 0 ? "Collect to move it into wallet" : ""
                ),
                systemImage: "person.crop.circle.badge.checkmark"
            )
        }
    }

    private func paidExitSummaryRow(
        title: String,
        value: String,
        detail: String,
        systemImage: String
    ) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: systemImage)
                .foregroundStyle(.secondary)
                .frame(width: 16)
            Text(title)
                .font(.caption.weight(.semibold))
                .frame(width: 104, alignment: .leading)
            Text(value)
                .font(.caption)
                .lineLimit(1)
                .truncationMode(.middle)
            if !detail.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }
    }

    private var paidExitSellerListingSettings: some View {
        surface {
            HStack(spacing: 12) {
                sectionHeader("What Buyers See", systemImage: "mappin.and.ellipse")
                Spacer(minLength: 16)
                paidExitSellerSaveButton
            }
            VStack(alignment: .leading, spacing: 10) {
                paidExitFormRow("Country") {
                    HStack(spacing: 8) {
                        TextField("FI", text: $paidExitCountryCode)
                            .frame(width: 70)
                        TextField("Region", text: $paidExitRegion)
                    }
                }
                paidExitFormRow("Connection") {
                    Picker("", selection: $paidExitNetworkClass) {
                        ForEach(["unknown", "datacenter", "residential", "mobile", "satellite", "community_mesh"], id: \.self) { value in
                            Text(paidExitNetworkClassTitle(value)).tag(value)
                        }
                    }
                    .labelsHidden()
                    .frame(maxWidth: 260)
                }
                paidExitFormRow("Works with") {
                    HStack(spacing: 16) {
                        Toggle("IPv4", isOn: $paidExitIpv4)
                        Toggle("IPv6", isOn: $paidExitIpv6)
                    }
                }
            }
            disclosureSection(
                title: "More Details",
                systemImage: "slider.horizontal.3",
                isExpanded: $paidExitListingAdvancedExpanded,
                font: .subheadline.weight(.semibold)
            ) {
                VStack(alignment: .leading, spacing: 10) {
                    paidExitFormRow("ASN") {
                        TextField("12345", text: $paidExitAsn)
                            .frame(width: 120)
                    }
                }
                .padding(.top, 8)
            }
        }
    }

    private var paidExitSellerPaymentSettings: some View {
        surface {
            sectionHeader("Price", systemImage: "creditcard.fill")
            VStack(alignment: .leading, spacing: 10) {
                paidExitFormRow("Meter") {
                    Picker("", selection: $paidExitMeter) {
                        ForEach(["bytes", "milliseconds", "packets"], id: \.self) { meter in
                            Text(paidExitMeterTitle(meter)).tag(meter)
                        }
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .frame(maxWidth: 320)
                }
                paidExitFormRow("Charge") {
                    HStack(spacing: 8) {
                        TextField("msat", text: $paidExitPriceMsat)
                            .frame(width: 130)
                        Text("per")
                            .foregroundStyle(.secondary)
                        paidExitPriceUnitControl
                    }
                }
                paidExitFormRow("Mint") {
                    TextField("https://mint.minibits.cash/Bitcoin", text: $paidExitAcceptedMints)
                }
            }
        }
    }

    @ViewBuilder
    private var paidExitPriceUnitControl: some View {
        if paidExitMeter == "bytes" {
            Picker("", selection: $paidExitPerUnits) {
                ForEach(paidExitBytePriceUnitOptions, id: \.value) { option in
                    Text(option.label).tag(option.value)
                }
            }
            .labelsHidden()
            .frame(width: 150)
        } else {
            TextField("units", text: $paidExitPerUnits)
                .frame(width: 130)
        }
    }

    private var paidExitBytePriceUnitOptions: [(label: String, value: String)] {
        var options: [(label: String, value: String)] = [
            ("100 KB", "100 KB"),
            ("1 MB", "1 MB"),
            ("10 MB", "10 MB"),
            ("100 MB", "100 MB"),
            ("1 GB", "1 GB")
        ]
        let current = paidExitPerUnits.trimmingCharacters(in: .whitespacesAndNewlines)
        if !current.isEmpty && !options.contains(where: { $0.value == current }) {
            let label = UInt64(current)
                .map { paidRouteMeterUnitText($0, meter: paidExitMeter) } ?? current
            options.insert((label, current), at: 0)
        }
        return options
    }

    private var paidExitSellerTermsSettings: some View {
        surface {
            disclosureSection(
                title: "Trial and Limits",
                systemImage: "slider.horizontal.3",
                isExpanded: $paidExitAdvancedTermsExpanded,
                font: .headline
            ) {
                VStack(alignment: .leading, spacing: 10) {
                    paidExitFormRow("Per buyer") {
                        HStack(spacing: 8) {
                            paidExitTermInput("Max balance", "250 sat", text: $paidExitMaxChannelCapacitySat)
                            paidExitTermInput("Channel expires", "1 day", text: $paidExitChannelExpirySecs)
                        }
                    }
                    paidExitFormRow("Free test") {
                        HStack(spacing: 8) {
                            paidExitTermInput("Before payment", paidExitMeter == "bytes" ? "1 MB" : "units", text: $paidExitFreeProbeUnits)
                            paidExitTermInput("After payment runs out", paidExitMeter == "bytes" ? "256 KB" : "units", text: $paidExitGraceUnits)
                        }
                    }
                    if !state.paidExitSeller.settlementText.isEmpty {
                        Text(state.paidExitSeller.settlementText)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.top, 8)
            }
        }
    }

    private func paidExitTermInput(_ title: String, _ placeholder: String, text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            TextField(placeholder, text: text)
                .frame(width: 132)
        }
    }

    private func paidExitFormRow<Content: View>(
        _ title: String,
        @ViewBuilder content: () -> Content
    ) -> some View {
        HStack(alignment: .center, spacing: 12) {
            Text(title)
                .foregroundStyle(.secondary)
                .frame(width: 96, alignment: .leading)
            content()
        }
    }

    private var paidExitSellerActivitySettings: some View {
        surface {
            HStack(spacing: 12) {
                sectionHeader("Customers", systemImage: "person.2.fill")
                Spacer(minLength: 16)
                paidExitSellerAdvertiseButton
                paidExitSellerPaymentsButton
            }
            paidRoutePaymentActionResult(state.paidRouteMarket.lastPaymentAction)
            if !state.paidExitSeller.sessions.isEmpty || !state.paidExitSeller.channels.isEmpty {
                paidExitSellerCustomerSummary
            }
            if state.paidExitSeller.sessions.isEmpty {
                emptyRow("No customers connected", systemImage: "person.2")
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(state.paidExitSeller.sessions, id: \.sessionId) { session in
                        paidExitSellerSessionRow(session)
                    }
                }
            }
        }
    }

    private var paidExitSellerCustomerSummary: some View {
        let sessions = state.paidExitSeller.sessions
        let connected = Int(state.paidExitSeller.currentConnectionCount)
        let behind = sessions.filter { $0.unpaidMsat > 0 }.count
        let collectable = sessions.filter(paidExitSellerSessionCanCollect).count
        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                badge("\(connected) connected", style: connected > 0 ? .ok : .muted)
                badge("\(state.paidExitSeller.pastConnectionCount) past", style: .muted)
                badge("\(state.paidExitSeller.channels.count) open channels", style: .muted)
                if behind > 0 || state.paidExitSeller.totalUnpaidMsat > 0 {
                    badge("\(max(behind, 1)) behind", style: .warn)
                }
                if collectable > 0 {
                    badge("\(collectable) collectable", style: .muted)
                }
            }
            HStack(spacing: 12) {
                Text(fallbackText(state.paidExitSeller.totalTrafficText, "\(formatBytes(state.paidExitSeller.totalBillableBytes)) routed"))
                Text(fallbackText(state.paidExitSeller.totalPaidText, "\(formatPaidRouteMsat(state.paidExitSeller.totalPaidMsat)) paid"))
                Text(fallbackText(state.paidExitSeller.totalDueText, "\(formatPaidRouteMsat(state.paidExitSeller.totalDueMsat)) due"))
                if state.paidExitSeller.totalUnpaidMsat > 0 {
                    Text(fallbackText(state.paidExitSeller.totalUnpaidText, "\(formatPaidRouteMsat(state.paidExitSeller.totalUnpaidMsat)) behind"))
                        .foregroundStyle(Color.orange)
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
    }

    private var paidExitSellerActionButtons: some View {
        HStack(spacing: 8) {
            paidExitSellerSaveButton
            paidExitSellerAdvertiseButton
            paidExitSellerPaymentsButton
        }
    }

    private var paidExitSellerSaveButton: some View {
        Button {
            manager.savePaidExitSellerSettings(
                upstream: paidExitCurrentUpstream,
                meter: paidExitMeter,
                priceMsat: paidExitPriceMsat,
                perUnits: paidExitPerUnits,
                acceptedMints: paidExitAcceptedMints,
                maxChannelCapacitySat: paidExitMaxChannelCapacitySat,
                channelExpirySecs: paidExitChannelExpirySecs,
                freeProbeUnits: paidExitFreeProbeUnits,
                graceUnits: paidExitGraceUnits,
                countryCode: paidExitCountryCode,
                region: paidExitRegion,
                asn: paidExitAsn,
                networkClass: paidExitNetworkClass,
                ipv4: paidExitIpv4,
                ipv6: paidExitIpv6
            )
        } label: {
            Label("Save", systemImage: "checkmark")
        }
        .disabled(manager.actionInFlight)
    }

    private var paidExitSellerAdvertiseButton: some View {
        Button {
            manager.publishPaidExitOffer()
        } label: {
            Label("Advertise", systemImage: "paperplane.fill")
        }
        .disabled(manager.actionInFlight || !state.paidExitSeller.enabled)
    }

    private var paidExitSellerPaymentsButton: some View {
        Button {
            manager.receivePaidRoutePayments()
        } label: {
            Label("Payments", systemImage: "tray.and.arrow.down.fill")
        }
        .disabled(manager.actionInFlight || !state.paidExitSeller.enabled)
    }

    private var paidRouteMarketSettings: some View {
        let market = state.paidRouteMarket
        return VStack(alignment: .leading, spacing: 14) {
            if market.supported && !market.sessions.isEmpty {
                paidRouteActiveSessionSection(market)
                paidRouteOfferDiscoverySection(market)
            } else {
                paidRouteOfferDiscoverySection(market)
            }
        }
    }

    private func paidRouteActiveSessionSection(_ market: NativePaidRouteMarketState) -> some View {
        surface {
            HStack(spacing: 12) {
                sectionHeader("Current Internet", systemImage: "bolt.horizontal.circle.fill")
                Spacer(minLength: 16)
                Button {
                    manager.streamPaidRoutePayments()
                } label: {
                    Label("Pay", systemImage: "arrow.up.right.circle.fill")
                }
                .controlSize(.small)
                .disabled(manager.actionInFlight || !paidRouteHasStreamablePayments(market.sessions))
                .help("Send due payments")
            }
            VStack(alignment: .leading, spacing: 8) {
                ForEach(market.sessions, id: \.sessionId) { session in
                    paidRouteSessionRow(session)
                }
            }
            paidRoutePaymentActionResult(market.lastPaymentAction)
        }
    }

    private func paidRouteOfferDiscoverySection(_ market: NativePaidRouteMarketState) -> some View {
        let visibleOffers = paidRouteVisibleOffers(market)
        return surface {
            HStack(spacing: 12) {
                sectionHeader("Find Internet", systemImage: "cart.fill")
                Spacer(minLength: 16)
                Button {
                    manager.discoverPaidRouteOffers()
                } label: {
                    Label("Find", systemImage: "magnifyingglass")
                }
                .controlSize(.small)
                .disabled(manager.actionInFlight || !market.supported)
            }

            if !market.statusText.isEmpty {
                Text(market.statusText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if market.supported && !market.offers.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        label("Available")
                        Spacer(minLength: 8)
                        Picker("Country", selection: $paidRouteOfferCountryFilter) {
                            Text("Any country").tag("all")
                            ForEach(market.countryOptions, id: \.self) { country in
                                Text(country).tag(country)
                            }
                        }
                        .pickerStyle(.menu)
                        .controlSize(.small)
                        .help("Country")
                        Picker("Class", selection: $paidRouteOfferNetworkFilter) {
                            Text("Any class").tag("all")
                            ForEach(market.networkClassOptions, id: \.self) { networkClass in
                                Text(paidExitNetworkClassTitle(networkClass)).tag(networkClass)
                            }
                        }
                        .pickerStyle(.menu)
                        .controlSize(.small)
                        .help("Network class")
                        Picker("Sort", selection: $paidRouteOfferSort) {
                            Text("Quality").tag("quality")
                            Text("Price").tag("price")
                            Text("Newest").tag("newest")
                        }
                        .pickerStyle(.segmented)
                        .controlSize(.small)
                        .frame(width: 210)
                    }
                    .onChange(of: paidRouteOfferCountryFilter) { _, _ in
                        applyPaidRouteMarketFilter()
                    }
                    .onChange(of: paidRouteOfferNetworkFilter) { _, _ in
                        applyPaidRouteMarketFilter()
                    }
                    .onChange(of: paidRouteOfferSort) { _, _ in
                        applyPaidRouteMarketFilter()
                    }
                    if visibleOffers.isEmpty {
                        Text("No matching offers")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(visibleOffers, id: \.key) { offer in
                            paidRouteOfferRow(offer)
                        }
                    }
                }
            }
        }
    }

    private var paidRouteWalletSettings: some View {
        let market = state.paidRouteMarket
        return VStack(alignment: .leading, spacing: 14) {
            if market.supported {
                paidRouteWalletSection(market.wallet)
            } else {
                surface {
                    sectionHeader("Cashu Wallet", systemImage: "creditcard.fill")
                    if !market.statusText.isEmpty {
                        Text(market.statusText)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    private func paidRouteWalletSection(_ wallet: NativePaidRouteWalletState) -> some View {
        surface {
            HStack(spacing: 10) {
                sectionHeader("Cashu Wallet", systemImage: "creditcard.fill")
                if wallet.balanceKnown {
                    Text(fallbackText(wallet.totalBalanceText, formatPaidRouteMsat(wallet.totalBalanceMsat)))
                        .font(.caption.weight(.medium))
                }
                if !wallet.defaultMint.isEmpty {
                    Text(wallet.defaultMint)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Spacer(minLength: 12)
                Button {
                    manager.refreshPaidRouteWallet()
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .help("Refresh wallet")
                .disabled(manager.actionInFlight)
            }

            VStack(spacing: 6) {
                HStack(spacing: 8) {
                    TextField("Mint URL", text: $paidRouteMintUrl)
                    TextField("Label", text: $paidRouteMintLabel)
                        .frame(width: 120)
                    Button {
                        manager.addPaidRouteWalletMint(url: paidRouteMintUrl, label: paidRouteMintLabel)
                    } label: {
                        Label("Add", systemImage: "plus.circle.fill")
                    }
                    .disabled(manager.actionInFlight)
                }

                HStack(spacing: 8) {
                    TextField("Top up sats", text: $paidRouteTopupAmount)
                        .frame(width: 110)
                    Button {
                        manager.topUpPaidRouteWallet(mintUrl: nil, amountSat: paidRouteTopupAmount)
                    } label: {
                        Label("Top Up", systemImage: "arrow.down.circle.fill")
                    }
                    .disabled(manager.actionInFlight || parsePositiveUInt64(paidRouteTopupAmount) == nil)

                    TextField("Send sats", text: $paidRouteSendAmount)
                        .frame(width: 105)
                    Button {
                        manager.sendPaidRouteWalletToken(mintUrl: nil, amountSat: paidRouteSendAmount)
                    } label: {
                        Label("Export", systemImage: "paperplane.fill")
                    }
                    .disabled(manager.actionInFlight || parsePositiveUInt64(paidRouteSendAmount) == nil)
                }

                HStack(spacing: 8) {
                    TextField("Cashu token", text: $paidRouteReceiveToken)
                    Button {
                        manager.receivePaidRouteWalletToken(paidRouteReceiveToken)
                    } label: {
                        Label("Receive", systemImage: "tray.and.arrow.down.fill")
                    }
                    .disabled(manager.actionInFlight || paidRouteReceiveToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }

                HStack(spacing: 8) {
                    TextField("Lightning invoice", text: $paidRouteWithdrawInvoice)
                    Button {
                        manager.withdrawPaidRouteWalletLightning(mintUrl: nil, invoice: paidRouteWithdrawInvoice)
                    } label: {
                        Label("Withdraw", systemImage: "bolt.fill")
                    }
                    .disabled(manager.actionInFlight || paidRouteWithdrawInvoice.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }

            if wallet.mints.isEmpty {
                emptyRow("No wallet mints", systemImage: "creditcard")
            } else {
                VStack(spacing: 6) {
                    ForEach(wallet.mints, id: \.url) { mint in
                        paidRouteMintRow(mint)
                    }
                }
            }

            paidRouteWalletActionResult(wallet.lastAction)
        }
    }

    @ViewBuilder
    private func paidRouteWalletActionResult(_ action: NativePaidRouteWalletActionState) -> some View {
        if !action.kind.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Image(systemName: paidRouteWalletActionIcon(action.kind))
                        .foregroundStyle(.secondary)
                    Text(action.statusText.isEmpty ? paidRouteWalletActionTitle(action.kind) : action.statusText)
                        .font(.caption.weight(.medium))
                    if action.amountSat > 0 {
                        Text(fallbackText(action.amountText, "\(action.amountSat) sat"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    if action.feeSat > 0 {
                        Text(fallbackText(action.feeText, "\(action.feeSat) sat fee"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                if !action.paymentRequest.isEmpty {
                    paidRouteWalletOutputRow("Invoice", value: action.paymentRequest, copied: .paymentRequest)
                }
                if !action.token.isEmpty {
                    paidRouteWalletOutputRow("Token", value: action.token, copied: .cashuToken)
                }
                if !action.preimage.isEmpty {
                    paidRouteWalletOutputRow("Preimage", value: action.preimage, copied: .lightningPreimage)
                }
            }
        }
    }

    private func paidRouteWalletOutputRow(_ title: String, value: String, copied: CopyValue) -> some View {
        HStack(spacing: 8) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 56, alignment: .leading)
            Text(value)
                .font(.caption)
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
            Spacer(minLength: 8)
            copyButton(value: value, copied: copied, systemImage: "doc.on.doc")
        }
    }

    @ViewBuilder
    private func paidRoutePaymentActionResult(_ action: NativePaidRoutePaymentActionState) -> some View {
        if !action.kind.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Image(systemName: paidRoutePaymentActionIcon(action.kind))
                        .foregroundStyle(.secondary)
                    Text(action.statusText.isEmpty ? paidRoutePaymentActionTitle(action.kind) : action.statusText)
                        .font(.caption.weight(.medium))
                    if action.paidMsat > 0 {
                        Text(fallbackText(action.paidText, "\(formatPaidRouteMsat(action.paidMsat)) paid"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    if action.unpaidMsat > 0 {
                        Text(fallbackText(action.unpaidText, "\(formatPaidRouteMsat(action.unpaidMsat)) behind"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                if !action.envelopeJson.isEmpty {
                    paidRouteWalletOutputRow("Payment", value: action.envelopeJson, copied: .paymentEnvelope)
                }
            }
        }
    }

    private func paidRouteMintRow(_ mint: NativePaidRouteWalletMintState) -> some View {
        HStack(spacing: 10) {
            Button {
                manager.setPaidRouteDefaultMint(mint.url)
            } label: {
                Image(systemName: mint.isDefault ? "star.fill" : "star")
                    .foregroundStyle(mint.isDefault ? .yellow : .secondary)
            }
            .buttonStyle(.plain)
            .disabled(manager.actionInFlight || mint.isDefault)

            VStack(alignment: .leading, spacing: 2) {
                Text(mint.label.isEmpty ? mint.url : mint.label)
                    .lineLimit(1)
                Text(mint.url)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 12)
            if mint.balanceKnown {
                Text(fallbackText(mint.balanceText, formatPaidRouteMsat(mint.balanceMsat)))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Button {
                manager.removePaidRouteWalletMint(mint.url)
            } label: {
                Image(systemName: "trash")
            }
            .buttonStyle(.plain)
            .disabled(manager.actionInFlight)
        }
    }

    private func paidRouteOfferRow(_ offer: NativePaidRouteOfferState) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Image(systemName: "network")
                    .foregroundStyle(.secondary)
                Text(paidRouteOfferTitle(offer))
                    .fontWeight(.medium)
                    .lineLimit(1)
                Spacer(minLength: 12)
                Text(offer.priceText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if paidRouteOfferHasBuyerChannel(offer) {
                    Label("Ready", systemImage: "checkmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.green)
                } else {
                    Button {
                        manager.buyPaidRouteOffer(offer)
                    } label: {
                        Label("Buy", systemImage: "cart.fill")
                    }
                    .controlSize(.small)
                    .disabled(manager.actionInFlight)
                }
            }
            HStack(spacing: 10) {
                Text(offer.statusText)
                let metricText = paidRouteMetricText(
                    fallbackText(
                        offer.qualityText,
                        paidRouteQualityText(
                            latencyMs: offer.latencyMs,
                            jitterMs: offer.jitterMs,
                            packetLossPpm: offer.packetLossPpm
                        )
                    ),
                    offer.bandwidthText
                )
                if !metricText.isEmpty {
                    Text(metricText)
                }
                Text(paidRouteIpText(ipv4: offer.ipv4, ipv6: offer.ipv6))
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
    }

    private func paidRouteOfferHasBuyerChannel(_ offer: NativePaidRouteOfferState) -> Bool {
        state.paidRouteMarket.channels.contains { channel in
            channel.offerId == offer.offerId
                && channel.counterpartyNpub == offer.sellerNpub
                && channel.role == "buyer"
                && ["opening", "probing", "active", "paused"].contains(channel.status)
                && state.paidRouteMarket.sessions.contains { session in
                    session.channelId == channel.channelId && session.allowRouting
                }
        }
    }

    private func paidRouteSessionIsSelected(_ session: NativePaidRouteSessionState) -> Bool {
        let seller = paidRouteSessionSellerNpub(session)
        return !seller.isEmpty && !state.wireguardExitEnabled && state.exitNode == seller
    }

    private func paidRouteSessionSellerNpub(_ session: NativePaidRouteSessionState) -> String {
        state.paidRouteMarket.channels.first { channel in
            channel.channelId == session.channelId && channel.role == "buyer"
        }?.counterpartyNpub ?? ""
    }

    private func paidRouteMarketChannel(for session: NativePaidRouteSessionState) -> NativePaidRouteChannelState? {
        state.paidRouteMarket.channels.first { $0.channelId == session.channelId }
    }

    private func paidRouteSessionLiveMetaText(
        _ session: NativePaidRouteSessionState,
        channel: NativePaidRouteChannelState?,
        counterpartyLabel: String
    ) -> String {
        var parts: [String] = []
        if let channel {
            if !channel.counterpartyNpub.isEmpty {
                parts.append("\(counterpartyLabel) \(paidRouteShortIdentifier(channel.counterpartyNpub))")
            }
            if !channel.status.isEmpty {
                parts.append("channel \(paidRoutePlainStatus(channel.status, fallback: channel.status).lowercased())")
            }
            if !channel.capacityText.isEmpty {
                parts.append("\(channel.capacityText) capacity")
            }
        } else if !session.channelId.isEmpty {
            parts.append("channel \(paidRouteShortIdentifier(session.channelId))")
        }
        if !session.accessState.isEmpty {
            parts.append(paidRouteAccessTitle(session.accessState, fallback: session.lifecycleStatus))
        }
        if session.updatedAtUnix > 0 {
            parts.append("updated \(paidRouteRelativePastText(session.updatedAtUnix))")
        }
        if session.expiresAtUnix > 0 {
            parts.append(paidRouteExpiryText(session.expiresAtUnix))
        }
        return parts.joined(separator: " · ")
    }

    private func paidRouteSessionRow(_ session: NativePaidRouteSessionState) -> some View {
        let selected = paidRouteSessionIsSelected(session)
        let channel = paidRouteMarketChannel(for: session)
        let metricText = paidRouteMetricText(
            fallbackText(
                session.qualityText,
                paidRouteQualityText(
                    latencyMs: session.latencyMs,
                    jitterMs: session.jitterMs,
                    packetLossPpm: session.packetLossPpm
                )
            ),
            session.bandwidthText
        )
        return VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Image(systemName: session.allowRouting ? "bolt.horizontal.circle.fill" : "pause.circle.fill")
                    .foregroundStyle(session.allowRouting ? .green : .orange)
                Text(paidRouteBuyerSessionTitle(session, selected: selected))
                    .fontWeight(.medium)
                Spacer(minLength: 12)
                if selected {
                    Button {
                        manager.selectDirectExit()
                    } label: {
                        Label("Stop", systemImage: "stop.circle.fill")
                    }
                    .controlSize(.small)
                    .disabled(manager.actionInFlight)
                    .help("Stop using this seller")
                } else {
                    Button {
                        manager.usePaidRouteSession(session)
                    } label: {
                        Label("Connect", systemImage: "arrow.right.circle.fill")
                    }
                    .controlSize(.small)
                    .disabled(manager.actionInFlight || !session.allowRouting)
                    .help("Use this seller")
                }
                Button {
                    manager.probePaidRouteSession(session)
                } label: {
                    Image(systemName: "speedometer")
                }
                .buttonStyle(.borderless)
                .disabled(manager.actionInFlight)
                .help("Probe exit quality")
                if paidRouteSessionCanOpenChannel(session) {
                    Button {
                        manager.openPaidRouteChannelFromWallet(session)
                    } label: {
                        Image(systemName: "creditcard.fill")
                    }
                    .buttonStyle(.borderless)
                    .disabled(manager.actionInFlight || state.paidRouteMarket.wallet.defaultMint.isEmpty)
                    .help("Fund this exit")
                }
                if paidRouteSessionCanSignPayment(session) {
                    Button {
                        manager.signPaidRoutePaymentEnvelopeFromWallet(session)
                    } label: {
                        Image(systemName: "arrow.up.forward.circle.fill")
                    }
                    .buttonStyle(.borderless)
                    .disabled(manager.actionInFlight)
                    .help("Pay due usage")
                }
                if !selected && paidRouteSessionCanCloseChannel(session) {
                    Button {
                        manager.closePaidRouteChannelFromWallet(session)
                    } label: {
                        Label("Close", systemImage: "checkmark.seal.fill")
                    }
                    .controlSize(.small)
                    .disabled(manager.actionInFlight)
                    .help("Close and settle channel")
                }
                if paidRouteSessionHasSendableEnvelope(session) {
                    Button {
                        manager.sendPaidRoutePaymentEnvelope(state.paidRouteMarket.lastPaymentAction.envelopeJson)
                    } label: {
                        Image(systemName: "paperplane.fill")
                    }
                    .buttonStyle(.borderless)
                    .disabled(manager.actionInFlight)
                    .help("Send payment")
                }
                Text(fallbackText(session.paidText, "\(formatPaidRouteMsat(session.paidMsat)) paid"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 10) {
                Text(fallbackText(session.usageText, paidRouteUsageText(session)))
                Text(fallbackText(session.amountDueText, "\(formatPaidRouteMsat(session.amountDueMsat)) due"))
                if session.unpaidMsat > 0 {
                    Text(fallbackText(session.unpaidText, "\(formatPaidRouteMsat(session.unpaidMsat)) behind"))
                }
                if !session.locationText.isEmpty {
                    Text(session.locationText)
                        .foregroundStyle(session.countryClaimStatus == "mismatch" ? Color.orange : Color.secondary)
                } else {
                    if !session.realizedExitIp.isEmpty {
                        Text(session.realizedExitIp)
                    }
                    if !session.observedCountryCode.isEmpty {
                        Text(session.observedCountryCode)
                    }
                    if let countryClaimText = paidRouteCountryClaimText(session) {
                        Text(countryClaimText)
                            .foregroundStyle(session.countryClaimStatus == "mismatch" ? Color.orange : Color.green)
                    }
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            Text(paidRouteSessionLiveMetaText(session, channel: channel, counterpartyLabel: "seller"))
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            if !metricText.isEmpty {
                Text(metricText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            if !session.settlementText.isEmpty {
                Text(session.settlementText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private func paidRouteSessionCanOpenChannel(_ session: NativePaidRouteSessionState) -> Bool {
        !session.paymentChannelReady
            && ["free_probe", "grace", "suspended"].contains(session.accessState)
            && ["opening", "probing", "active", "paused"].contains(session.lifecycleStatus)
    }

    private func paidRouteSessionCanSignPayment(_ session: NativePaidRouteSessionState) -> Bool {
        session.paymentChannelReady
            && session.amountDueMsat > session.paidMsat
            && ["grace", "suspended"].contains(session.accessState)
            && ["probing", "active", "paused"].contains(session.lifecycleStatus)
    }

    private func paidRouteSessionCanCloseChannel(_ session: NativePaidRouteSessionState) -> Bool {
        session.paymentChannelReady
            && !session.sessionId.isEmpty
            && ["closed", "expired"].contains(session.lifecycleStatus) == false
    }

    private func paidRouteHasStreamablePayments(_ sessions: [NativePaidRouteSessionState]) -> Bool {
        sessions.contains { paidRouteSessionCanSignPayment($0) }
    }

    private func paidRouteSessionHasSendableEnvelope(_ session: NativePaidRouteSessionState) -> Bool {
        let action = state.paidRouteMarket.lastPaymentAction
        return ["create", "open_channel", "sign", "close"].contains(action.kind)
            && action.sessionId == session.sessionId
            && !action.envelopeJson.isEmpty
    }

    private func paidRouteCountryClaimText(_ session: NativePaidRouteSessionState) -> String? {
        switch session.countryClaimStatus {
        case "match":
            return session.claimedCountryCode.isEmpty ? nil : "verified"
        case "mismatch":
            return session.claimedCountryCode.isEmpty ? "country mismatch" : "claimed \(session.claimedCountryCode)"
        default:
            return nil
        }
    }

    private func paidRouteBuyerSessionTitle(_ session: NativePaidRouteSessionState, selected: Bool) -> String {
        if selected {
            return session.allowRouting ? "Connected" : "Selected"
        }
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
        return paidRoutePlainStatus(session.statusText, fallback: session.lifecycleStatus)
    }

    private func paidPublicExitSubtitle(_ session: NativePaidRouteSessionState) -> String {
        if !session.observedCountryCode.isEmpty && !session.realizedExitIp.isEmpty {
            return "\(session.observedCountryCode), \(session.realizedExitIp)"
        }
        if !session.statusText.isEmpty {
            return session.statusText
        }
        return "Paid internet seller"
    }

    private func paidExitSellerSessionRow(_ session: NativePaidRouteSessionState) -> some View {
        let channel = paidExitSellerChannel(for: session)
        let metricText = paidRouteMetricText(
            fallbackText(
                session.qualityText,
                paidRouteQualityText(
                    latencyMs: session.latencyMs,
                    jitterMs: session.jitterMs,
                    packetLossPpm: session.packetLossPpm
                )
            ),
            session.bandwidthText
        )
        return VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Image(systemName: session.allowRouting ? "bolt.horizontal.circle.fill" : "pause.circle.fill")
                    .foregroundStyle(session.allowRouting ? .green : .orange)
                Text(paidExitSellerSessionTitle(session))
                    .fontWeight(.medium)
                Spacer(minLength: 12)
                if paidExitSellerSessionCanCollect(session) {
                    Button {
                        manager.collectPaidExitChannel(session)
                    } label: {
                        Label(paidExitSellerCollectButtonTitle(session), systemImage: paidExitSellerCollectButtonIcon(session))
                    }
                    .controlSize(.small)
                    .disabled(manager.actionInFlight || !state.paidExitSeller.enabled)
                    .help(paidExitSellerCollectButtonHelp(session))
                }
                Text(fallbackText(session.paidText, "\(formatPaidRouteMsat(session.paidMsat)) paid"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 10) {
                Text(fallbackText(session.usageText, paidRouteUsageText(session)))
                Text(fallbackText(session.amountDueText, "\(formatPaidRouteMsat(session.amountDueMsat)) due"))
                if session.unpaidMsat > 0 {
                    Text(fallbackText(session.unpaidText, "\(formatPaidRouteMsat(session.unpaidMsat)) behind"))
                }
                if session.packets > 0 {
                    Text("\(session.packets) packets")
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            Text(paidRouteSessionLiveMetaText(session, channel: channel, counterpartyLabel: "buyer"))
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            if !metricText.isEmpty {
                Text(metricText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            if !session.settlementText.isEmpty {
                Text(session.settlementText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private func paidExitSellerChannel(for session: NativePaidRouteSessionState) -> NativePaidRouteChannelState? {
        state.paidExitSeller.channels.first { $0.channelId == session.channelId }
    }

    private func paidRouteShortIdentifier(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > 18 else {
            return trimmed
        }
        return "\(trimmed.prefix(10))…\(trimmed.suffix(6))"
    }

    private func paidRouteRelativePastText(_ unix: UInt64) -> String {
        let now = UInt64(Date().timeIntervalSince1970)
        guard unix < now else {
            return "just now"
        }
        return "\(formatCompactDurationSeconds(now - unix)) ago"
    }

    private func paidRouteExpiryText(_ unix: UInt64) -> String {
        let now = UInt64(Date().timeIntervalSince1970)
        if unix >= now {
            return "ends in \(formatCompactDurationSeconds(unix - now))"
        }
        return "ended \(formatCompactDurationSeconds(now - unix)) ago"
    }

    private func paidExitSellerSessionCanCollect(_ session: NativePaidRouteSessionState) -> Bool {
        session.paymentChannelReady
            && session.paidMsat > 0
            && !session.channelId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func paidExitSellerCollectButtonTitle(_ session: NativePaidRouteSessionState) -> String {
        fallbackText(session.collectActionText, session.allowRouting ? "End & Collect" : "Collect")
    }

    private func paidExitSellerCollectButtonIcon(_ session: NativePaidRouteSessionState) -> String {
        session.allowRouting ? "stop.circle.fill" : "checkmark.seal.fill"
    }

    private func paidExitSellerCollectButtonHelp(_ session: NativePaidRouteSessionState) -> String {
        fallbackText(session.collectActionHelpText, session.allowRouting ? "Stop routing and move paid channel funds to wallet" : "Move paid channel funds to wallet")
    }

    private func paidExitSellerSessionTitle(_ session: NativePaidRouteSessionState) -> String {
        if !session.titleText.isEmpty {
            return session.titleText
        }
        if session.allowRouting {
            return "Buyer online"
        }
        if session.unpaidMsat > 0 {
            return "Waiting for payment"
        }
        return session.statusText.isEmpty ? "Buyer session" : session.statusText
    }

    private func paidExitMeterTitle(_ value: String) -> String {
        switch value {
        case "milliseconds": return "Time"
        case "packets": return "Packets"
        default: return "Bytes"
        }
    }

    private func paidExitNetworkClassTitle(_ value: String) -> String {
        switch value {
        case "datacenter": return "Datacenter"
        case "residential": return "Residential"
        case "mobile": return "Mobile"
        case "satellite": return "Satellite"
        case "community_mesh": return "Community mesh"
        default: return "Unknown"
        }
    }

    private func paidExitPrivateAccessTitle(_ value: String) -> String {
        value == "denied" ? "Denied" : value
    }

    private func paidRouteOfferTitle(_ offer: NativePaidRouteOfferState) -> String {
        let country = offer.countryCode.isEmpty ? "Unknown country" : offer.countryCode
        let network = paidExitNetworkClassTitle(offer.networkClass)
        return "\(country) - \(network)"
    }

    private func paidRouteVisibleOffers(_ market: NativePaidRouteMarketState) -> [NativePaidRouteOfferState] {
        if market.hiddenOfferCount > 0 || !market.visibleOffers.isEmpty {
            return market.visibleOffers
        }
        return market.offers
    }

    private func applyPaidRouteMarketFilter() {
        manager.setPaidRouteMarketFilter(
            countryCode: paidRouteOfferCountryFilter,
            networkClass: paidRouteOfferNetworkFilter,
            sort: paidRouteOfferSort
        )
    }

    private func paidRouteWalletActionTitle(_ kind: String) -> String {
        switch kind {
        case "topup": return "Top up"
        case "receive": return "Received"
        case "send": return "Token ready"
        case "withdraw": return "Withdrawn"
        case "refresh": return "Wallet refreshed"
        case "open_channel": return "Exit funded"
        case "close": return "Channel closed"
        default: return kind
        }
    }

    private func paidRouteWalletActionIcon(_ kind: String) -> String {
        switch kind {
        case "topup": return "arrow.down.circle.fill"
        case "receive": return "tray.and.arrow.down.fill"
        case "send": return "paperplane.fill"
        case "withdraw": return "bolt.fill"
        case "open_channel": return "creditcard.fill"
        default: return "creditcard.fill"
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
        case "close": return "Channel closed"
        case "collect": return "Collected"
        case "stream": return "Payments sent"
        case "probe": return "Quality checked"
        default:
            return kind.isEmpty ? "Payment" : kind.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    private func paidRoutePaymentActionIcon(_ kind: String) -> String {
        switch kind {
        case "send": return "paperplane.fill"
        case "receive": return "tray.and.arrow.down.fill"
        case "apply": return "checkmark.seal.fill"
        case "create": return "doc.badge.plus"
        case "open_channel": return "creditcard.fill"
        case "sign": return "arrow.up.forward.circle.fill"
        case "close": return "checkmark.seal.fill"
        case "collect": return "checkmark.seal.fill"
        case "stream": return "arrow.up.right.circle.fill"
        case "probe": return "speedometer"
        default: return "creditcard.fill"
        }
    }

    private func paidRouteIpText(ipv4: Bool, ipv6: Bool) -> String {
        switch (ipv4, ipv6) {
        case (true, true): return "IPv4/IPv6"
        case (true, false): return "IPv4"
        case (false, true): return "IPv6"
        default: return "IP unknown"
        }
    }

    private func paidRouteQualityText(latencyMs: UInt32, jitterMs: UInt32, packetLossPpm: UInt32) -> String {
        if latencyMs == 0, jitterMs == 0, packetLossPpm == 0 {
            return "Quality unmeasured"
        }
        let lossPercent = Double(packetLossPpm) / 10_000.0
        return "\(latencyMs) ms · \(jitterMs) ms jitter · \(String(format: "%.2f", lossPercent))% loss"
    }

    private func paidRouteMetricText(_ qualityText: String, _ bandwidthText: String) -> String {
        [qualityText, bandwidthText]
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && $0 != "Quality unmeasured" }
            .joined(separator: " · ")
    }

    private func paidRouteSessionDetail(_ session: NativePaidRouteSessionState) -> String {
        if !session.detailText.isEmpty {
            return session.detailText
        }
        let access = paidRouteAccessTitle(session.accessState, fallback: session.lifecycleStatus)
        let used = paidRouteUsageText(session)
        return "\(access), \(used), \(formatPaidRouteMsat(session.amountDueMsat)) due"
    }

    private func paidRouteUsageText(_ session: NativePaidRouteSessionState) -> String {
        if !session.usageText.isEmpty {
            return session.usageText
        }
        if session.bytes > 0 {
            return "\(formatBytes(session.bytes)) used"
        }
        if session.packets > 0 {
            return "\(session.packets) packets"
        }
        return "\(session.deliveredUnits) units"
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
            return raw
                .replacingOccurrences(of: "_", with: " ")
                .capitalized
        }
    }

    private func formatPaidRouteMsat(_ msat: UInt64) -> String {
        if msat >= 1_000 {
            let sats = Double(msat) / 1_000.0
            return "\(String(format: "%.3f", sats)) sat"
        }
        return "\(msat) msat"
    }

    private func fallbackText(_ value: String, _ fallback: String) -> String {
        value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? fallback : value
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

    private func paidExitTrafficUnitDraft(_ units: UInt64, meter: String) -> String {
        switch meter {
        case "bytes":
            return formatBinaryBytesCompact(units)
        default:
            return String(units)
        }
    }

    private func paidExitPricingUnitDraft(_ units: UInt64, meter: String) -> String {
        switch meter {
        case "bytes":
            return formatDecimalBytes(units)
        default:
            return String(units)
        }
    }

    private func paidExitDurationDraft(_ seconds: UInt64) -> String {
        if seconds == 0 {
            return "0 sec"
        }
        if seconds % 86_400 == 0 {
            let days = seconds / 86_400
            return days == 1 ? "1 day" : "\(days) days"
        }
        if seconds % 3_600 == 0 {
            let hours = seconds / 3_600
            return hours == 1 ? "1 hour" : "\(hours) hours"
        }
        if seconds % 60 == 0 {
            let minutes = seconds / 60
            return minutes == 1 ? "1 min" : "\(minutes) min"
        }
        return "\(seconds) sec"
    }

    private func formatCompactDurationSeconds(_ seconds: UInt64) -> String {
        if seconds < 60 {
            return "\(seconds)s"
        }
        let minutes = seconds / 60
        if minutes < 60 {
            return "\(minutes)m"
        }
        let hours = minutes / 60
        if hours < 48 {
            return "\(hours)h"
        }
        let days = hours / 24
        return "\(days)d"
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

    private func formatBinaryBytesCompact(_ bytes: UInt64) -> String {
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

    private func parsePositiveUInt64(_ value: String) -> UInt64? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = UInt64(trimmed), parsed > 0 else {
            return nil
        }
        return parsed
    }

    private func routeChoice(
        title: String,
        subtitle: String,
        selected: Bool,
        enabled: Bool,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack {
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(selected ? .green : .secondary)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .foregroundStyle(.primary)
                    Text(subtitle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
        .disabled(!enabled || manager.actionInFlight)
        .opacity(enabled ? 1 : 0.55)
    }

    private var settingsSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            deviceSettings
            generalSettings
            fipsSettings
            publicFipsSettings
            pubsubSettings
            relaySettings
            networkSettings
            systemSettings
            diagnosticsSection
        }
    }

    private var relaySettings: some View {
        surface {
            sectionHeader("Relays", systemImage: "dot.radiowaves.left.and.right")
            HStack(spacing: 8) {
                TextField("wss://relay.example.com", text: $relayInput)
                    .textFieldStyle(.roundedBorder)
                    .disableAutocorrection(true)
                    .onSubmit { addRelayFromInput() }
                Button {
                    addRelayFromInput()
                } label: {
                    Label("Add", systemImage: "plus")
                }
                .disabled(relayInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || manager.actionInFlight)
            }
            VStack(alignment: .leading, spacing: 6) {
                ForEach(state.relays, id: \.url) { relay in
                    relayRow(relay)
                }
            }
        }
    }

    private func relayRow(_ relay: NativeRelayState) -> some View {
        HStack(spacing: 8) {
            Circle()
                .fill(relay.status == "connected" ? Color.green : Color.secondary.opacity(0.65))
                .frame(width: 9, height: 9)
            Text(relay.url)
                .lineLimit(1)
                .truncationMode(.middle)
                .foregroundStyle(relay.enabled ? .primary : .secondary)
            Spacer(minLength: 8)
            Toggle("", isOn: Binding(
                get: { relay.enabled },
                set: { manager.setRelay(relay.url, enabled: $0) }
            ))
            .labelsHidden()
            .disabled(manager.actionInFlight)
            Button(role: .destructive) {
                manager.deleteRelay(relay.url)
            } label: {
                Image(systemName: "trash")
            }
            .buttonStyle(.borderless)
            .disabled(manager.actionInFlight)
        }
        .padding(.vertical, 3)
    }

    private func addRelayFromInput() {
        if manager.addRelay(relayInput) {
            relayInput = ""
        }
    }

    private var deviceSettings: some View {
        surface {
            sectionHeader("This Device", systemImage: "macbook")
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                GridRow {
                    label("Name")
                    TextField("Name", text: $nodeName)
                }
                GridRow {
                    label("Tunnel IP")
                    TextField("Tunnel IP", text: $tunnelIp)
                }
            }
            Button {
                manager.saveDeviceSettings(
                    nodeName: nodeName,
                    endpoint: endpoint,
                    tunnelIp: tunnelIp,
                    listenPort: listenPort
                )
            } label: {
                Label("Save", systemImage: "checkmark")
            }
            .disabled(manager.actionInFlight)
        }
    }

    private var generalSettings: some View {
        surface {
            sectionHeader("General", systemImage: "gearshape.fill")
            VStack(alignment: .leading, spacing: 8) {
                settingsToggleRow("Start VPN automatically", isOn: Binding(
                    get: { state.autoconnect },
                    set: { manager.setAutoconnect($0) }
                ))
                settingsToggleRow("Launch on startup", isOn: Binding(
                    get: { state.launchOnStartup },
                    set: { manager.setLaunchOnStartup($0) }
                ), disabled: !state.startupSettingsSupported)
                settingsToggleRow("Menu bar on close", isOn: Binding(
                    get: { state.closeToTrayOnClose },
                    set: { manager.setCloseToTray($0) }
                ), disabled: !state.trayBehaviorSupported)
                settingsToggleRow("Block internet if selected source disconnects", isOn: Binding(
                    get: { state.exitNodeLeakProtection },
                    set: { manager.setExitNodeLeakProtection($0) }
                ), disabled: manager.actionInFlight)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var publicFipsSettings: some View {
        surface {
            sectionHeader("Public .fips Addresses", systemImage: "network")
            VStack(alignment: .leading, spacing: 8) {
                Text("Public .fips addresses let hosts reach each other with end-to-end encryption, without static IPs, TLS certificates, or NAT port forwarding.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let url = URL(string: "https://learn.fips.network/") {
                    Link("Learn FIPS", destination: url)
                        .font(.caption)
                }
                settingsToggleRow("Let this Mac serve its public .fips address", isOn: Binding(
                    get: { state.fipsHostTunnelEnabled },
                    set: { manager.setFipsHostTunnel($0) }
                ))
                VStack(alignment: .leading, spacing: 8) {
                    detailValueRow("Your public FIPS address", publicFipsAddress)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Public .fips inbound TCP ports")
                            .foregroundStyle(.secondary)
                        TextField("", text: $fipsHostInboundTcpPorts)
                    }
                    Button {
                        manager.saveFipsHostInboundTcpPorts(fipsHostInboundTcpPorts)
                    } label: {
                        Label("Save", systemImage: "checkmark")
                    }
                    .disabled(manager.actionInFlight)
                }
                .disabled(!state.fipsHostTunnelEnabled)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var publicFipsAddress: String {
        state.ownNpub.isEmpty ? "" : "\(state.ownNpub).fips"
    }

    private var fipsSettings: some View {
        surface {
            sectionHeader("FIPS", systemImage: "shield.fill")
            VStack(alignment: .leading, spacing: 8) {
                settingsToggleRow("Connect to non-roster FIPS peers", isOn: Binding(
                    get: { state.connectToNonRosterFipsPeers },
                    set: { manager.setConnectToNonRosterFipsPeers($0) }
                ))
                settingsToggleRow("Find peers over Nostr relays", isOn: Binding(
                    get: { state.fipsNostrDiscoveryEnabled },
                    set: { manager.setFipsNostrDiscoveryEnabled($0) }
                ))
                settingsToggleRow("Use bootstrap servers", isOn: Binding(
                    get: { state.fipsBootstrapEnabled },
                    set: { manager.setFipsBootstrapEnabled($0) }
                ))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var pubsubSettings: some View {
        surface {
            sectionHeader("Nostr Pubsub", systemImage: "point.3.connected.trianglepath.dotted")
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                GridRow {
                    label("Mode")
                    Picker("", selection: $nostrPubsubMode) {
                        Text("Off").tag("off")
                        Text("Client").tag("client")
                        Text("Relay").tag("relay")
                    }
                    .pickerStyle(.segmented)
                }
                GridRow {
                    label("Fanout")
                    TextField("4", text: $nostrPubsubFanout)
                }
                GridRow {
                    label("Hops")
                    TextField("2", text: $nostrPubsubMaxHops)
                }
                GridRow {
                    label("Max event bytes")
                    TextField("65536", text: $nostrPubsubMaxEventBytes)
                }
            }
            Button {
                manager.saveNostrPubsubSettings(
                    mode: nostrPubsubMode,
                    fanout: nostrPubsubFanout,
                    maxHops: nostrPubsubMaxHops,
                    maxEventBytes: nostrPubsubMaxEventBytes
                )
            } label: {
                Label("Save", systemImage: "checkmark")
            }
            .disabled(manager.actionInFlight)
        }
    }

    private func settingsToggleRow(_ title: String, isOn: Binding<Bool>, disabled: Bool = false) -> some View {
        HStack(spacing: 12) {
            Text(title)
            Spacer(minLength: 16)
            Toggle("", isOn: isOn)
                .labelsHidden()
                .toggleStyle(.switch)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .disabled(disabled)
    }

    private var wireGuardExitSettings: some View {
        surface {
            sectionHeader("WireGuard Upstream", systemImage: "network")
            wireGuardUpstreamEditor
        }
    }

    private var wireGuardUpstreamSettings: some View {
        surface {
            disclosureSection(
                title: "Upstream VPN",
                systemImage: "network",
                isExpanded: $wireGuardUpstreamExpanded,
                font: .headline
            ) {
                wireGuardUpstreamEditor
                    .padding(.top, 8)
            }
        }
    }

    private var wireGuardUpstreamEditor: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Optional: paste a WireGuard config if you use a separate VPN provider.")
                .font(.caption)
                .foregroundStyle(.secondary)

            TextEditor(text: $wireguardExitConfig)
                .font(.system(.body, design: .monospaced))
                .frame(minHeight: 160)
                .padding(6)
                .background(Color(nsColor: .textBackgroundColor))
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Color(nsColor: .separatorColor))
                )

            HStack {
                Button {
                    manager.chooseWireGuardConfigFile()
                } label: {
                    Label("Import File", systemImage: "doc.badge.plus")
                }
                .disabled(manager.actionInFlight)

                Button {
                    manager.saveWireGuardExitConfig(wireguardExitConfig)
                } label: {
                    Label("Save", systemImage: "checkmark")
                }
                .disabled(manager.actionInFlight)
            }
        }
    }

    private var networkSettings: some View {
        surface {
            HStack {
                sectionHeader("Networks", systemImage: "rectangle.stack")
                Spacer()
                TextField("New network", text: $networkNameInput)
                    .frame(width: 180)
                    .onSubmit { addNetwork() }
                Button {
                    addNetwork()
                } label: {
                    Image(systemName: "plus")
                }
                .disabled(networkNameInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || manager.actionInFlight)
            }

            if let network = shownNetwork {
                Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                    GridRow {
                        label(network.enabled ? "Active" : "Shown")
                        TextField("Name", text: networkNameBinding(network))
                        Button {
                            manager.renameNetwork(networkId: network.id, name: networkNameDrafts[network.id] ?? network.name)
                        } label: {
                            Image(systemName: "checkmark")
                        }
                        .disabled(!network.localIsAdmin || manager.actionInFlight)
                    }
                    GridRow {
                        label("Network ID")
                        Text(displayNetworkId(network.networkId))
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .textSelection(.enabled)
                        copyButton(value: network.networkId, copied: .meshId, systemImage: "doc.on.doc")
                    }
                    GridRow {
                        label("")
                        Button(role: .destructive) {
                            pendingNetworkRemoval = network
                        } label: {
                            Label("Delete this network", systemImage: "trash")
                        }
                        .buttonStyle(.borderless)
                        .disabled(manager.actionInFlight)
                        .confirmationDialog(
                            "Remove \(displayName(network))?",
                            isPresented: Binding(
                                get: { pendingNetworkRemoval?.id == network.id },
                                set: { if !$0 { pendingNetworkRemoval = nil } }
                            ),
                            titleVisibility: .visible
                        ) {
                            Button("Remove", role: .destructive) {
                                if let target = pendingNetworkRemoval {
                                    manager.removeNetwork(target.id)
                                }
                                pendingNetworkRemoval = nil
                            }
                            Button("Cancel", role: .cancel) { pendingNetworkRemoval = nil }
                        } message: {
                            Text("This deletes the network from this device.")
                        }
                    }
                }
            }

            disclosureSection(
                title: "Saved Networks",
                systemImage: "rectangle.stack",
                isExpanded: $savedNetworksExpanded,
                font: .subheadline.weight(.medium)
            ) {
                VStack(alignment: .leading, spacing: 10) {
                    if manager.inactiveNetworks.isEmpty {
                        emptyRow("No saved networks", systemImage: "rectangle.stack")
                    } else {
                        ForEach(manager.inactiveNetworks, id: \.id) { network in
                            savedNetworkRow(network)
                        }
                    }
                }
                .padding(.top, 8)
            }
        }
    }

    private func savedNetworkRow(_ network: NativeNetworkState) -> some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 3) {
                TextField("Name", text: networkNameBinding(network))
                    .textFieldStyle(.plain)
                Text("\(network.onlineCount) of \(network.expectedCount) connected")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Activate") {
                activateNetwork(network)
            }
            .disabled(manager.actionInFlight)
            Button(role: .destructive) {
                pendingNetworkRemoval = network
            } label: {
                Image(systemName: "trash")
            }
            .disabled(manager.actionInFlight)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
        .confirmationDialog(
            "Remove \(pendingNetworkRemoval.map(displayName) ?? "network")?",
            isPresented: Binding(
                get: { pendingNetworkRemoval?.id == network.id },
                set: { if !$0 { pendingNetworkRemoval = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                if let target = pendingNetworkRemoval {
                    manager.removeNetwork(target.id)
                }
                pendingNetworkRemoval = nil
            }
            Button("Cancel", role: .cancel) { pendingNetworkRemoval = nil }
        } message: {
            Text("This deletes the network from this device.")
        }
    }

    private var systemSettings: some View {
        surface {
            HStack(spacing: 8) {
                sectionHeader("System", systemImage: "gearshape.2")
                if !systemVersionLabel.isEmpty {
                    Text(systemVersionLabel)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                Spacer()
                if manager.serviceSettling || manager.updateChecking || manager.updateInstalling {
                    ProgressView()
                        .controlSize(.small)
                }
            }

            HStack(spacing: 8) {
                badge(state.serviceInstalled ? "Service installed" : "Service missing", style: state.serviceInstalled ? .ok : .warn)
                badge(state.serviceRunning ? "Running" : "Stopped", style: state.serviceRunning ? .ok : .muted)
                if manager.serviceUpdateRecommended {
                    badge("Update available", style: .warn)
                }
                badge(state.cliInstalled ? "CLI installed" : "CLI missing", style: state.cliInstalled ? .ok : .muted)
                badge(manager.updateAvailable ? "Update \(manager.updateVersion)" : "Up to date", style: manager.updateAvailable ? .warn : .ok)
            }

            if manager.serviceUpdateRecommended || !state.serviceStatusDetail.isEmpty || !manager.updateStatus.isEmpty {
                Text(firstNonEmpty(manager.updateStatus, state.serviceStatusDetail, fallback: ""))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }

            HStack {
                Button {
                    manager.installService()
                } label: {
                    Label(serviceInstallButtonTitle, systemImage: manager.serviceUpdateRecommended ? "arrow.up.circle" : "arrow.down.to.line")
                }
                .disabled(!state.serviceSupported || manager.actionInFlight || manager.serviceSettling)

                Button {
                    manager.checkForUpdates()
                } label: {
                    Label("Check Updates", systemImage: "arrow.triangle.2.circlepath")
                }
                .disabled(manager.updateChecking || manager.updateInstalling)

                Button {
                    manager.installCli()
                } label: {
                    Label(state.cliInstalled ? "Reinstall CLI" : "Install CLI", systemImage: "terminal")
                }
                .disabled(!state.cliInstallSupported || manager.actionInFlight)
            }
        }
    }

    private var diagnosticsSection: some View {
        surface {
            disclosureSection(
                title: "Diagnostics",
                systemImage: "waveform.path.ecg",
                isExpanded: $diagnosticsExpanded
            ) {
                VStack(alignment: .leading, spacing: 12) {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 170), alignment: .leading)], alignment: .leading, spacing: 10) {
                        metric("Peers", "\(state.connectedPeerCount)/\(state.expectedPeerCount)")
                        metric("Roster FIPS", "\(state.fipsConnectedPeerCount)/\(state.fipsRosterPeerCount) direct")
                        metric("Other FIPS", "\(state.nonFipsRosterPeerCount)")
                        metric("Interface", state.network.defaultInterface.isEmpty ? "unknown" : state.network.defaultInterface)
                        metric("IPv4", state.network.primaryIpv4.isEmpty ? "-" : state.network.primaryIpv4)
                        metric("IPv6", state.network.primaryIpv6.isEmpty ? "-" : state.network.primaryIpv6)
                        metric("Gateway", firstNonEmpty(state.network.gatewayIpv4, state.network.gatewayIpv6, fallback: "unknown"))
                        metric("Mapping", state.portMapping.activeProtocol.isEmpty ? "none" : state.portMapping.activeProtocol)
                        metric("External", state.portMapping.externalEndpoint.isEmpty ? "stun/direct" : state.portMapping.externalEndpoint)
                    }
                    if state.health.isEmpty {
                        emptyRow("No health warnings", systemImage: "checkmark.circle")
                    } else {
                        ForEach(state.health, id: \.code) { issue in
                            HStack(alignment: .top, spacing: 8) {
                                badge(issue.severity, style: healthStyle(issue.severity))
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(issue.summary)
                                    Text(issue.detail)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
                .padding(.top, 8)
            }
        }
    }

    private func disclosureSection<Content: View>(
        title: String,
        systemImage: String,
        isExpanded: Binding<Bool>,
        font: Font = .headline,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.14)) {
                    isExpanded.wrappedValue.toggle()
                }
            } label: {
                HStack(spacing: 7) {
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 10)
                        .rotationEffect(.degrees(isExpanded.wrappedValue ? 90 : 0))
                    Label(title, systemImage: systemImage)
                        .font(font)
                    Spacer(minLength: 0)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(title)
            .accessibilityValue(isExpanded.wrappedValue ? "Expanded" : "Collapsed")

            if isExpanded.wrappedValue {
                content()
            }
        }
    }

    private func surface<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            content()
        }
        .padding(14)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
    }

    private func sectionHeader(_ title: String, systemImage: String) -> some View {
        Label(title, systemImage: systemImage)
            .font(.headline)
    }

    private func emptyRow(_ text: String, systemImage: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: systemImage)
            Text(text)
        }
        .foregroundStyle(.secondary)
        .font(.subheadline)
        .padding(.vertical, 6)
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .foregroundStyle(.secondary)
            .frame(width: 86, alignment: .leading)
    }

    private func metric(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(value.isEmpty ? "-" : value)
                .font(.subheadline.weight(.medium))
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
        }
    }

    private func badge(_ text: String, style: BadgeStyle) -> some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .foregroundStyle(style.foreground)
            .background(style.background, in: RoundedRectangle(cornerRadius: 6))
    }

    private func copyButton(
        value: String,
        copied: CopyValue,
        peerNpub: String? = nil,
        systemImage: String
    ) -> some View {
        Button {
            manager.copy(value, as: copied, peerNpub: peerNpub)
        } label: {
            Image(systemName: copyIndicator(copied, peerNpub: peerNpub) ? "checkmark" : systemImage)
        }
        .buttonStyle(.borderless)
    }

    private func copyIndicator(_ copied: CopyValue, peerNpub: String?) -> Bool {
        manager.copiedValue == copied && (copied != .peerNpub || manager.copiedPeerNpub == peerNpub)
    }

    private func networkNameBinding(_ network: NativeNetworkState) -> Binding<String> {
        Binding(
            get: { networkNameDrafts[network.id] ?? network.name },
            set: { networkNameDrafts[network.id] = $0 }
        )
    }

    private func activateNetwork(_ network: NativeNetworkState) {
        guard !network.enabled else { return }
        shownNetworkId = network.id
        manager.setNetworkEnabled(networkId: network.id, enabled: true)
    }

    private func endpointHints(from value: String) -> [String] {
        value
            .components(separatedBy: CharacterSet(charactersIn: ", \n\r\t"))
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    private func addNetwork(defaultName: String = "") {
        let name = networkNameInput.trimmingCharacters(in: .whitespacesAndNewlines)
        manager.addNetwork(name.isEmpty ? defaultName : name)
        networkNameInput = ""
    }

    private func syncDrafts() {
        if let shownNetworkId,
           !state.networks.contains(where: { $0.id == shownNetworkId }) {
            self.shownNetworkId = nil
        }
        if state.nodeName != lastSyncedNodeName {
            nodeName = state.nodeName
            lastSyncedNodeName = state.nodeName
        }
        if state.endpoint != lastSyncedEndpoint {
            endpoint = state.endpoint
            lastSyncedEndpoint = state.endpoint
        }
        if state.tunnelIp != lastSyncedTunnelIp {
            tunnelIp = state.tunnelIp
            lastSyncedTunnelIp = state.tunnelIp
        }
        if state.listenPort != lastSyncedListenPort {
            listenPort = String(state.listenPort)
            lastSyncedListenPort = state.listenPort
        }
        if state.fipsHostInboundTcpPorts != lastSyncedFipsHostInboundTcpPorts {
            fipsHostInboundTcpPorts = state.fipsHostInboundTcpPorts
            lastSyncedFipsHostInboundTcpPorts = state.fipsHostInboundTcpPorts
        }
        if state.nostrPubsubMode != lastSyncedNostrPubsubMode {
            nostrPubsubMode = state.nostrPubsubMode
            lastSyncedNostrPubsubMode = state.nostrPubsubMode
        }
        if state.nostrPubsubFanout != lastSyncedNostrPubsubFanout {
            nostrPubsubFanout = String(state.nostrPubsubFanout)
            lastSyncedNostrPubsubFanout = state.nostrPubsubFanout
        }
        if state.nostrPubsubMaxHops != lastSyncedNostrPubsubMaxHops {
            nostrPubsubMaxHops = String(state.nostrPubsubMaxHops)
            lastSyncedNostrPubsubMaxHops = state.nostrPubsubMaxHops
        }
        if state.nostrPubsubMaxEventBytes != lastSyncedNostrPubsubMaxEventBytes {
            nostrPubsubMaxEventBytes = String(state.nostrPubsubMaxEventBytes)
            lastSyncedNostrPubsubMaxEventBytes = state.nostrPubsubMaxEventBytes
        }
        if lastSyncedWireguardExitConfig != state.wireguardExitConfig {
            wireguardExitConfig = state.wireguardExitConfig
            lastSyncedWireguardExitConfig = state.wireguardExitConfig
        }
        if lastSyncedPaidExitSeller != state.paidExitSeller {
            let seller = state.paidExitSeller
            paidExitMeter = seller.meter
            paidExitPriceMsat = String(seller.priceMsat)
            paidExitPerUnits = fallbackText(seller.perUnitsText, paidExitPricingUnitDraft(seller.perUnits, meter: seller.meter))
            paidExitAcceptedMints = seller.acceptedMints.joined(separator: ", ")
            paidExitMaxChannelCapacitySat = String(seller.maxChannelCapacitySat)
            paidExitChannelExpirySecs = paidExitDurationDraft(seller.channelExpirySecs)
            paidExitFreeProbeUnits = fallbackText(seller.freeProbeText, paidExitTrafficUnitDraft(seller.freeProbeUnits, meter: seller.meter))
            paidExitGraceUnits = fallbackText(seller.graceText, paidExitTrafficUnitDraft(seller.graceUnits, meter: seller.meter))
            paidExitCountryCode = seller.countryCode
            paidExitRegion = seller.region
            paidExitAsn = seller.asn == 0 ? "" : String(seller.asn)
            paidExitNetworkClass = seller.networkClass
            paidExitIpv4 = seller.ipv4
            paidExitIpv6 = seller.ipv6
            lastSyncedPaidExitSeller = seller
        }

        for network in state.networks {
            if networkNameDrafts[network.id] == nil {
                networkNameDrafts[network.id] = network.name
            }
        }

        if let network = shownNetwork {
            let participants = sortedParticipants(network)
            if let selectedDevicePubkeyHex,
               participants.contains(where: { $0.pubkeyHex == selectedDevicePubkeyHex }) {
                return
            }
            selectedDevicePubkeyHex = participants.first?.pubkeyHex
        } else {
            selectedDevicePubkeyHex = nil
        }
    }

    private func displayName(_ network: NativeNetworkState) -> String {
        network.name.isEmpty ? "Network" : network.name
    }

    /// A valid device ID is a bech32-encoded npub: `npub1` + 58 bech32 chars.
    private func isValidDeviceId(_ value: String) -> Bool {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count == 63, trimmed.hasPrefix("npub1") else { return false }
        let body = trimmed.dropFirst(5)
        let allowed: Set<Character> = Set("qpzry9x8gf2tvdw0s3jn54khce6mua7l")
        return body.allSatisfy { allowed.contains($0) }
    }

    private var headerVpnStatusText: String {
        manager.vpnStatusText
    }

    private var headerStatusDotVisible: Bool {
        state.exitNodeBlocked || state.exitNodeActive || state.vpnActive || state.vpnEnabled
    }

    private var headerStatusColor: Color {
        if state.exitNodeBlocked {
            return .red
        }
        if state.exitNodeActive || state.vpnActive {
            return .green
        }
        if state.vpnEnabled {
            return .orange
        }
        return .secondary
    }

    private var headerStatusTextColor: Color {
        state.exitNodeBlocked ? .red : .secondary
    }

    private func deviceAvailabilityText(_ network: NativeNetworkState) -> String {
        if network.expectedCount == 0 {
            return "No devices"
        }
        let deviceWord = network.expectedCount == 1 ? "device" : "devices"
        return "\(network.onlineCount) online · \(network.expectedCount) \(deviceWord)"
    }

    private var serviceInstallButtonTitle: String {
        if manager.serviceUpdateRecommended {
            return "Update Service"
        }
        return state.serviceInstalled ? "Reinstall Service" : "Install Service"
    }

    private func sortedParticipants(_ network: NativeNetworkState) -> [NativeParticipantState] {
        network.participants.sorted { lhs, rhs in
            if isSelf(lhs) != isSelf(rhs) {
                return isSelf(lhs)
            }
            if lhs.reachable != rhs.reachable {
                return lhs.reachable && !rhs.reachable
            }
            return deviceName(lhs).localizedCaseInsensitiveCompare(deviceName(rhs)) == .orderedAscending
        }
    }

    private func visibleParticipants(_ network: NativeNetworkState, search: String) -> [NativeParticipantState] {
        let needle = search.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !needle.isEmpty else {
            return sortedParticipants(network)
        }
        return sortedParticipants(network).filter { participant in
            [
                deviceName(participant),
                participant.alias,
                participant.magicDnsAlias,
                participant.magicDnsName,
                participant.npub,
                participant.tunnelIp,
                deviceStatusText(participant),
            ].contains { $0.lowercased().contains(needle) }
        }
    }

    private func selectedParticipant(in network: NativeNetworkState) -> NativeParticipantState? {
        let participants = sortedParticipants(network)
        if let selectedDevicePubkeyHex,
           let selected = participants.first(where: { $0.pubkeyHex == selectedDevicePubkeyHex }) {
            return selected
        }
        return participants.first
    }

    private func isSelf(_ participant: NativeParticipantState) -> Bool {
        participant.npub == state.ownNpub || participant.meshState == "local"
    }

    private func deviceName(_ participant: NativeParticipantState) -> String {
        if !participant.magicDnsName.isEmpty {
            return participant.magicDnsName
        }
        if isSelf(participant), !state.selfMagicDnsName.isEmpty {
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

    private func deviceSubtitle(_ participant: NativeParticipantState) -> String {
        return ""
    }

    private func deviceMagicDnsName(_ participant: NativeParticipantState) -> String {
        if !participant.magicDnsName.isEmpty {
            return participant.magicDnsName
        }
        if isSelf(participant), !state.selfMagicDnsName.isEmpty {
            return state.selfMagicDnsName
        }
        if !participant.magicDnsAlias.isEmpty, !state.magicDnsSuffix.isEmpty {
            return "\(participant.magicDnsAlias).\(state.magicDnsSuffix)"
        }
        return ""
    }

    private func isActiveExitParticipant(_ participant: NativeParticipantState) -> Bool {
        state.exitNodeActive && !state.exitNode.isEmpty && participant.npub == state.exitNode
    }

    private func exitNodeBadgeText(_ participant: NativeParticipantState) -> String {
        isActiveExitParticipant(participant) ? "Exit active" : "Exit offered"
    }

    private func exitNodeBadgeStyle(_ participant: NativeParticipantState) -> BadgeStyle {
        isActiveExitParticipant(participant) ? .ok : .warn
    }

    private func deviceRoleText(_ participant: NativeParticipantState) -> String {
        var roles: [String] = []
        if isSelf(participant) {
            roles.append("This device")
        }
        if participant.isAdmin {
            roles.append("Admin")
        }
        if participant.offersExitNode {
            roles.append(exitNodeBadgeText(participant))
        }
        return roles.isEmpty ? "Member" : roles.joined(separator: ", ")
    }

    private func deviceStatusText(_ participant: NativeParticipantState) -> String {
        switch participant.state {
        case "local", "online", "present":
            return "Online"
        case "pending":
            return "Connecting"
        case "offline", "absent", "off":
            return "Offline"
        case _ where participant.reachable:
            return "Online"
        default:
            return "Unknown"
        }
    }

    private func fipsPathText(_ participant: NativeParticipantState) -> String {
        if isSelf(participant) {
            return "This device"
        }
        if isDirectFipsPeer(participant) {
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

    private func isDirectFipsPeer(_ participant: NativeParticipantState) -> Bool {
        !isSelf(participant)
            && participant.reachable
            && !participant.fipsTransportAddr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func isFipsRouted(_ participant: NativeParticipantState) -> Bool {
        !isSelf(participant)
            && participant.reachable
            && participant.fipsTransportAddr.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func connectivityDot(_ participant: NativeParticipantState, size: CGFloat) -> some View {
        Circle()
            .fill(connectivityTint(participant))
            .frame(width: size, height: size)
    }

    private func connectivityTint(_ participant: NativeParticipantState) -> Color {
        switch participant.state {
        case "local", "online", "present":
            return .green
        case "pending":
            return .orange
        default:
            return .secondary
        }
    }

    private func exitNodeCandidates(_ network: NativeNetworkState, search: String) -> [NativeParticipantState] {
        let needle = search.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return network.participants.filter { participant in
            if isSelf(participant) || !participant.offersExitNode {
                return false
            }
            guard !needle.isEmpty else {
                return true
            }
            return [
                participant.alias,
                participant.magicDnsAlias,
                participant.magicDnsName,
                participant.npub,
                participant.tunnelIp,
            ].contains { $0.lowercased().contains(needle) }
        }
    }

    private func badgeStyle(for state: String) -> BadgeStyle {
        switch state {
        case "local", "online", "present":
            return .ok
        case "pending":
            return .warn
        case "offline", "absent":
            return .bad
        default:
            return .muted
        }
    }

    private func healthStyle(_ severity: String) -> BadgeStyle {
        switch severity {
        case "critical":
            return .bad
        case "warning":
            return .warn
        case "info":
            return .muted
        default:
            return .muted
        }
    }
}

private struct PendingParticipantRemoval {
    let networkId: String
    let npub: String
    let deviceName: String
}

private struct PendingJoinRequest: Identifiable {
    let id = UUID()
    let networkId: String
    let networkName: String
    let request: String
}

private func looksLikeJoinRequestQrOrLink(_ value: String) -> Bool {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    return trimmed.hasPrefix("nvpn://join-request/") && trimmed.count > "nvpn://join-request/".count
}

private struct LocalSearchScope<Content: View>: View {
    @State private var search = ""
    private let content: (Binding<String>) -> Content

    init(@ViewBuilder content: @escaping (Binding<String>) -> Content) {
        self.content = content
    }

    var body: some View {
        content($search)
    }
}

private struct SyncedTextFieldRow: View {
    let title: String
    let placeholder: String
    let identity: String
    let value: String
    let systemImage: String
    let disabled: Bool
    let onSave: (String) -> Void

    @State private var draft = ""
    @State private var syncedIdentity = ""
    @State private var syncedValue = ""

    var body: some View {
        HStack(spacing: 8) {
            Text(title)
                .foregroundStyle(.secondary)
                .frame(width: 86, alignment: .leading)
            TextField(placeholder, text: $draft)
            Button {
                onSave(draft)
            } label: {
                Label("Save", systemImage: systemImage)
            }
            .disabled(disabled)
        }
        .onAppear {
            syncDraft(force: true)
        }
        .onChange(of: identity) { _, _ in
            syncDraft(force: true)
        }
        .onChange(of: value) { _, _ in
            syncDraft(force: false)
        }
    }

    private func syncDraft(force: Bool) {
        let identityChanged = syncedIdentity != identity
        if force || identityChanged || draft == syncedValue {
            draft = value
        }
        syncedIdentity = identity
        syncedValue = value
    }
}

private struct WrappingIdentifierText: NSViewRepresentable {
    let value: String
    let font: NSFont
    let color: NSColor

    func makeNSView(context: Context) -> NSTextView {
        let textView = NSTextView()
        textView.drawsBackground = false
        textView.isEditable = false
        textView.isSelectable = true
        textView.isRichText = false
        textView.textContainerInset = .zero
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.lineBreakMode = .byCharWrapping
        textView.textContainer?.widthTracksTextView = true
        textView.isHorizontallyResizable = false
        textView.isVerticallyResizable = true
        textView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return textView
    }

    func updateNSView(_ textView: NSTextView, context: Context) {
        textView.textStorage?.setAttributedString(attributedText)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSTextView, context: Context) -> CGSize? {
        guard let width = proposal.width, let textContainer = nsView.textContainer else {
            return nil
        }
        nsView.frame.size.width = width
        textContainer.containerSize = CGSize(width: width, height: .greatestFiniteMagnitude)
        nsView.layoutManager?.ensureLayout(for: textContainer)
        let height = nsView.layoutManager?.usedRect(for: textContainer).height ?? font.pointSize
        return CGSize(width: width, height: ceil(height))
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

struct InviteQRCodeView: View {
    let invite: String

    var body: some View {
        if invite.isEmpty {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color(nsColor: .textBackgroundColor))
                .overlay(Image(systemName: "qrcode").foregroundStyle(.secondary))
        } else if let image = qrImage(invite) {
            Image(nsImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .padding(8)
                .background(Color.white, in: RoundedRectangle(cornerRadius: 8))
        } else {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color(nsColor: .textBackgroundColor))
                .overlay(Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange))
        }
    }

    private func qrImage(_ text: String) -> NSImage? {
        let data = Data(text.utf8)
        guard let filter = CIFilter(name: "CIQRCodeGenerator") else {
            return nil
        }
        filter.setValue(data, forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")
        guard let output = filter.outputImage else {
            return nil
        }
        let transformed = output.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        let representation = NSCIImageRep(ciImage: transformed)
        let image = NSImage(size: representation.size)
        image.addRepresentation(representation)
        return image
    }
}

enum SidebarItem: Hashable {
    case devices
    case internet
    case publicExits
    case sellExit
    case wallet
    case settings
}

enum BadgeStyle {
    case ok
    case warn
    case bad
    case muted
    case selected

    var foreground: Color {
        switch self {
        case .ok:
            return .green
        case .warn:
            return .orange
        case .bad:
            return .red
        case .muted:
            return .secondary
        case .selected:
            return .white
        }
    }

    var background: Color {
        switch self {
        case .ok:
            return .green.opacity(0.14)
        case .warn:
            return .orange.opacity(0.14)
        case .bad:
            return .red.opacity(0.14)
        case .muted:
            return .secondary.opacity(0.12)
        case .selected:
            return .white.opacity(0.18)
        }
    }
}

private func formatBytes(_ bytes: UInt64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"]
    var value = Double(bytes)
    var unitIndex = 0
    while value >= 1024, unitIndex < units.count - 1 {
        value /= 1024
        unitIndex += 1
    }
    if unitIndex == 0 {
        return "\(bytes) B"
    }
    if value.rounded() == value {
        return String(format: "%.0f %@", value, units[unitIndex])
    }
    return String(format: "%.1f %@", value, units[unitIndex])
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

private func cleanIp(_ value: String) -> String {
    value.split(separator: "/").first.map(String.init) ?? value
}

private func firstNonEmpty(_ values: String..., fallback: String) -> String {
    values.first { !$0.isEmpty } ?? fallback
}
