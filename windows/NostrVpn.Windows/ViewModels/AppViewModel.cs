using System;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using Microsoft.Win32;
using NostrVpn.Windows.Core;
using NostrVpn.Windows.Services;

namespace NostrVpn.Windows.ViewModels;

public enum AppPage
{
    Devices,
    AddNetwork,
    AddDevice,
    Internet,
    PublicExits,
    Wallet,
    SellAccess,
    Settings,
}

public sealed class AppViewModel : INotifyPropertyChanged, IDisposable
{
    private readonly AppCoreClient _core;
    private readonly DispatcherTimer _refreshTimer;
    private readonly DispatcherTimer _updateTimer;
    private readonly UpdateService _updateService = new();
    private NativeAppState _state = new();
    private AppPage _page = AppPage.Devices;
    private string _selectedParticipantKey = "";
    private string _shownNetworkId = "";
    private bool _actionInFlight;
    private string _notice = "";
    private string _inviteInput = "";
    private string _joinRequestInput = "";
    private string _participantInput = "";
    private string _participantAliasInput = "";
    private string _networkNameInput = "";
    private string _networkNameDraft = "";
    private string _networkMeshIdDraft = "";
    private string _nodeName = "";
    private string _endpoint = "";
    private string _tunnelIp = "";
    private string _listenPort = "";
    private string _relayInput = "";
    private string _relaysDraft = "";
    private string _fipsHostInboundTcpPorts = "";
    private string _advertisedRoutes = "";
    private string _wireguardExitConfig = "";
    private string _paidRouteMintUrl = "";
    private string _paidRouteTopUpAmount = "";
    private string _paidRouteSendAmount = "";
    private string _paidRouteReceiveToken = "";
    private string _paidRouteWithdrawInvoice = "";
    private string _manualJoinAdminId = "";
    private string _manualJoinMeshId = "";
    private bool _manualJoinExpanded;
    private string _networkSetupMode = "";
    private string _updateStatus = "";
    private Uri? _updateAssetUrl;
    private bool _updateUsesCoreDownload;
    private bool _updateChecking;
    private bool _updateInstalling;
    private bool _updateAvailable;
    private bool _autoInstallUpdates;
    private string _updateVersion = "";
    private QrMatrix _joinRequestQr = new();
    private bool _joinRequestPromptOpen;
    private static readonly TimeSpan UpdatePollInterval = LoadUpdatePollInterval();
    private static readonly Brush HeaderDangerBrush = new SolidColorBrush(Color.FromRgb(220, 38, 38));
    private static readonly Brush TextSecondaryBrush = new SolidColorBrush(Color.FromRgb(104, 113, 124));
    private static readonly Brush ActiveNetworkBrush = new SolidColorBrush(Color.FromRgb(22, 163, 74));
    private static readonly Brush InactiveNetworkBrush = new SolidColorBrush(Color.FromRgb(156, 163, 175));

    public AppViewModel()
    {
        var version = Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "";
        var dataDir = Environment.GetEnvironmentVariable("NVPN_APP_DATA_DIR");
        if (string.IsNullOrWhiteSpace(dataDir))
        {
            dataDir = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                "Nostr VPN");
        }
        _core = new AppCoreClient(dataDir, version);
        _autoInstallUpdates = LoadAutoInstallUpdates();
        ApplyState(_core.State(), syncDrafts: true);
        SyncLaunchOnStartupRegistration();

