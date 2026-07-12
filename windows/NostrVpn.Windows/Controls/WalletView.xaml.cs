using System;
using System.Windows;
using System.Windows.Controls;
using NostrVpn.Windows.ViewModels;

namespace NostrVpn.Windows.Controls;

public partial class WalletView : UserControl
{
    public WalletView()
    {
        InitializeComponent();
    }

    private AppViewModel ViewModel => (AppViewModel)DataContext;

    private async void RefreshPaidRouteWallet_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.RefreshPaidRouteWalletAsync();

    private async void AddPaidRouteWalletMint_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.AddPaidRouteWalletMintAsync();

    private async void TopUpPaidRouteWallet_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.TopUpPaidRouteWalletAsync();

    private async void PaidRouteReceiveToken_Changed(object sender, TextChangedEventArgs e)
    {
        if (DataContext is AppViewModel viewModel)
        {
            await viewModel.AutoReceivePaidRouteWalletTokenAsync();
        }
    }

    private async void ScanPaidRouteWalletToken_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.ScanPaidRouteWalletTokenAsync();

    private async void SendPaidRouteWalletToken_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.SendPaidRouteWalletTokenAsync();

    private async void WithdrawPaidRouteWalletLightning_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.WithdrawPaidRouteWalletLightningAsync();

    private void CopyWalletValue_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string value })
        {
            ViewModel.CopyText(value);
        }
    }
}
