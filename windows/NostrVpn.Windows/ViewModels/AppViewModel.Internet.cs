using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using NostrVpn.Windows.Core;

namespace NostrVpn.Windows.ViewModels;

public sealed partial class AppViewModel
{
    public Task SetAdvertiseExitNodeAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { AdvertiseExitNode = enabled }),
            "Saving internet sharing");
    }

    public Task SetExitNodeLeakProtectionAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { ExitNodeLeakProtection = enabled }),
            "Saving internet protection");
    }

    public Task SetWalletFiatEnabledAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { WalletFiatEnabled = enabled }),
            "Saving wallet display");
    }

    public Task SetWalletFiatCurrencyAsync(string currency)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { WalletFiatCurrency = currency }),
            "Saving wallet currency");
    }

    public Task SetWireGuardExitEnabledAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { WireguardExitEnabled = enabled }),
            "Saving WireGuard");
    }

    public Task SetExitNodeAsync(string npub)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { ExitNode = npub }),
            "Saving internet source");
    }

    public Task SelectDirectExitAsync()
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { InternetSource = "direct" }),
            "Saving internet source");
    }

    public Task SelectWireGuardUpstreamExitAsync()
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { InternetSource = "wireguard" }),
            "Saving internet source");
    }

    public Task SelectPeerExitAsync(string npub)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch
            {
                InternetSource = "private_vpn",
                ExitNode = npub,
            }),
            "Saving internet source");
    }

    public Task SelectPaidAutomaticExitAsync()
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { InternetSource = "paid_automatic" }),
            "Saving internet source");
    }

    public async Task SelectPaidManualExitAsync()
    {
        await DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { InternetSource = "paid_manual" }),
            "Saving internet source");
        Page = AppPage.PublicExits;
    }

    public Task DiscoverPaidRouteOffersAsync() =>
        DispatchAsync(NativeActions.DiscoverPaidRouteOffers(), "Finding sellers");

    public Task RefreshPaidRouteWalletAsync() =>
        DispatchAsync(NativeActions.RefreshPaidRouteWallet(), "Refreshing wallet");

    public Task AddPaidRouteWalletMintAsync()
    {
        var url = PaidRouteMintUrl.Trim();
        return string.IsNullOrWhiteSpace(url)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.AddPaidRouteWalletMint(url), "Adding mint");
    }

    public Task TopUpPaidRouteWalletAsync()
    {
        var amount = ParsePositiveUInt64(PaidRouteTopUpAmount);
        return amount is null
            ? Task.CompletedTask
            : DispatchAsync(
                NativeActions.TopUpPaidRouteWallet(OptionalTrimmed(PaidRouteMintUrl), amount.Value),
                "Creating top-up invoice");
    }

    public async Task ReceivePaidRouteWalletTokenAsync()
    {
        var token = PaidRouteReceiveToken.Trim();
        if (string.IsNullOrWhiteSpace(token))
        {
            return;
        }
        await DispatchAsync(NativeActions.ReceivePaidRouteWalletToken(token), "Importing token");
        PaidRouteReceiveToken = "";
    }

    public Task SendPaidRouteWalletTokenAsync()
    {
        var amount = ParsePositiveUInt64(PaidRouteSendAmount);
        return amount is null
            ? Task.CompletedTask
            : DispatchAsync(
                NativeActions.SendPaidRouteWalletToken(OptionalTrimmed(PaidRouteMintUrl), amount.Value),
                "Exporting token");
    }

    public Task WithdrawPaidRouteWalletLightningAsync()
    {
        var invoice = PaidRouteWithdrawInvoice.Trim();
        return string.IsNullOrWhiteSpace(invoice)
            ? Task.CompletedTask
            : DispatchAsync(
                NativeActions.WithdrawPaidRouteWalletLightning(OptionalTrimmed(PaidRouteMintUrl), invoice),
                "Paying invoice");
    }

    public Task BuyPaidRouteOfferAsync(NativePaidRouteOfferState offer) =>
        string.IsNullOrWhiteSpace(offer.Key)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.BuyPaidRouteOffer(offer.Key), "Connecting");

    public Task SelectPaidRouteSessionAsync(NativePaidRouteSessionState session) =>
        string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.SelectPaidRouteSession(session.SessionId, true), "Connecting");

    public Task ProbePaidRouteSessionAsync(NativePaidRouteSessionState session) =>
        string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.ProbePaidRouteSession(session.SessionId), "Checking connection");

    public Task OpenPaidRouteChannelAsync(NativePaidRouteSessionState session) =>
        string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.OpenPaidRouteChannelFromWallet(session.SessionId), "Funding seller");

    public Task SignPaidRoutePaymentAsync(NativePaidRouteSessionState session) =>
        string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.SignPaidRoutePaymentEnvelopeFromWallet(session.SessionId), "Paying seller");

    public Task ClosePaidRouteChannelAsync(NativePaidRouteSessionState session) =>
        string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.ClosePaidRouteChannelFromWallet(session.SessionId), "Settling channel");

    public Task SendPaidRoutePaymentEnvelopeAsync()
    {
        var envelope = State.PaidRouteMarket.LastPaymentAction.EnvelopeJson.Trim();
        return string.IsNullOrWhiteSpace(envelope)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.SendPaidRoutePaymentEnvelope(envelope), "Sending payment");
    }

    public Task StreamPaidRoutePaymentsAsync() =>
        DispatchAsync(NativeActions.StreamPaidRoutePayments(), "Paying for usage");

    public Task SetPaidExitEnabledAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { PaidExitEnabled = enabled }),
            "Saving listing");
    }

    public Task PublishPaidExitOfferAsync() =>
        DispatchAsync(NativeActions.PublishPaidExitOffer(), "Advertising listing");

    public Task ReceivePaidRoutePaymentsAsync() =>
        DispatchAsync(NativeActions.ReceivePaidRoutePayments(), "Checking payments");
}