        ShowDevicesCommand = new RelayCommand(_ => Page = AppPage.Devices);
        ShowAddNetworkCommand = new RelayCommand(_ =>
        {
            SetNetworkSetupMode("");
            Page = AppPage.AddNetwork;
        });
        ShowAddDeviceCommand = new RelayCommand(
            _ => Page = AppPage.AddDevice,
            _ => ActiveNetwork is { LocalIsAdmin: true, Enabled: true });
        ShowInternetCommand = new RelayCommand(_ => Page = AppPage.Internet);
        ShowPublicExitsCommand = new RelayCommand(_ => Page = AppPage.PublicExits, _ => PaidRouteMarketVisible);
        ShowWalletCommand = new RelayCommand(_ => Page = AppPage.Wallet, _ => PaidRouteMarketVisible);
        ShowSellAccessCommand = new RelayCommand(_ => Page = AppPage.SellAccess, _ => PaidExitSellerVisible);
        ShowSettingsCommand = new RelayCommand(_ => Page = AppPage.Settings);
        RefreshCommand = new AsyncRelayCommand(_ => RefreshAsync(), _ => !ActionInFlight);
        ToggleVpnCommand = new AsyncRelayCommand(_ => ToggleVpnAsync(), _ => !ActionInFlight && State.VpnControlSupported && RuntimeActiveNetwork is not null);
        CopyThisDeviceCommand = new RelayCommand(_ => CopyText(ThisDeviceCopyValue), _ => !string.IsNullOrWhiteSpace(ThisDeviceCopyValue));
        CopyPeerCommand = new RelayCommand(parameter => CopyText(parameter as string ?? ""));
        ImportInviteCommand = new AsyncRelayCommand(_ => ImportInviteAsync(InviteInput), _ => !ActionInFlight && !string.IsNullOrWhiteSpace(InviteInput));
        PasteInviteCommand = new RelayCommand(_ => PasteInviteFromClipboard(), _ => !ActionInFlight);
        ImportJoinRequestQrImageCommand = new AsyncRelayCommand(_ => ImportJoinRequestQrImageAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true);
        ToggleNearbyDiscoveryCommand = new AsyncRelayCommand(_ => DispatchAsync(State.NearbyDiscoveryActive ? NativeActions.StopNearbyDiscovery() : NativeActions.StartNearbyDiscovery(), "Finding nearby"));
        ToggleJoinRequestBroadcastCommand = new AsyncRelayCommand(_ => DispatchAsync(State.InviteBroadcastActive ? NativeActions.StopJoinRequestBroadcast() : NativeActions.StartJoinRequestBroadcast(), State.InviteBroadcastActive ? "Stopping nearby" : "Advertising nearby"));
        AddParticipantCommand = new AsyncRelayCommand(_ => AddParticipantAsync(), _ => !ActionInFlight && ActiveNetwork is { LocalIsAdmin: true, Enabled: true } && !string.IsNullOrWhiteSpace(ParticipantInput) && !ParticipantInputInvalid);
        SaveNodeCommand = new AsyncRelayCommand(_ => SaveNodeAsync(), _ => !ActionInFlight);
        AddRelayCommand = new AsyncRelayCommand(_ => AddRelayAsync(), _ => !ActionInFlight && !string.IsNullOrWhiteSpace(RelayInput));
        SaveRelaysCommand = new AsyncRelayCommand(_ => SaveRelaysAsync(), _ => !ActionInFlight);
        ImportWireGuardExitCommand = new AsyncRelayCommand(_ => ImportWireGuardExitAsync(), _ => !ActionInFlight);
        SaveWireGuardExitCommand = new AsyncRelayCommand(_ => SaveWireGuardExitAsync(), _ => !ActionInFlight);
        CreateNetworkCommand = new AsyncRelayCommand(_ => CreateNetworkAsync(), _ => !ActionInFlight);
        ShowCreateNetworkSetupCommand = new RelayCommand(_ => SetNetworkSetupMode("create"));
        ShowJoinNetworkSetupCommand = new RelayCommand(_ => SetNetworkSetupMode("join"));
        BackToNetworkSetupChoicesCommand = new RelayCommand(_ => SetNetworkSetupMode(""));
        AddNetworkCommand = new AsyncRelayCommand(_ => AddNetworkAsync(), _ => !ActionInFlight && !string.IsNullOrWhiteSpace(NetworkNameInput));
        ManualAddNetworkCommand = new AsyncRelayCommand(
            _ => ManualAddNetworkAsync(),
            _ => !ActionInFlight && CanSubmitManualJoin);
        ToggleManualJoinCommand = new RelayCommand(_ => ManualJoinExpanded = !ManualJoinExpanded);
        ActivateInactiveNetworkCommand = new AsyncRelayCommand(
            parameter => ActivateInactiveNetworkAsync(parameter as string),
            parameter => !ActionInFlight && parameter is string id && !string.IsNullOrWhiteSpace(id));
        SaveNetworkNameCommand = new AsyncRelayCommand(_ => RenameActiveNetworkAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && !string.IsNullOrWhiteSpace(NetworkNameDraft));
        SaveNetworkMeshIdCommand = new AsyncRelayCommand(_ => SaveActiveNetworkMeshIdAsync(), _ => !ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && !string.IsNullOrWhiteSpace(NetworkMeshIdDraft));
        CopyNetworkIdCommand = new RelayCommand(_ => CopyText(ActiveNetwork?.NetworkId ?? ""), _ => !string.IsNullOrWhiteSpace(ActiveNetwork?.NetworkId));
        InstallServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.InstallSystemService(), "Installing service"), _ => !ActionInFlight && State.ServiceSupported);
        EnableServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.EnableSystemService(), "Enabling service"), _ => !ActionInFlight && State.ServiceEnablementSupported);
        DisableServiceCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.DisableSystemService(), "Disabling service"), _ => !ActionInFlight && State.ServiceEnablementSupported);
        InstallCliCommand = new AsyncRelayCommand(_ => DispatchAsync(NativeActions.InstallCli(), "Installing CLI"), _ => !ActionInFlight && State.CliInstallSupported);
        CheckUpdatesCommand = new AsyncRelayCommand(_ => CheckUpdatesAsync(), _ => !UpdateChecking && !UpdateInstalling);
        InstallUpdateCommand = new AsyncRelayCommand(_ => InstallUpdateAsync(), _ => UpdateInstallEnabled);

        StartupService.RegisterDeepLinkProtocol();

        _refreshTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2) };
        _refreshTimer.Tick += async (_, _) => await RefreshAsync();
        _refreshTimer.Start();
        _ = CheckUpdatesAsync(manual: false);

        _updateTimer = new DispatcherTimer { Interval = UpdatePollInterval };
        _updateTimer.Tick += async (_, _) => await CheckUpdatesAsync(manual: false);
        _updateTimer.Start();
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public NativeAppState State
    {
        get => _state;
        private set
        {
            _state = value;
            if (!PageIsVisible(_page))
            {
                _page = AppPage.Devices;
                OnPropertyChanged(nameof(Page));
            }
            OnPropertyChanged();
            RaiseDerivedStateChanged();
        }
    }

    public AppPage Page
    {
        get => _page;
        set
        {
            var nextPage = PageIsVisible(value) ? value : AppPage.Devices;
            if (_page == nextPage)
            {
                return;
            }
            if (_page == AppPage.AddNetwork && nextPage != AppPage.AddNetwork)
            {
                SetNetworkSetupMode("");
            }
            _page = nextPage;
            OnPropertyChanged();
        }
    }

    public bool ActionInFlight
    {
        get => _actionInFlight;
        private set
        {
            _actionInFlight = value;
            OnPropertyChanged();
            OnPropertyChanged(nameof(PublicFipsRoutingEnabled));
            RaisePaidRouteWalletInputStateChanged();
            CommandManager.InvalidateRequerySuggested();
        }
    }

    public string Notice
    {
        get => _notice;
        private set
        {
            _notice = value;
            OnPropertyChanged();
        }
    }

    public string InviteInput
    {
        get => _inviteInput;
        set
        {
            if (!SetField(ref _inviteInput, value))
            {
                return;
            }
            CommandManager.InvalidateRequerySuggested();
            // Auto-import as soon as the field contains a recognisable invite —
            // saves the user a click and matches "paste and you're in" mental
            // model. We only fire on full nvpn:// URLs so partial typing
            // doesn't trigger; the import itself clears the field.
            var trimmed = (value ?? string.Empty).Trim();
            if (!ActionInFlight && LooksLikeInviteCode(trimmed))
            {
                _ = ImportInviteAsync(trimmed);
            }
        }
    }
    public string JoinRequestInput
    {
        get => _joinRequestInput;
        set
        {
            if (!SetField(ref _joinRequestInput, value))
            {
                return;
            }
            CommandManager.InvalidateRequerySuggested();
            var trimmed = (value ?? string.Empty).Trim();
            if (!ActionInFlight && ActiveNetwork?.LocalIsAdmin == true && LooksLikeJoinRequest(trimmed))
            {
                _ = ConfirmAndImportJoinRequestAsync(trimmed);
            }
        }
    }
    public string ParticipantInput
    {
        get => _participantInput;
        set
        {
            if (SetField(ref _participantInput, value))
            {
                OnPropertyChanged(nameof(ParticipantInputInvalid));
                OnPropertyChanged(nameof(ParticipantInputErrorVisibility));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }
    public string ParticipantAliasInput { get => _participantAliasInput; set => SetField(ref _participantAliasInput, value); }
    public bool ParticipantInputInvalid
    {
        get
        {
            var trimmed = (_participantInput ?? string.Empty).Trim();
            return trimmed.Length > 0 && !IsValidDeviceId(trimmed);
        }
    }
    public Visibility ParticipantInputErrorVisibility => ParticipantInputInvalid ? Visibility.Visible : Visibility.Collapsed;

    public string ManualJoinAdminId
    {
        get => _manualJoinAdminId;
        set
        {
            if (SetField(ref _manualJoinAdminId, value))
            {
                OnPropertyChanged(nameof(ManualJoinAdminInvalid));
                OnPropertyChanged(nameof(ManualJoinAdminErrorVisibility));
                OnPropertyChanged(nameof(CanSubmitManualJoin));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public string ManualJoinMeshId
    {
        get => _manualJoinMeshId;
        set
        {
            if (SetField(ref _manualJoinMeshId, value))
            {
                OnPropertyChanged(nameof(CanSubmitManualJoin));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public bool ManualJoinExpanded
    {
        get => _manualJoinExpanded;
        set
        {
            if (SetField(ref _manualJoinExpanded, value))
            {
                OnPropertyChanged(nameof(ManualJoinExpanderToggleText));
            }
        }
    }

    public string ManualJoinExpanderToggleText => ManualJoinExpanded ? "Add manually ▴" : "Add manually ▾";

    public bool ManualJoinAdminInvalid
    {
        get
        {
            var trimmed = (_manualJoinAdminId ?? string.Empty).Trim();
            return trimmed.Length > 0 && !IsValidDeviceId(trimmed);
        }
    }

    public Visibility ManualJoinAdminErrorVisibility => ManualJoinAdminInvalid ? Visibility.Visible : Visibility.Collapsed;

    public bool CanSubmitManualJoin
    {
        get
        {
            var admin = (_manualJoinAdminId ?? string.Empty).Trim();
            var mesh = (_manualJoinMeshId ?? string.Empty).Trim();
            return admin.Length > 0 && mesh.Length > 0 && !ManualJoinAdminInvalid;
        }
    }

    public string NetworkNameInput { get => _networkNameInput; set => SetField(ref _networkNameInput, value); }
    public string NetworkNameDraft
    {
        get => _networkNameDraft;
        set
        {
            if (SetField(ref _networkNameDraft, value))
            {
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }
    public string NetworkMeshIdDraft
    {
        get => _networkMeshIdDraft;
        set
        {
            if (SetField(ref _networkMeshIdDraft, value))
            {
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }
    public string NodeName { get => _nodeName; set => SetField(ref _nodeName, value); }
    public string Endpoint { get => _endpoint; set => SetField(ref _endpoint, value); }
    public string TunnelIp { get => _tunnelIp; set => SetField(ref _tunnelIp, value); }
    public string ListenPort { get => _listenPort; set => SetField(ref _listenPort, value); }
    public string RelayInput
    {
        get => _relayInput;
        set
        {
            if (SetField(ref _relayInput, value))
            {
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }
    public string RelaysDraft { get => _relaysDraft; set => SetField(ref _relaysDraft, value); }
    public string FipsHostInboundTcpPorts { get => _fipsHostInboundTcpPorts; set => SetField(ref _fipsHostInboundTcpPorts, value); }
    public string AdvertisedRoutes { get => _advertisedRoutes; set => SetField(ref _advertisedRoutes, value); }
    public string WireguardExitConfig { get => _wireguardExitConfig; set => SetField(ref _wireguardExitConfig, value); }

    public string PaidRouteMintUrl
    {
        get => _paidRouteMintUrl;
        set
        {
            if (SetField(ref _paidRouteMintUrl, value))
            {
                OnPropertyChanged(nameof(CanAddPaidRouteWalletMint));
            }
        }
    }

    public string PaidRouteTopUpAmount
    {
        get => _paidRouteTopUpAmount;
        set
        {
            if (SetField(ref _paidRouteTopUpAmount, value))
            {
                OnPropertyChanged(nameof(CanTopUpPaidRouteWallet));
            }
        }
    }

    public string PaidRouteSendAmount
    {
        get => _paidRouteSendAmount;
        set
        {
            if (SetField(ref _paidRouteSendAmount, value))
            {
                OnPropertyChanged(nameof(CanSendPaidRouteWalletToken));
            }
        }
    }

    public string PaidRouteReceiveToken
    {
        get => _paidRouteReceiveToken;
        set
        {
            if (SetField(ref _paidRouteReceiveToken, value))
            {
                OnPropertyChanged(nameof(CanReceivePaidRouteWalletToken));
            }
        }
    }

    public string PaidRouteWithdrawInvoice
    {
        get => _paidRouteWithdrawInvoice;
        set
        {
            if (SetField(ref _paidRouteWithdrawInvoice, value))
            {
                OnPropertyChanged(nameof(CanWithdrawPaidRouteWalletLightning));
            }
        }
    }

    public bool CanAddPaidRouteWalletMint =>
        PaidRouteMarketVisible
        && !ActionInFlight
        && !string.IsNullOrWhiteSpace(PaidRouteMintUrl);

    public bool CanTopUpPaidRouteWallet =>
        PaidRouteMarketVisible
        && !ActionInFlight
        && ParsePositiveUInt64(PaidRouteTopUpAmount) is not null;

    public bool CanSendPaidRouteWalletToken =>
        PaidRouteMarketVisible
        && !ActionInFlight
        && ParsePositiveUInt64(PaidRouteSendAmount) is not null;

    public bool CanReceivePaidRouteWalletToken =>
        PaidRouteMarketVisible
        && !ActionInFlight
        && !string.IsNullOrWhiteSpace(PaidRouteReceiveToken);

    public bool CanWithdrawPaidRouteWalletLightning =>
        PaidRouteMarketVisible
        && !ActionInFlight
        && !string.IsNullOrWhiteSpace(PaidRouteWithdrawInvoice);

    public bool PaidRouteMarketVisible => State.PaidRouteMarket.Supported;

    public bool PaidExitSellerVisible => State.PaidExitSeller.Supported;

    // Bullet-style radio indicators next to each exit-node row.
    public string DirectExitMarker =>
        State.InternetSource == "direct" ? "●" : "○";

    public string WireguardExitMarker => State.InternetSource == "wireguard" ? "●" : "○";

    public string PaidAutomaticExitMarker => State.InternetSource == "paid_automatic" ? "●" : "○";

    public string PaidManualExitMarker => State.InternetSource == "paid_manual" ? "●" : "○";

    public IEnumerable<NativeParticipantState> ExitNodeParticipants =>
        (ActiveNetwork?.Participants ?? [])
            .Where(participant => participant.OffersExitNode && !participant.IsSelf)
            .OrderBy(participant => participant.DisplayName, StringComparer.OrdinalIgnoreCase);

    public string WireguardExitSubtitle
    {
        get
        {
            if (!State.WireguardExitConfigured)
            {
                return "No WireGuard config saved yet";
            }
            return string.IsNullOrWhiteSpace(State.WireguardExitEndpoint)
                ? "Configured"
                : State.WireguardExitEndpoint;
        }
    }

    public string PaidRouteWalletBalanceText =>
        TextOr(State.PaidRouteMarket.Wallet.TotalBalanceText, FormatPaidRouteMsat(State.PaidRouteMarket.Wallet.TotalBalanceMsat));

    public string WalletNavigationText => string.IsNullOrWhiteSpace(State.PaidRouteMarket.Wallet.NavigationBalanceText)
        ? "Wallet"
        : $"Wallet {State.PaidRouteMarket.Wallet.NavigationBalanceText}";

    public string PaidRouteWalletFiatText => State.PaidRouteMarket.Wallet.FiatBalanceText;

    public string PaidRouteWalletRateText => string.IsNullOrWhiteSpace(State.PaidRouteMarket.Wallet.ExchangeRateText)
        ? ""
        : $"{State.PaidRouteMarket.Wallet.ExchangeRateText} · {State.PaidRouteMarket.Wallet.ExchangeRateSources}";

    public string PaidRouteMarketStatusText => string.IsNullOrWhiteSpace(State.PaidRouteMarket.StatusText)
        ? $"{State.PaidRouteMarket.Offers.Count} internet sellers"
        : State.PaidRouteMarket.StatusText;

    public string PaidExitSellerStatusText
    {
        get
        {
            if (!string.IsNullOrWhiteSpace(State.PaidExitSeller.StatusText))
            {
                return State.PaidExitSeller.StatusText
                    .Replace("Paid exit selling", "Selling internet")
                    .Replace("paid exit selling", "selling internet");
            }
            return State.PaidExitSeller.Supported
                ? "People can pay to use my internet"
                : "This platform cannot sell public internet access";
        }
    }

    public string PaidExitSellerSummary =>
        $"{TextOr(State.PaidExitSeller.CountryCode, "Country unset")} · {NativeDisplayText.NetworkClassTitle(State.PaidExitSeller.NetworkClass)} · {TextOr(State.PaidExitSeller.PriceText, NativeDisplayText.PriceText(State.PaidExitSeller.PriceMsat, State.PaidExitSeller.PerUnits, State.PaidExitSeller.Meter, State.PaidExitSeller.PerUnitsText))}";

    public string PaidExitSellerTrialText =>
        $"Free test: {TextOr(State.PaidExitSeller.FreeProbeText, NativeDisplayText.TrafficUnitText(State.PaidExitSeller.FreeProbeUnits, State.PaidExitSeller.Meter))} · Grace: {TextOr(State.PaidExitSeller.GraceText, NativeDisplayText.TrafficUnitText(State.PaidExitSeller.GraceUnits, State.PaidExitSeller.Meter))}";

    public string PaidExitSellerChannelExpiryText =>
        $"Channel expires: {TextOr(State.PaidExitSeller.ChannelExpiryText, FormatRemaining(State.PaidExitSeller.ChannelExpirySecs))}";

    public string PaidExitSellerSettlementText => State.PaidExitSeller.SettlementText;

    public string PaidExitSellerPublicIpText => string.IsNullOrWhiteSpace(State.PaidExitSeller.PublicIpText)
        ? ""
        : $"Public IP: {State.PaidExitSeller.PublicIpText}";

    public string PaidExitSellerChannelCreditText =>
        $"{TextOr(State.PaidExitSeller.ChannelCreditTitleText, "Pending buyer credit")}: {TextOr(State.PaidExitSeller.ChannelCreditText, FormatPaidRouteMsat(State.PaidExitSeller.ChannelCreditMsat))}";

    public string PaidExitSellerChannelCreditHelpText => TextOr(
        State.PaidExitSeller.ChannelCreditHelpText,
        State.PaidExitSeller.ChannelCreditMsat > 0 ? "Collect to move it into wallet" : ""
    );

    public string PaidExitSellerPaymentStatusText
    {
        get
        {
            var action = State.PaidRouteMarket.LastPaymentAction;
            if (string.IsNullOrWhiteSpace(action.Kind) && string.IsNullOrWhiteSpace(action.StatusText))
            {
                return "";
            }
            return $"Payments: {TextOr(action.StatusText, NativeDisplayText.PaymentActionTitle(action.Kind))}";
        }
    }

    public string PaidExitSellerMintsText => State.PaidExitSeller.AcceptedMints.Count == 0
        ? "No accepted mints configured"
        : $"Accepted mints: {string.Join(", ", State.PaidExitSeller.AcceptedMints)}";

    public string PaidExitSellerSessionsText
    {
        get
        {
            var seller = State.PaidExitSeller;
            var pieces = new List<string>
            {
                $"{seller.CurrentConnectionCount} connected",
                $"{seller.PastConnectionCount} past",
                TextOr(seller.TotalTrafficText, $"{NativeDisplayText.FormatBytes(seller.TotalBillableBytes)} routed"),
                $"{TextOr(seller.TotalPaidText, FormatPaidRouteMsat(seller.TotalPaidMsat))} paid",
                $"{TextOr(seller.TotalDueText, FormatPaidRouteMsat(seller.TotalDueMsat))} due",
            };
            if (seller.TotalUnpaidMsat > 0)
            {
                pieces.Add($"{TextOr(seller.TotalUnpaidText, FormatPaidRouteMsat(seller.TotalUnpaidMsat))} behind");
            }
            return string.Join(" · ", pieces);
        }
    }

    public IEnumerable<string> PaidRouteWalletMintRows => State.PaidRouteMarket.Wallet.Mints.Select(mint =>
    {
        var marker = mint.IsDefault ? "default" : "mint";
        var balance = mint.BalanceKnown
            ? $" · {TextOr(mint.BalanceText, FormatPaidRouteMsat(mint.BalanceMsat))}"
            : "";
        return $"{marker} · {TextOr(mint.Label, mint.Url)}{balance}";
    });

    public string PaidRouteWalletMintsStatusText => State.PaidRouteMarket.Wallet.Mints.Count == 0
        ? "No wallet mints"
        : "";

    public bool PaidRouteHasStreamablePayments =>
        State.PaidRouteMarket.Sessions.Any(PaidRouteSessionCanSignPayment);

    public bool UpdateChecking
    {
        get => _updateChecking;
        private set
        {
            if (SetField(ref _updateChecking, value))
            {
                OnPropertyChanged(nameof(UpdateInstallEnabled));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public bool UpdateInstalling
    {
        get => _updateInstalling;
        private set
        {
            if (SetField(ref _updateInstalling, value))
            {
                OnPropertyChanged(nameof(UpdateInstallEnabled));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public bool UpdateAvailable
    {
        get => _updateAvailable;
        private set
        {
            if (SetField(ref _updateAvailable, value))
            {
                OnPropertyChanged(nameof(UpdateStripeText));
                OnPropertyChanged(nameof(UpdateInstallEnabled));
                CommandManager.InvalidateRequerySuggested();
            }
        }
    }

    public string UpdateVersion
    {
        get => _updateVersion;
        private set
        {
            if (SetField(ref _updateVersion, value))
            {
                OnPropertyChanged(nameof(UpdateStripeText));
            }
        }
    }

    public string UpdateStatus
    {
        get => _updateStatus;
        private set => SetField(ref _updateStatus, value);
    }

    public bool AutoInstallUpdates
    {
        get => _autoInstallUpdates;
        set
        {
            if (!SetField(ref _autoInstallUpdates, value))
            {
                return;
            }
            SaveAutoInstallUpdates(value);
            if (value && UpdateInstallEnabled)
            {
                _ = InstallUpdateAsync();
            }
        }
    }

    public bool UpdateInstallEnabled => UpdateAvailable && _updateUsesCoreDownload && !UpdateChecking && !UpdateInstalling;

    public string UpdateStripeText => string.IsNullOrWhiteSpace(State.AppVersion)
        ? $"Update available: {UpdateVersion}"
        : $"Update available: {UpdateVersion} (you're on {State.AppVersion})";

    public QrMatrix JoinRequestQr
    {
        get => _joinRequestQr;
        private set => SetField(ref _joinRequestQr, value);
    }

    public string JoinRequestQrCodeOrLink => State.JoinRequestQrCodeOrLink;
    public bool ShowNetworkSetupChoices => string.IsNullOrWhiteSpace(_networkSetupMode);
    public bool ShowNetworkSetupCreate => _networkSetupMode == "create";
    public bool ShowNetworkSetupJoin => _networkSetupMode == "join";

    private void SetNetworkSetupMode(string mode)
    {
        if (_networkSetupMode == mode)
        {
            return;
        }
        _networkSetupMode = mode;
        OnPropertyChanged(nameof(ShowNetworkSetupChoices));
        OnPropertyChanged(nameof(ShowNetworkSetupCreate));
        OnPropertyChanged(nameof(ShowNetworkSetupJoin));
    }

    private NativeNetworkState? RuntimeActiveNetwork => State.Networks.FirstOrDefault(network => network.Enabled);
    public bool HasEnabledNetwork => RuntimeActiveNetwork is not null;
    public NativeNetworkState? ActiveNetwork =>
        State.Networks.FirstOrDefault(network => network.Id == _shownNetworkId)
        ?? RuntimeActiveNetwork
        ?? State.Networks.FirstOrDefault();
    public bool HasActiveNetwork => ActiveNetwork is not null;
    public bool ShowJoinRequestQr => !HasEnabledNetwork && !string.IsNullOrWhiteSpace(State.JoinRequestQrCodeOrLink);
    public string OfferExitNodeLabel
    {
        get
        {
            var name = string.IsNullOrWhiteSpace(ActiveNetwork?.Name) ? "this network" : ActiveNetwork!.Name;
            return $"Share this device's internet in {name}";
        }
    }
    public IEnumerable<NativeNetworkState> InactiveNetworks => State.Networks.Where(network => !network.Enabled);
    public NativeParticipantState? SelectedParticipant
    {
        get
        {
            var network = ActiveNetwork;
            return network is null ? null : ResolveSelectedParticipant(network);
        }
        set
        {
            var nextKey = value is null ? "" : ParticipantKey(value);
            SetSelectedParticipantKey(nextKey, ignoreTransientClear: true);
        }
    }
    public string SelectedParticipantKey
    {
        get => _selectedParticipantKey;
        set => SetSelectedParticipantKey(value, ignoreTransientClear: true);
    }
    public bool SelectedParticipantCanRename => ActiveNetwork?.LocalIsAdmin == true
        && SelectedParticipant is not null;
    public bool SelectedParticipantCanManage => ActiveNetwork?.LocalIsAdmin == true
        && SelectedParticipant is { IsSelf: false };
    public string ActiveNetworkName => DisplayNetworkName(ActiveNetwork);
    public string PublicFipsAddress => string.IsNullOrWhiteSpace(State.OwnNpub) ? "" : $"{State.OwnNpub}.fips";
    public bool PublicFipsRoutingEnabled => State.FipsHostTunnelEnabled && !ActionInFlight;
    public Brush ShownNetworkStatusBrush => ActiveNetwork?.Enabled == true ? ActiveNetworkBrush : InactiveNetworkBrush;
    public bool ShowNetworkStatusDot => State.Networks.Count > 1;
    public bool ShowLinkDeviceCard => ActiveNetwork?.Enabled == true;
    public string ActiveNetworkDisplayNetworkId => DisplayNetworkId(ActiveNetwork?.NetworkId ?? "");
    public string HeroSubtitle => $"{State.ConnectedPeerCount} of {State.ExpectedPeerCount} connected";
    public string VpnButtonText => State.VpnEnabled ? "On" : "Off";
    /// Mirrors `AppManager.vpnStatusText` on macOS so the header and the tray
    /// surface the same status string across platforms.
    public string VpnStatusText
    {
        get
        {
            if (!string.IsNullOrWhiteSpace(State.Error))
            {
                return State.Error;
            }
            if (State.ExitNodeBlocked)
            {
                return string.IsNullOrWhiteSpace(State.ExitNodeStatusText)
                    ? "Internet blocked"
                    : State.ExitNodeStatusText;
            }
            if (State.ExitNodeActive && !string.IsNullOrWhiteSpace(State.ExitNodeStatusText))
            {
                return State.ExitNodeStatusText;
            }
            if (State.VpnActive)
            {
                return string.IsNullOrWhiteSpace(State.VpnStatus) ? "VPN on" : State.VpnStatus;
            }
            if (State.VpnEnabled)
            {
                return string.IsNullOrWhiteSpace(State.VpnStatus) ? "Turning on" : State.VpnStatus;
            }
            if (DaemonStarting(State))
            {
                return State.VpnStatus;
            }
            return "Off";
        }
    }
    public Brush VpnStatusBrush => State.ExitNodeBlocked ? HeaderDangerBrush : TextSecondaryBrush;

    private static bool DaemonStarting(NativeAppState state) =>
        string.IsNullOrWhiteSpace(state.Error)
        && state.ServiceRunning
        && string.Equals(
            state.VpnStatus.Trim(),
            "Background service starting",
            StringComparison.Ordinal);

    public string ThisDeviceCopyValue => !string.IsNullOrWhiteSpace(State.OwnNpub) ? State.OwnNpub : State.TunnelIp;
    public Visibility NoNearbyInvitesNoticeVisibility => State.NearbyDiscoveryActive && State.LanPeers.Count == 0
        ? Visibility.Visible
        : Visibility.Collapsed;
    public string NearbyDiscoveryButtonText => State.NearbyDiscoveryActive
        ? $"Finding nearby · {FormatRemaining(State.NearbyDiscoveryRemainingSecs)}"
        : "Find nearby";
    public string JoinRequestBroadcastButtonText => State.InviteBroadcastActive
        ? $"Advertising · {FormatRemaining(State.InviteBroadcastRemainingSecs)}"
        : "Advertise nearby";

    private static string FormatRemaining(ulong seconds)
    {
        if (seconds == 0)
        {
            return "off";
        }
        var days = seconds / 86_400;
        if (days > 0)
        {
            var hours = seconds % 86_400 / 3_600;
            return hours == 0 ? $"{days}d" : $"{days}d {hours}h";
        }
        var hoursOnly = seconds / 3_600;
        if (hoursOnly > 0)
        {
            var minutesOnly = seconds % 3_600 / 60;
            return minutesOnly == 0 ? $"{hoursOnly}h" : $"{hoursOnly}h {minutesOnly}m";
        }
        var minutes = seconds / 60;
        if (minutes == 0)
        {
            return $"{seconds}s";
        }
        var remSecs = seconds % 60;
        return remSecs == 0 ? $"{minutes}m" : $"{minutes}m{remSecs:D2}s";
    }

    private static string FormatPaidRouteMsat(ulong msat)
    {
        if (msat == 0)
        {
            return "0 sat";
        }
        var whole = msat / 1_000;
        var remainder = msat % 1_000;
        return remainder == 0 ? $"{whole} sat" : $"{whole}.{remainder:D3} sat";
    }

    private static bool PaidRouteSessionCanSignPayment(NativePaidRouteSessionState session) =>
        session.CanPay;
    public string ServiceSummary => State.ServiceInstalled ? "Service installed" : "Service missing";
    public string CliSummary => State.CliInstalled ? "CLI installed" : "CLI missing";
    public string SystemVersionLabel
    {
        get
        {
            var app = State.AppVersion.Trim();
            var daemon = State.DaemonBinaryVersion.Trim();
            return (string.IsNullOrEmpty(app), string.IsNullOrEmpty(daemon)) switch
            {
                (true, true) => "",
                (false, true) => $"gui v{app}",
                (true, false) => $"daemon v{daemon}",
                (false, false) when app == daemon => $"v{app}",
                _ => $"gui v{app} · daemon v{daemon}",
            };
        }
    }
    public string DiagnosticsInterface => string.IsNullOrWhiteSpace(State.Network.DefaultInterface) ? "unknown" : State.Network.DefaultInterface;
    public string DiagnosticsIpv4 => string.IsNullOrWhiteSpace(State.Network.PrimaryIpv4) ? "-" : State.Network.PrimaryIpv4;
    public string DiagnosticsIpv6 => string.IsNullOrWhiteSpace(State.Network.PrimaryIpv6) ? "-" : State.Network.PrimaryIpv6;
    public string DiagnosticsGateway => FirstNonEmpty(State.Network.GatewayIpv4, State.Network.GatewayIpv6, "unknown");
    public string DiagnosticsMapping => string.IsNullOrWhiteSpace(State.PortMapping.ActiveProtocol) ? "none" : State.PortMapping.ActiveProtocol;
    public string DiagnosticsExternal => string.IsNullOrWhiteSpace(State.PortMapping.ExternalEndpoint) ? "stun/direct" : State.PortMapping.ExternalEndpoint;
    public string DiagnosticsPeers => $"{State.ConnectedPeerCount}/{State.ExpectedPeerCount}";
    public string DiagnosticsFips => $"{State.FipsConnectedPeerCount}/{State.FipsRosterPeerCount} direct";
    public string DiagnosticsOtherFips => $"{State.NonFipsRosterPeerCount}";
    public ICommand ShowDevicesCommand { get; }
    public ICommand ShowAddNetworkCommand { get; }
    public ICommand ShowAddDeviceCommand { get; }
    public ICommand ShowInternetCommand { get; }
    public ICommand ShowPublicExitsCommand { get; }
    public ICommand ShowWalletCommand { get; }
    public ICommand ShowSellAccessCommand { get; }
    public ICommand ShowSettingsCommand { get; }
    public ICommand RefreshCommand { get; }
    public ICommand ToggleVpnCommand { get; }
    public ICommand CopyThisDeviceCommand { get; }
    public ICommand CopyPeerCommand { get; }
    public ICommand ImportInviteCommand { get; }
    public ICommand PasteInviteCommand { get; }
    public ICommand ImportJoinRequestQrImageCommand { get; }
    public ICommand ToggleNearbyDiscoveryCommand { get; }
    public ICommand ToggleJoinRequestBroadcastCommand { get; }
    public ICommand AddParticipantCommand { get; }
    public ICommand SaveNodeCommand { get; }
    public ICommand AddRelayCommand { get; }
    public ICommand SaveRelaysCommand { get; }
    public ICommand ImportWireGuardExitCommand { get; }
    public ICommand SaveWireGuardExitCommand { get; }
    public ICommand CreateNetworkCommand { get; }
    public ICommand ShowCreateNetworkSetupCommand { get; }
    public ICommand ShowJoinNetworkSetupCommand { get; }
    public ICommand BackToNetworkSetupChoicesCommand { get; }
    public ICommand AddNetworkCommand { get; }
    public ICommand ManualAddNetworkCommand { get; }
    public ICommand ToggleManualJoinCommand { get; }
    public ICommand ActivateInactiveNetworkCommand { get; }
    public ICommand SaveNetworkNameCommand { get; }
    public ICommand SaveNetworkMeshIdCommand { get; }
    public ICommand CopyNetworkIdCommand { get; }
    public ICommand InstallServiceCommand { get; }
    public ICommand EnableServiceCommand { get; }
    public ICommand DisableServiceCommand { get; }
    public ICommand InstallCliCommand { get; }
    public ICommand CheckUpdatesCommand { get; }
    public ICommand InstallUpdateCommand { get; }

    public async Task RefreshAsync()
    {
        if (ActionInFlight)
        {
            return;
        }
        try
        {
            var state = await Task.Run(_core.Refresh);
            ApplyState(state, syncDrafts: false);
        }
        catch (Exception error)
        {
            Notice = error.Message;
        }
    }

    public Task ToggleVpnAsync()
    {
        return DispatchAsync(
            State.VpnEnabled ? NativeActions.DisconnectVpn() : NativeActions.ConnectVpn(),
            State.VpnEnabled ? "Turning VPN off" : "Turning VPN on");
    }

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

    public Task DiscoverPaidRouteOffersAsync()
    {
        return DispatchAsync(NativeActions.DiscoverPaidRouteOffers(), "Finding sellers");
    }

    public Task RefreshPaidRouteWalletAsync()
    {
        return DispatchAsync(NativeActions.RefreshPaidRouteWallet(), "Refreshing wallet");
    }

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

    public Task BuyPaidRouteOfferAsync(NativePaidRouteOfferState offer)
    {
        return string.IsNullOrWhiteSpace(offer.Key)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.BuyPaidRouteOffer(offer.Key), "Connecting");
    }

    public Task SelectPaidRouteSessionAsync(NativePaidRouteSessionState session)
    {
        return string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.SelectPaidRouteSession(session.SessionId, true), "Connecting");
    }

    public Task ProbePaidRouteSessionAsync(NativePaidRouteSessionState session)
    {
        return string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.ProbePaidRouteSession(session.SessionId), "Checking connection");
    }

    public Task OpenPaidRouteChannelAsync(NativePaidRouteSessionState session)
    {
        return string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.OpenPaidRouteChannelFromWallet(session.SessionId), "Funding seller");
    }

    public Task SignPaidRoutePaymentAsync(NativePaidRouteSessionState session)
    {
        return string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.SignPaidRoutePaymentEnvelopeFromWallet(session.SessionId), "Paying seller");
    }

    public Task ClosePaidRouteChannelAsync(NativePaidRouteSessionState session)
    {
        return string.IsNullOrWhiteSpace(session.SessionId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.ClosePaidRouteChannelFromWallet(session.SessionId), "Settling channel");
    }

    public Task SendPaidRoutePaymentEnvelopeAsync()
    {
        var envelope = State.PaidRouteMarket.LastPaymentAction.EnvelopeJson.Trim();
        return string.IsNullOrWhiteSpace(envelope)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.SendPaidRoutePaymentEnvelope(envelope), "Sending payment");
    }

    public Task StreamPaidRoutePaymentsAsync()
    {
        return DispatchAsync(NativeActions.StreamPaidRoutePayments(), "Paying for usage");
    }

    public Task SetPaidExitEnabledAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch
            {
                PaidExitEnabled = enabled,
            }),
            "Saving listing");
    }

    public Task PublishPaidExitOfferAsync()
    {
        return DispatchAsync(NativeActions.PublishPaidExitOffer(), "Advertising listing");
    }

    public Task ReceivePaidRoutePaymentsAsync()
    {
        return DispatchAsync(NativeActions.ReceivePaidRoutePayments(), "Checking payments");
    }

    public Task SetLaunchOnStartupAsync(bool enabled)
    {
        try
        {
            StartupService.SetLaunchOnStartup(enabled);
        }
        catch (Exception error)
        {
            Notice = error.Message;
            return Task.CompletedTask;
        }
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { LaunchOnStartup = enabled }),
            "Saving startup");
    }

    private void SyncLaunchOnStartupRegistration()
    {
        try
        {
            StartupService.SyncLaunchOnStartup(State.StartupSettingsSupported && State.LaunchOnStartup);
        }
        catch (Exception error)
        {
            Notice = error.Message;
        }
    }

    public Task SetCloseToTrayAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { CloseToTrayOnClose = enabled }),
            "Saving tray behavior");
    }

    public Task SetAutoconnectAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { Autoconnect = enabled }),
            "Saving VPN option");
    }

    public Task SetFipsHostTunnelAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { FipsHostTunnelEnabled = enabled }),
            "Saving FIPS option");
    }

    public Task SetConnectToNonRosterFipsPeersAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { ConnectToNonRosterFipsPeers = enabled }),
            "Saving FIPS option");
    }

    public Task SetFipsNostrDiscoveryEnabledAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { FipsNostrDiscoveryEnabled = enabled }),
            "Saving FIPS option");
    }

    public Task SetFipsBootstrapEnabledAsync(bool enabled)
    {
        return DispatchAsync(
            NativeActions.UpdateSettings(new SettingsPatch { FipsBootstrapEnabled = enabled }),
            "Saving FIPS option");
    }

    public Task RemoveParticipantAsync(NativeParticipantState participant)
    {
        var network = ActiveNetwork;
        if (network?.LocalIsAdmin != true || participant.IsSelf)
        {
            return Task.CompletedTask;
        }
        return DispatchAsync(NativeActions.RemoveParticipant(network.Id, participant.Npub), "Removing device");
    }

    public Task ToggleAdminAsync(NativeParticipantState participant)
    {
        var network = ActiveNetwork;
        if (network?.LocalIsAdmin != true || participant.IsSelf)
        {
            return Task.CompletedTask;
        }
        return DispatchAsync(
            participant.IsAdmin
                ? NativeActions.RemoveAdmin(network.Id, participant.Npub)
                : NativeActions.AddAdmin(network.Id, participant.Npub),
            participant.IsAdmin ? "Removing admin" : "Adding admin");
    }

    public Task ActivateNetworkAsync(string networkId)
    {
        return DispatchAsync(NativeActions.SetNetworkEnabled(networkId, true), "Activating network");
    }

    public void SelectShownNetwork(string networkId)
    {
        if (_shownNetworkId == networkId)
        {
            return;
        }
        _shownNetworkId = networkId;
        NormalizeSelectedParticipant(State);
        SyncDrafts(State);
        RaiseDerivedStateChanged();
        CommandManager.InvalidateRequerySuggested();
    }

    public Task RemoveNetworkAsync(string networkId)
    {
        return DispatchAsync(NativeActions.RemoveNetwork(networkId), "Deleting network");
    }

    public Task ImportNearbyJoinRequestAsync(string? request)
    {
        return string.IsNullOrWhiteSpace(request)
            ? Task.CompletedTask
            : ConfirmAndImportJoinRequestAsync(request.Trim());
    }

    public Task RenameActiveNetworkAsync()
    {
        var network = ActiveNetwork;
        var name = NetworkNameDraft.Trim();
        return network is null || string.IsNullOrWhiteSpace(name)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.RenameNetwork(network.Id, name), "Renaming network");
    }

    public Task SaveActiveNetworkMeshIdAsync()
    {
        var network = ActiveNetwork;
        var meshId = NormalizeNetworkIdInput(NetworkMeshIdDraft);
        return network is null || string.IsNullOrWhiteSpace(meshId)
            ? Task.CompletedTask
            : DispatchAsync(NativeActions.SetNetworkMeshId(network.Id, meshId), "Saving network ID");
    }

    public Task SetParticipantAliasAsync(NativeParticipantState participant, string alias)
    {
        return ActiveNetwork?.LocalIsAdmin == true
            ? DispatchAsync(NativeActions.SetParticipantAlias(participant.Npub, alias.Trim()), "Saving alias")
            : Task.CompletedTask;
    }

    public Task SetParticipantEndpointHintsAsync(NativeParticipantState participant, string hints)
    {
        if (ActiveNetwork?.LocalIsAdmin != true || participant.IsSelf)
        {
            return Task.CompletedTask;
        }
        var parsed = (hints ?? string.Empty)
            .Split([',', '\n', '\r', '\t', ' '], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .Where(value => !string.IsNullOrWhiteSpace(value))
            .Distinct(StringComparer.Ordinal)
            .ToList();
        return DispatchAsync(
            NativeActions.SetParticipantEndpointHints(participant.Npub, parsed),
            "Saving address hints");
    }

    public void CopyText(string value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return;
        }
        Clipboard.SetText(value);
        Notice = "Copied";
    }

    public async Task CheckUpdatesAsync(bool manual = true)
    {
        if (UpdateChecking || UpdateInstalling)
        {
            return;
        }
        UpdateChecking = true;
        if (manual)
        {
            UpdateStatus = "Checking for updates";
        }
        try
        {
            var result = await _updateService.CheckAsync(State.AppVersion, State.ConfigPath);
            UpdateAvailable = result.Available;
            UpdateVersion = result.Tag;
            _updateAssetUrl = result.Available ? result.AssetUrl : null;
            _updateUsesCoreDownload = result.Available && result.UseCoreDownload;
            OnPropertyChanged(nameof(UpdateInstallEnabled));
            CommandManager.InvalidateRequerySuggested();
            if (result.Available)
            {
                UpdateStatus = result.Message;
                if (AutoInstallUpdates && result.UseCoreDownload)
                {
                    await InstallUpdateAsync();
                }
            }
            else if (manual)
            {
                UpdateStatus = result.Message;
            }
            else
            {
                UpdateStatus = "";
            }
        }
        catch (Exception error)
        {
            _updateAssetUrl = null;
            _updateUsesCoreDownload = false;
            OnPropertyChanged(nameof(UpdateInstallEnabled));
            CommandManager.InvalidateRequerySuggested();
            if (manual)
            {
                UpdateStatus = error.Message;
            }
        }
        finally
        {
            UpdateChecking = false;
        }
    }

    private async Task InstallUpdateAsync()
    {
        if (!_updateUsesCoreDownload || UpdateInstalling)
        {
            return;
        }
        UpdateInstalling = true;
        UpdateStatus = $"Downloading {UpdateVersion}";
        try
        {
            var path = await _updateService.DownloadWithCoreAsync(State.AppVersion, State.ConfigPath);
            UpdateStatus = $"Downloaded {Path.GetFileName(path)}";
            if (!UpdateService.SkipOpen)
            {
                _ = Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
            }
        }
        catch (Exception error)
        {
            UpdateStatus = error.Message;
        }
        finally
        {
            UpdateInstalling = false;
        }
    }

    public void HandleDeepLink(string url)
    {
#if DEBUG
        DebugRosterE2eTrace($"received {url}");
#endif
        if (url.StartsWith("nvpn://invite/", StringComparison.OrdinalIgnoreCase))
        {
            _ = ImportInviteAsync(url);
            return;
        }
        if (url.StartsWith("nvpn://join-request", StringComparison.OrdinalIgnoreCase))
        {
            _ = ConfirmAndImportJoinRequestAsync(url);
            return;
        }

#if DEBUG
        if (!Uri.TryCreate(url, UriKind.Absolute, out var uri)
            || !uri.Host.Equals("debug", StringComparison.OrdinalIgnoreCase))
        {
            DebugRosterE2eTrace("debug URL parse failed");
            return;
        }
        var query = ParseDebugQuery(uri.Query);
        var requestedNetwork = QueryValue(query, "networkId", "network");
        var network = State.Networks.FirstOrDefault(candidate =>
            string.IsNullOrWhiteSpace(requestedNetwork)
            || candidate.Id == requestedNetwork
            || candidate.NetworkId == requestedNetwork);
        if (network is null)
        {
            DebugRosterE2eTrace($"network not found: {requestedNetwork}");
            return;
        }
        if (uri.AbsolutePath.Equals("/request-join", StringComparison.OrdinalIgnoreCase))
        {
            _ = DispatchAsync(NativeActions.RequestNetworkJoin(network.Id), "Requesting access");
            return;
        }
        if (uri.AbsolutePath.Equals("/accept-join", StringComparison.OrdinalIgnoreCase))
        {
            var requester = QueryValue(query, "requesterNpub", "requester")
                ?? network.InboundJoinRequests.FirstOrDefault()?.RequesterNpub;
            if (!string.IsNullOrWhiteSpace(requester))
            {
                DebugRosterE2eTrace($"dispatching accept for {network.Id}");
                _ = DispatchAsync(NativeActions.AcceptJoinRequest(network.Id, requester), "Adding device");
            }
        }
#endif
    }

