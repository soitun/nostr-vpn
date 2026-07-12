import AppKit
import CoreImage
import SwiftUI

extension RootView {
    var paidRouteWalletSettings: some View {
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

    func paidRouteWalletSection(_ wallet: NativePaidRouteWalletState) -> some View {
        surface {
            Text("Use this Cashu wallet to pay for internet access and receive earnings when you sell bandwidth.")
                .font(.callout)
                .foregroundStyle(.secondary)

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

            Toggle("Show exchange rate", isOn: Binding(
                get: { state.walletFiatEnabled },
                set: { manager.setWalletFiatEnabled($0) }
            ))
            .disabled(manager.actionInFlight)

            if state.walletFiatEnabled {
                HStack(spacing: 10) {
                    Picker("Currency", selection: Binding(
                        get: { state.walletFiatCurrency },
                        set: { manager.setWalletFiatCurrency($0) }
                    )) {
                        ForEach(["USD", "EUR", "GBP", "CAD", "AUD", "JPY", "CHF"], id: \.self) {
                            Text($0).tag($0)
                        }
                    }
                    .frame(width: 180)

                    if !wallet.fiatBalanceText.isEmpty {
                        Text(wallet.fiatBalanceText)
                            .font(.headline)
                    }
                    Spacer(minLength: 8)
                }

                if !wallet.exchangeRateText.isEmpty {
                    Text([wallet.exchangeRateText, wallet.exchangeRateStatus]
                        .filter { !$0.isEmpty }
                        .joined(separator: " · "))
                        .font(.caption)
                        .foregroundStyle(wallet.exchangeRateStale ? Color.orange : Color.secondary)
                }
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
    func paidRouteWalletActionResult(_ action: NativePaidRouteWalletActionState) -> some View {
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

    func paidRouteWalletOutputRow(_ title: String, value: String, copied: CopyValue) -> some View {
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
    func paidRoutePaymentActionResult(_ action: NativePaidRoutePaymentActionState) -> some View {
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

    func paidRouteMintRow(_ mint: NativePaidRouteWalletMintState) -> some View {
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
}

