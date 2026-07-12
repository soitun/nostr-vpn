import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct InternetPage: View {
    @ObservedObject var model: AppModel
    let network: NetworkState?

    private var exitParticipants: [ParticipantState] {
        network?.participants.filter { participant in
            participant.offersExitNode && !isSelf(participant, state: model.state)
        } ?? []
    }

    private func selectSource(_ source: String) {
        model.dispatch(
            NativeActions.updateSettings(["internetSource": source]),
            status: "Saving internet"
        )
    }

    private func selectPeer(_ npub: String) {
        model.dispatch(
            NativeActions.updateSettings(["internetSource": "private_vpn", "exitNode": npub]),
            status: "Saving internet"
        )
    }

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                AppCard {
                    Text("Internet source")
                        .font(.headline)
                    Picker("Internet source", selection: Binding(
                        get: { model.state.internetSource },
                        set: selectSource
                    )) {
                        Text("This device").tag("direct")
                        Text("Private VPN device").tag("private_vpn")
                        Text("Paid · Automatic").tag("paid_automatic")
                        Text("Paid · Choose manually").tag("paid_manual")
                        Text("WireGuard VPN").tag("wireguard")
                    }
                    .pickerStyle(.menu)

                    if model.state.internetSource == "private_vpn" {
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
                                    selected: model.state.exitNode == participant.npub,
                                    enabled: true,
                                    action: { selectPeer(participant.npub) }
                                )
                            }
                        }
                    }
                }

                if model.state.internetSource == "paid_automatic" {
                    AppCard {
                        Text("Automatic paid provider")
                            .font(.headline)
                        Text("Experimental")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text(model.state.exitNodeStatusText.isEmpty
                            ? "Looking for a working provider at a reasonable price"
                            : model.state.exitNodeStatusText)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                } else if model.state.internetSource == "paid_manual" {
                    PaidRouteMarketCard(model: model, mode: .market)
                }

                AppCard {
                    Text("Share Internet")
                        .font(.headline)
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
                if model.state.paidExitSeller.supported {
                    PaidExitSellerStatusCard(state: model.state)
                }
                if model.state.internetSource == "wireguard" {
                    WireGuardSettingsCard(model: model)
                }
            }
            .padding()
        }
        .safeAreaPadding(.bottom, 92)
        .background(AppColors.background)
    }
}
struct PaidRouteWalletPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                Text("Pay for internet access and receive earnings when you sell bandwidth.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                AppCard {
                    Toggle("Show fiat value", isOn: Binding(
                        get: { model.state.walletFiatEnabled },
                        set: { enabled in
                            model.dispatch(
                                NativeActions.updateSettings(["walletFiatEnabled": enabled]),
                                status: "Saving wallet display"
                            )
                        }
                    ))
                    if model.state.walletFiatEnabled {
                        Text("Rates from Coinbase and Kraken")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                        Picker("Currency", selection: Binding(
                            get: { model.state.walletFiatCurrency },
                            set: { currency in
                                model.dispatch(
                                    NativeActions.updateSettings(["walletFiatCurrency": currency]),
                                    status: "Saving wallet currency"
                                )
                            }
                        )) {
                            ForEach(["USD", "EUR", "GBP", "CAD", "AUD", "JPY", "CHF"], id: \.self) {
                                Text($0).tag($0)
                            }
                        }
                        .pickerStyle(.menu)
                    }
                }
                PaidRouteMarketCard(model: model, mode: .wallet)
            }
            .padding()
        }
        .safeAreaPadding(.bottom, 92)
        .background(AppColors.background)
    }
}

enum PaidRouteCardMode {
    case market
    case wallet
}

private enum PaidRouteWalletFlow: String, Identifiable {
    case receive
    case send

    var id: String { rawValue }
}