#if DEBUG
    private static void DebugRosterE2eTrace(string message)
    {
        var path = Environment.GetEnvironmentVariable("NVPN_ROSTER_E2E_TRACE_PATH");
        if (!string.IsNullOrWhiteSpace(path))
        {
            File.AppendAllText(path, $"{DateTimeOffset.UtcNow:O} {message}{Environment.NewLine}");
        }
    }

    private static Dictionary<string, string> ParseDebugQuery(string raw)
    {
        return raw.TrimStart('?')
            .Split('&', StringSplitOptions.RemoveEmptyEntries)
            .Select(pair => pair.Split('=', 2))
            .ToDictionary(
                pair => Uri.UnescapeDataString(pair[0].Replace('+', ' ')),
                pair => pair.Length > 1 ? Uri.UnescapeDataString(pair[1].Replace('+', ' ')) : "",
                StringComparer.OrdinalIgnoreCase);
    }

    private static string? QueryValue(IReadOnlyDictionary<string, string> query, params string[] names)
    {
        return names.Select(name => query.GetValueOrDefault(name))
            .FirstOrDefault(value => !string.IsNullOrWhiteSpace(value));
    }
#endif

    public void Dispose()
    {
        _refreshTimer.Stop();
        _updateTimer.Stop();
        _core.Dispose();
    }

    private async Task DispatchAsync(string actionJson, string status)
    {
        if (ActionInFlight)
        {
            return;
        }
        ActionInFlight = true;
        // Defer the in-progress notice so fast actions (broadcast/listen toggle,
        // copy, etc.) never flash the notice card. The card collapses when empty,
        // so showing it for ~50ms shifts the entire content below — that's what
        // looked like a flicker on the Share page when toggling broadcast/listen.
        using var noticeCts = new CancellationTokenSource();
        if (SynchronizationContext.Current is not null)
        {
            _ = Task.Delay(TimeSpan.FromMilliseconds(250), noticeCts.Token).ContinueWith(
                _ => Notice = status,
                CancellationToken.None,
                TaskContinuationOptions.OnlyOnRanToCompletion,
                TaskScheduler.FromCurrentSynchronizationContext());
        }
        try
        {
            var state = await Task.Run(() => _core.Dispatch(actionJson));
#if DEBUG
            DebugRosterE2eTrace($"dispatch completed: {state.Error}");
#endif
            noticeCts.Cancel();
            ApplyState(state, syncDrafts: true);
            Notice = string.IsNullOrWhiteSpace(state.Error) ? "" : state.Error;
        }
        catch (Exception error)
        {
#if DEBUG
            DebugRosterE2eTrace($"dispatch threw: {error}");
#endif
            noticeCts.Cancel();
            Notice = error.Message;
        }
        finally
        {
            ActionInFlight = false;
        }
    }

    private async Task ImportInviteAsync(string invite)
    {
        var trimmed = invite.Trim();
        if (string.IsNullOrEmpty(trimmed))
        {
            return;
        }
        await DispatchAsync(NativeActions.ImportNetworkInvite(trimmed), "Importing invite");
        // Always clear the paste field after a dispatch — keeps stale invites
        // from sticking around between sessions, and gives instant visual
        // feedback that the import was accepted.
        InviteInput = "";
    }

    private void PasteInviteFromClipboard()
    {
        try
        {
            if (Clipboard.ContainsText())
            {
                InviteInput = Clipboard.GetText().Trim();
            }
        }
        catch (Exception error)
        {
            Notice = error.Message;
        }
    }

    private static bool LooksLikeInviteCode(string value)
        => value.StartsWith("nvpn://invite/", StringComparison.OrdinalIgnoreCase);

    private static bool LooksLikeJoinRequest(string value)
    {
        const string prefix = "nvpn://join-request/";
        return value.StartsWith(prefix, StringComparison.OrdinalIgnoreCase) && value.Length > prefix.Length;
    }

    private const string Bech32BodyCharset = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

    /// <summary>
    /// A valid device ID is a bech32-encoded npub: "npub1" + 58 bech32 chars.
    /// </summary>
    public static bool IsValidDeviceId(string value)
    {
        if (string.IsNullOrWhiteSpace(value)) return false;
        var trimmed = value.Trim();
        if (trimmed.Length != 63 || !trimmed.StartsWith("npub1", StringComparison.Ordinal)) return false;
        for (var i = 5; i < trimmed.Length; i++)
        {
            if (Bech32BodyCharset.IndexOf(trimmed[i]) < 0) return false;
        }
        return true;
    }

    private async Task ConfirmAndImportJoinRequestAsync(string request)
    {
        var trimmed = request.Trim();
        if (_joinRequestPromptOpen || !LooksLikeJoinRequest(trimmed))
        {
            return;
        }
        var network = ActiveNetwork;
        if (network?.LocalIsAdmin != true)
        {
            return;
        }
        _joinRequestPromptOpen = true;
        try
        {
            var name = string.IsNullOrWhiteSpace(network.Name) ? "this network" : network.Name;
            var result = MessageBox.Show(
                $"Add the device from this join request to {name}?",
                "Add device?",
                MessageBoxButton.YesNo,
                MessageBoxImage.Question);
            if (result != MessageBoxResult.Yes)
            {
                return;
            }
            await DispatchAsync(NativeActions.ImportJoinRequest(trimmed), "Adding device");
            if (string.IsNullOrWhiteSpace(State.Error))
            {
                JoinRequestInput = "";
                Page = AppPage.Devices;
                Notice = "Device added";
            }
        }
        finally
        {
            _joinRequestPromptOpen = false;
        }
    }

    private async Task ImportJoinRequestQrImageAsync()
    {
        var dialog = new OpenFileDialog
        {
            Filter = "Images|*.png;*.jpg;*.jpeg;*.bmp;*.gif|All files|*.*",
            Multiselect = false,
        };
        if (dialog.ShowDialog() != true)
        {
            return;
        }
        var result = await Task.Run(() => _core.DecodeQrImage(dialog.FileName));
        if (!string.IsNullOrWhiteSpace(result.Error))
        {
            Notice = result.Error;
            return;
        }
        var value = result.Value.Trim();
        if (LooksLikeJoinRequest(value))
        {
            await ConfirmAndImportJoinRequestAsync(value);
            return;
        }
        var network = ActiveNetwork;
        if (IsValidDeviceId(value) && network?.LocalIsAdmin == true)
        {
            await DispatchAsync(NativeActions.AddParticipant(network.Id, value, null), "Adding device");
        }
        else
        {
            await DispatchAsync(NativeActions.ImportJoinRequest(value), "Adding device");
        }
    }

    private async Task ImportWireGuardExitAsync()
    {
        var dialog = new OpenFileDialog
        {
            Filter = "WireGuard configs|*.conf;*.txt|All files|*.*",
            Multiselect = false,
        };
        if (dialog.ShowDialog() != true)
        {
            return;
        }
        try
        {
            var config = await File.ReadAllTextAsync(dialog.FileName);
            if (string.IsNullOrWhiteSpace(config))
            {
                Notice = "Selected WireGuard config is empty.";
                return;
            }
            WireguardExitConfig = config;
            await SaveWireGuardExitAsync();
        }
        catch (Exception error)
        {
            Notice = $"Could not read WireGuard config: {error.Message}";
        }
    }

    private Task AddParticipantAsync()
    {
        var network = ActiveNetwork;
        if (network?.LocalIsAdmin != true)
        {
            return Task.CompletedTask;
        }
        return DispatchAsync(
            NativeActions.AddParticipant(network.Id, ParticipantInput.Trim(), string.IsNullOrWhiteSpace(ParticipantAliasInput) ? null : ParticipantAliasInput.Trim()),
            "Adding device");
    }

    private Task AddNetworkAsync()
    {
        return DispatchAsync(NativeActions.AddNetwork(NetworkNameInput.Trim()), "Adding network");
    }

    private async Task CreateNetworkAsync()
    {
        var name = string.IsNullOrWhiteSpace(NetworkNameInput) ? "My Network" : NetworkNameInput.Trim();
        NetworkNameInput = "";
        await DispatchAsync(NativeActions.AddNetwork(name), "Creating network");
        // Land on the new network's Devices view. Add Network may have
        // been the active page (or we may have been showing the
        // pre-network setup card); either way, Devices is the next
        // meaningful destination.
        SetNetworkSetupMode("");
        Page = AppPage.Devices;
    }

    private async Task ManualAddNetworkAsync()
    {
        var admin = (ManualJoinAdminId ?? string.Empty).Trim();
        var mesh = NormalizeNetworkIdInput(ManualJoinMeshId);
        if (admin.Length == 0 || mesh.Length == 0 || ManualJoinAdminInvalid)
        {
            return;
        }
        await DispatchAsync(NativeActions.ManualAddNetwork(admin, mesh), "Adding network");
        ManualJoinAdminId = "";
        ManualJoinMeshId = "";
        ManualJoinExpanded = false;
    }

    private Task ActivateInactiveNetworkAsync(string? networkId)
    {
        if (string.IsNullOrWhiteSpace(networkId))
        {
            return Task.CompletedTask;
        }
        return DispatchAsync(NativeActions.SetNetworkEnabled(networkId, true), "Switching network");
    }

    private Task SaveNodeAsync()
    {
        ushort? port = ushort.TryParse(ListenPort.Trim(), out var parsed) ? parsed : null;
        return DispatchAsync(NativeActions.UpdateSettings(new SettingsPatch
        {
            NodeName = NodeName,
            Endpoint = Endpoint,
            TunnelIp = TunnelIp,
            ListenPort = port,
            FipsHostInboundTcpPorts = FipsHostInboundTcpPorts,
        }), "Saving device");
    }

    private Task SaveRelaysAsync()
    {
        var relays = RelaysDraft
            .Split(new[] { '\r', '\n', ',', ' ', '\t' }, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .ToList();
        return DispatchAsync(NativeActions.UpdateSettings(new SettingsPatch
        {
            Relays = relays,
        }), "Saving relays");
    }

    private Task AddRelayAsync()
    {
        var url = NormalizeRelayUrl(RelayInput);
        if (url is null) return Task.CompletedTask;

        var (enabled, disabled) = RelayLists();
        disabled.RemoveAll(relay => relay == url);
        if (!enabled.Contains(url)) enabled.Add(url);
        RelayInput = "";
        return SaveRelayListsAsync(enabled, disabled);
    }

    public Task SetRelayEnabledAsync(NativeRelayState relay, bool enabledValue)
    {
        var url = NormalizeRelayUrl(relay.Url);
        if (url is null) return Task.CompletedTask;

        var (enabled, disabled) = RelayLists();
        enabled.RemoveAll(relayUrl => relayUrl == url);
        disabled.RemoveAll(relayUrl => relayUrl == url);
        if (enabledValue)
        {
            enabled.Add(url);
        }
        else
        {
            disabled.Add(url);
        }
        return SaveRelayListsAsync(enabled, disabled);
    }

    public Task RemoveRelayAsync(NativeRelayState relay)
    {
        var url = NormalizeRelayUrl(relay.Url);
        if (url is null) return Task.CompletedTask;

        var (enabled, disabled) = RelayLists();
        enabled.RemoveAll(relayUrl => relayUrl == url);
        disabled.RemoveAll(relayUrl => relayUrl == url);
        return SaveRelayListsAsync(enabled, disabled);
    }

    private (List<string> Enabled, List<string> Disabled) RelayLists()
    {
        var enabled = UniqueRelays(State.Relays.Where(relay => relay.Enabled).Select(relay => relay.Url));
        var disabled = UniqueRelays(State.Relays.Where(relay => !relay.Enabled).Select(relay => relay.Url));
        disabled.RemoveAll(relay => enabled.Contains(relay));
        return (enabled, disabled);
    }

    private Task SaveRelayListsAsync(List<string> enabledInput, List<string> disabledInput)
    {
        var enabled = UniqueRelays(enabledInput);
        var disabled = UniqueRelays(disabledInput);
        disabled.RemoveAll(relay => enabled.Contains(relay));
        return DispatchAsync(NativeActions.UpdateSettings(new SettingsPatch
        {
            Relays = enabled,
            DisabledRelays = disabled,
        }), "Saving relays");
    }

    private static string? NormalizeRelayUrl(string value)
    {
        var trimmed = value.Trim();
        if (string.IsNullOrWhiteSpace(trimmed)) return null;
        return trimmed.StartsWith("ws://", StringComparison.OrdinalIgnoreCase)
            || trimmed.StartsWith("wss://", StringComparison.OrdinalIgnoreCase)
            ? trimmed
            : $"wss://{trimmed}";
    }

    private static List<string> UniqueRelays(IEnumerable<string> values)
    {
        var seen = new HashSet<string>();
        return values
            .Select(NormalizeRelayUrl)
            .Where(relay => relay is not null && seen.Add(relay))
            .Select(relay => relay!)
            .ToList();
    }

    private Task SaveWireGuardExitAsync()
    {
        return DispatchAsync(NativeActions.UpdateSettings(new SettingsPatch
        {
            WireguardExitConfig = WireguardExitConfig,
        }), "Saving WireGuard");
    }

    private void ApplyState(NativeAppState state, bool syncDrafts)
    {
        TagSelfParticipants(state);
        NormalizeSelectedParticipant(state);
        State = state;
        JoinRequestQr = string.IsNullOrWhiteSpace(state.JoinRequestQrCodeOrLink)
            ? new QrMatrix()
            : _core.QrMatrix(state.JoinRequestQrCodeOrLink);
        if (syncDrafts)
        {
            SyncDrafts(state);
        }
        CommandManager.InvalidateRequerySuggested();
    }

    private static void TagSelfParticipants(NativeAppState state)
    {
        var ownNpub = state.OwnNpub;
        foreach (var network in state.Networks)
        {
            foreach (var participant in network.Participants)
            {
                participant.IsSelf =
                    string.Equals(participant.MeshState, "local", StringComparison.OrdinalIgnoreCase)
                    || (!string.IsNullOrEmpty(ownNpub) && participant.Npub == ownNpub);
            }
        }
    }

    private void SyncDrafts(NativeAppState state)
    {
        var active = state.Networks.FirstOrDefault(network => network.Id == _shownNetworkId)
            ?? state.Networks.FirstOrDefault(network => network.Enabled)
            ?? state.Networks.FirstOrDefault();
        NodeName = state.NodeName;
        Endpoint = state.Endpoint;
        TunnelIp = state.TunnelIp;
        ListenPort = state.ListenPort.ToString();
        RelaysDraft = string.Join(Environment.NewLine, state.Relays.Select(relay => relay.Url));
        FipsHostInboundTcpPorts = state.FipsHostInboundTcpPorts;
        WireguardExitConfig = state.WireguardExitConfig;
        if (string.IsNullOrWhiteSpace(PaidRouteMintUrl))
        {
            PaidRouteMintUrl = state.PaidRouteMarket.Wallet.DefaultMint;
        }
        NetworkNameDraft = active?.Name ?? "";
        NetworkMeshIdDraft = DisplayNetworkId(active?.NetworkId ?? "");
    }

    private static string NormalizeNetworkIdInput(string value)
    {
        var trimmed = (value ?? string.Empty).Trim();
        var compact = new string(trimmed.Where(ch => !char.IsWhiteSpace(ch) && ch != '-').ToArray());
        if (compact.Length == 0 && trimmed.All(ch => char.IsWhiteSpace(ch) || ch == '-'))
        {
            return "";
        }
        return compact.Length > 0 && compact.All(Uri.IsHexDigit) ? compact.ToLowerInvariant() : trimmed;
    }

    private static string DisplayNetworkId(string value)
    {
        var trimmed = (value ?? string.Empty).Trim();
        if (trimmed.Length <= 4 || trimmed.Any(ch => !Uri.IsHexDigit(ch)))
        {
            return trimmed;
        }
        return string.Join("-", Enumerable.Range(0, (trimmed.Length + 3) / 4)
            .Select(index => trimmed.Substring(index * 4, Math.Min(4, trimmed.Length - index * 4))));
    }

    private static string DisplayNetworkName(NativeNetworkState? network)
    {
        if (network is null)
        {
            return "Nostr VPN";
        }
        return string.IsNullOrWhiteSpace(network.Name) ? "Private network" : network.Name;
    }

    private bool PageIsVisible(AppPage page) =>
        page switch
        {
            AppPage.PublicExits or AppPage.Wallet => PaidRouteMarketVisible,
            AppPage.SellAccess => PaidExitSellerVisible,
            _ => true,
        };

    private void RaiseDerivedStateChanged()
    {
        if (!string.IsNullOrWhiteSpace(_shownNetworkId)
            && State.Networks.All(network => network.Id != _shownNetworkId))
        {
            _shownNetworkId = "";
        }
        OnPropertyChanged(nameof(ActiveNetwork));
        OnPropertyChanged(nameof(ExitNodeParticipants));
        OnPropertyChanged(nameof(HasActiveNetwork));
        OnPropertyChanged(nameof(HasEnabledNetwork));
        OnPropertyChanged(nameof(ShowJoinRequestQr));
        OnPropertyChanged(nameof(OfferExitNodeLabel));
        OnPropertyChanged(nameof(InactiveNetworks));
        OnPropertyChanged(nameof(SelectedParticipantKey));
        OnPropertyChanged(nameof(SelectedParticipant));
        RaiseSelectedParticipantChanged();
        OnPropertyChanged(nameof(ActiveNetworkName));
        OnPropertyChanged(nameof(ShownNetworkStatusBrush));
        OnPropertyChanged(nameof(ShowNetworkStatusDot));
        OnPropertyChanged(nameof(ShowLinkDeviceCard));
        OnPropertyChanged(nameof(JoinRequestQrCodeOrLink));
        OnPropertyChanged(nameof(ShowJoinRequestQr));
        OnPropertyChanged(nameof(ShowNetworkSetupChoices));
        OnPropertyChanged(nameof(ShowNetworkSetupCreate));
        OnPropertyChanged(nameof(ShowNetworkSetupJoin));
        OnPropertyChanged(nameof(ActiveNetworkDisplayNetworkId));
        OnPropertyChanged(nameof(HeroSubtitle));
        OnPropertyChanged(nameof(VpnButtonText));
        OnPropertyChanged(nameof(VpnStatusText));
        OnPropertyChanged(nameof(VpnStatusBrush));
        OnPropertyChanged(nameof(UpdateStripeText));
        OnPropertyChanged(nameof(ThisDeviceCopyValue));
        OnPropertyChanged(nameof(PublicFipsAddress));
        OnPropertyChanged(nameof(PublicFipsRoutingEnabled));
        OnPropertyChanged(nameof(NearbyDiscoveryButtonText));
        OnPropertyChanged(nameof(JoinRequestBroadcastButtonText));
        OnPropertyChanged(nameof(NoNearbyInvitesNoticeVisibility));
        OnPropertyChanged(nameof(ServiceSummary));
        OnPropertyChanged(nameof(CliSummary));
        OnPropertyChanged(nameof(SystemVersionLabel));
        OnPropertyChanged(nameof(DiagnosticsInterface));
        OnPropertyChanged(nameof(DiagnosticsIpv4));
        OnPropertyChanged(nameof(DiagnosticsIpv6));
        OnPropertyChanged(nameof(DiagnosticsGateway));
        OnPropertyChanged(nameof(DiagnosticsMapping));
        OnPropertyChanged(nameof(DiagnosticsExternal));
        OnPropertyChanged(nameof(DiagnosticsPeers));
        OnPropertyChanged(nameof(DiagnosticsFips));
        OnPropertyChanged(nameof(DiagnosticsOtherFips));
        OnPropertyChanged(nameof(DirectExitMarker));
        OnPropertyChanged(nameof(WireguardExitMarker));
        OnPropertyChanged(nameof(PaidAutomaticExitMarker));
        OnPropertyChanged(nameof(PaidManualExitMarker));
        OnPropertyChanged(nameof(WireguardExitSubtitle));
        OnPropertyChanged(nameof(PaidRouteMarketVisible));
        OnPropertyChanged(nameof(PaidExitSellerVisible));
        OnPropertyChanged(nameof(PaidRouteWalletBalanceText));
        OnPropertyChanged(nameof(WalletNavigationText));
        OnPropertyChanged(nameof(PaidRouteWalletFiatText));
        OnPropertyChanged(nameof(PaidRouteWalletRateText));
        OnPropertyChanged(nameof(PaidRouteMarketStatusText));
        OnPropertyChanged(nameof(PaidExitSellerStatusText));
        OnPropertyChanged(nameof(PaidExitSellerSummary));
        OnPropertyChanged(nameof(PaidExitSellerTrialText));
        OnPropertyChanged(nameof(PaidExitSellerChannelExpiryText));
        OnPropertyChanged(nameof(PaidExitSellerSettlementText));
        OnPropertyChanged(nameof(PaidExitSellerPublicIpText));
        OnPropertyChanged(nameof(PaidExitSellerChannelCreditText));
        OnPropertyChanged(nameof(PaidExitSellerChannelCreditHelpText));
        OnPropertyChanged(nameof(PaidExitSellerPaymentStatusText));
        OnPropertyChanged(nameof(PaidExitSellerMintsText));
        OnPropertyChanged(nameof(PaidExitSellerSessionsText));
        OnPropertyChanged(nameof(PaidRouteWalletMintRows));
        OnPropertyChanged(nameof(PaidRouteWalletMintsStatusText));
        OnPropertyChanged(nameof(PaidRouteHasStreamablePayments));
        RaisePaidRouteWalletInputStateChanged();
    }

    private void RaisePaidRouteWalletInputStateChanged()
    {
        OnPropertyChanged(nameof(CanAddPaidRouteWalletMint));
        OnPropertyChanged(nameof(CanTopUpPaidRouteWallet));
        OnPropertyChanged(nameof(CanSendPaidRouteWalletToken));
        OnPropertyChanged(nameof(CanReceivePaidRouteWalletToken));
        OnPropertyChanged(nameof(CanWithdrawPaidRouteWalletLightning));
    }

    private void RaiseSelectedParticipantChanged()
    {
        OnPropertyChanged(nameof(SelectedParticipantCanRename));
        OnPropertyChanged(nameof(SelectedParticipantCanManage));
    }

    private void NormalizeSelectedParticipant(NativeAppState state)
    {
        var network = state.Networks.FirstOrDefault(network => network.Id == _shownNetworkId)
            ?? state.Networks.FirstOrDefault(network => network.Enabled)
            ?? state.Networks.FirstOrDefault();
        if (network is null || network.Participants.Count == 0)
        {
            _selectedParticipantKey = "";
            return;
        }
        if (!string.IsNullOrWhiteSpace(_selectedParticipantKey)
            && network.Participants.FirstOrDefault(participant => ParticipantMatchesKey(participant, _selectedParticipantKey)) is { } selected)
        {
            _selectedParticipantKey = ParticipantKey(selected);
            return;
        }
        _selectedParticipantKey = SortedParticipants(network).FirstOrDefault() is { } first
            ? ParticipantKey(first)
            : "";
    }

    private NativeParticipantState? ResolveSelectedParticipant(NativeNetworkState network)
    {
        if (!string.IsNullOrWhiteSpace(_selectedParticipantKey))
        {
            var selected = network.Participants.FirstOrDefault(participant => ParticipantMatchesKey(participant, _selectedParticipantKey));
            if (selected is not null)
            {
                return selected;
            }
        }
        return SortedParticipants(network).FirstOrDefault();
    }

    private static IEnumerable<NativeParticipantState> SortedParticipants(NativeNetworkState network)
        => network.Participants
            .OrderBy(participant => !participant.IsSelf)
            .ThenBy(participant => !participant.Reachable)
            .ThenBy(participant => participant.DisplayName, StringComparer.OrdinalIgnoreCase);

    private static string ParticipantKey(NativeParticipantState participant)
        => participant.SelectionKey;

    private static bool ParticipantMatchesKey(NativeParticipantState participant, string key)
    {
        if (string.IsNullOrWhiteSpace(key))
        {
            return false;
        }
        return string.Equals(participant.SelectionKey, key, StringComparison.Ordinal)
            || (!string.IsNullOrWhiteSpace(participant.PubkeyHex)
                && string.Equals(participant.PubkeyHex, key, StringComparison.OrdinalIgnoreCase))
            || (!string.IsNullOrWhiteSpace(participant.Npub)
                && string.Equals(participant.Npub, key, StringComparison.Ordinal));
    }

    private void SetSelectedParticipantKey(string? value, bool ignoreTransientClear)
    {
        var nextKey = (value ?? string.Empty).Trim();
        if (nextKey.Length == 0 && ignoreTransientClear && CurrentSelectedParticipantStillExists())
        {
            return;
        }
        if (_selectedParticipantKey == nextKey)
        {
            return;
        }
        _selectedParticipantKey = nextKey;
        OnPropertyChanged(nameof(SelectedParticipantKey));
        OnPropertyChanged(nameof(SelectedParticipant));
        RaiseSelectedParticipantChanged();
    }

    private bool CurrentSelectedParticipantStillExists()
    {
        var network = ActiveNetwork;
        return network is not null
            && !string.IsNullOrWhiteSpace(_selectedParticipantKey)
            && network.Participants.Any(participant => ParticipantMatchesKey(participant, _selectedParticipantKey));
    }

    private static string FirstNonEmpty(string first, string second, string fallback)
    {
        if (!string.IsNullOrWhiteSpace(first))
        {
            return first;
        }
        return string.IsNullOrWhiteSpace(second) ? fallback : second;
    }

    private static string TextOr(string value, string fallback)
        => string.IsNullOrWhiteSpace(value) ? fallback : value;

    private static string? OptionalTrimmed(string value)
    {
        var trimmed = value.Trim();
        return string.IsNullOrWhiteSpace(trimmed) ? null : trimmed;
    }

    private static ulong? ParsePositiveUInt64(string value)
    {
        return ulong.TryParse(value.Trim(), out var parsed) && parsed > 0
            ? parsed
            : null;
    }

    private static bool LoadAutoInstallUpdates()
    {
        using var key = Registry.CurrentUser.OpenSubKey(@"Software\Nostr VPN");
        return key?.GetValue("AutoInstallUpdates") is int value && value != 0;
    }

    private static void SaveAutoInstallUpdates(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(@"Software\Nostr VPN");
        key?.SetValue("AutoInstallUpdates", enabled ? 1 : 0, RegistryValueKind.DWord);
    }

    private static TimeSpan LoadUpdatePollInterval()
    {
        var raw = Environment.GetEnvironmentVariable("NVPN_UPDATE_POLL_SECONDS");
        return double.TryParse(raw, out var seconds) && seconds > 0
            ? TimeSpan.FromSeconds(seconds)
            : TimeSpan.FromHours(6);
    }

    private bool SetField<T>(ref T field, T value, [CallerMemberName] string propertyName = "")
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }
        field = value;
        OnPropertyChanged(propertyName);
        return true;
    }

    private void OnPropertyChanged([CallerMemberName] string propertyName = "")
    {
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(propertyName));
    }
}
