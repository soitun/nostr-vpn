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

    private async void WalletFiatEnabled_Click(object sender, RoutedEventArgs e)
    {
        if (sender is CheckBox checkBox)
        {
            await ViewModel.SetWalletFiatEnabledAsync(checkBox.IsChecked == true);
        }
    }

    private async void WalletFiatCurrency_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (sender is ComboBox { SelectedItem: ComboBoxItem item }
            && item.Content is string currency
            && !string.Equals(currency, ViewModel.State.WalletFiatCurrency, StringComparison.Ordinal))
        {
            await ViewModel.SetWalletFiatCurrencyAsync(currency);
        }
    }

    private async void RefreshPaidRouteWallet_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.RefreshPaidRouteWalletAsync();

    private async void AddPaidRouteWalletMint_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.AddPaidRouteWalletMintAsync();

    private async void TopUpPaidRouteWallet_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.TopUpPaidRouteWalletAsync();

    private async void ReceivePaidRouteWalletToken_Click(object sender, RoutedEventArgs e) =>
        await ViewModel.ReceivePaidRouteWalletTokenAsync();

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