struct PaidRouteMarketCard: View {
    @ObservedObject var model: AppModel
    let mode: PaidRouteCardMode
    @State private var mintUrl = ""
    @State private var token = ""
    @State private var topUpAmount = ""
    @State private var sendAmount = ""
    @State private var withdrawInvoice = ""
    @State private var walletFlow: PaidRouteWalletFlow?
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
                    Text(mode == .wallet ? "Wallet" : "Buy Internet")
                        .font(.headline)
                    if mode == .market {
                        Text("Experimental")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Text(fallbackText(market.wallet.totalBalanceText, formatPaidRouteMsat(market.wallet.totalBalanceMsat)))
                        .font(mode == .wallet ? .largeTitle.bold() : .footnote)
                        .foregroundStyle(mode == .wallet ? .primary : .secondary)
                    if model.state.walletFiatEnabled && !market.wallet.fiatBalanceText.isEmpty {
                        Text("≈ \(market.wallet.fiatBalanceText)")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    if mode == .wallet && !market.wallet.exchangeRateText.isEmpty {
                        Text("\(market.wallet.exchangeRateText) · \(market.wallet.exchangeRateSources)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
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
                Text(mode == .wallet ? "Wallet is not supported on this platform" : "Buying internet is not supported on this platform")
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
        .sheet(item: $walletFlow) { flow in
            walletFlowSheet(flow)
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
                Button {
                    walletFlow = .receive
                } label: {
                    Label("Receive", systemImage: "arrow.down.circle.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)

                Button {
                    walletFlow = .send
                } label: {
                    Label("Send", systemImage: "arrow.up.circle.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
            }
            .controlSize(.large)
        }
    }

    private func walletFlowSheet(_ flow: PaidRouteWalletFlow) -> some View {
        NavigationStack {
            Form {
                Section("Lightning") {
                    if flow == .receive {
                        TextField("Amount in sats", text: $topUpAmount)
                            .keyboardType(.numberPad)
                        Button("Create Invoice") {
                            guard let amount = parsePositivePaidRouteAmount(topUpAmount) else { return }
                            model.dispatch(
                                NativeActions.topUpPaidRouteWallet(mintUrl: optionalPaidRouteMintUrl(mintUrl), amountSat: amount),
                                status: "Creating invoice"
                            )
                        }
                        .disabled(model.actionInFlight || parsePositivePaidRouteAmount(topUpAmount) == nil)
                    } else {
                        TextField("Invoice", text: $withdrawInvoice)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        Button("Pay") {
                            let trimmed = withdrawInvoice.trimmingCharacters(in: .whitespacesAndNewlines)
                            model.dispatch(
                                NativeActions.withdrawPaidRouteWalletLightning(mintUrl: optionalPaidRouteMintUrl(mintUrl), invoice: trimmed),
                                status: "Paying invoice"
                            )
                        }
                        .disabled(model.actionInFlight || withdrawInvoice.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }

                Section("Token") {
                    if flow == .receive {
                        TextField("Paste token", text: $token)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        Button("Import") {
                            let trimmed = token.trimmingCharacters(in: .whitespacesAndNewlines)
                            model.dispatch(
                                NativeActions.receivePaidRouteWalletToken(token: trimmed),
                                status: "Receiving token"
                            )
                        }
                        .disabled(model.actionInFlight || token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    } else {
                        TextField("Amount in sats", text: $sendAmount)
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
                }

                walletActionResult(market.wallet.lastAction)
            }
            .navigationTitle(flow == .receive ? "Receive" : "Send")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { walletFlow = nil }
                }
            }
        }
    }

    private var walletMintList: some View {
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
                            Text(mint.url)
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
                CopyLine(value: action.paymentRequest, displayValue: "Lightning invoice", model: model)
            }
            if !action.token.isEmpty {
                CopyLine(value: action.token, displayValue: "Token", model: model)
            }
            if !action.preimage.isEmpty {
                CopyLine(value: action.preimage, displayValue: "Lightning preimage", model: model)
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

struct PaidRouteOfferRow: View {
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

struct PaidRouteSessionRow: View {
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

struct PaidExitSellerStatusCard: View {
    let state: AppState

    var body: some View {
        let seller = state.paidExitSeller
        AppCard {
            Text("Sell Internet · Experimental")
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

struct ExitNodeRow: View {
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
