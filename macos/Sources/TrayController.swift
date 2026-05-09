import AppKit
import Combine
import SwiftUI

/// AppKit-backed tray menu.
///
/// SwiftUI's `MenuBarExtra` tears down and rebuilds the `Menu` content tree
/// every time the observed `AppManager` republishes its state (the refresh
/// task fires every ~1.5s). That rebuild dismisses any submenu the user has
/// open, so submenus appear to "close themselves" within ~1s. NSMenuItems
/// are persistent AppKit objects: mutating their titles in place leaves an
/// open submenu undisturbed.
@MainActor
final class TrayController: NSObject {
    private let manager: AppManager
    private let openMainWindow: () -> Void

    private let statusItem: NSStatusItem
    private let menu = NSMenu()

    // Static items
    private let openItem = NSMenuItem()
    private let exitNodeStatusItem = NSMenuItem()
    private let vpnToggleItem = NSMenuItem()
    private let advertiseExitItem = NSMenuItem()
    private let copyDeviceItem = NSMenuItem()
    private let networkSubmenuItem = NSMenuItem()
    private let exitNodeSubmenuItem = NSMenuItem()
    private let refreshItem = NSMenuItem()
    private let quitItem = NSMenuItem()

    private let networkSubmenu = NSMenu()
    private let exitNodeSubmenu = NSMenu()

    private var cancellables = Set<AnyCancellable>()

    /// Last rendered menu inputs, so we can short-circuit when nothing visible
    /// changed (avoids needless menu mutation while submenus are open).
    private var lastSnapshot: MenuSnapshot?

    init(manager: AppManager, openMainWindow: @escaping () -> Void) {
        self.manager = manager
        self.openMainWindow = openMainWindow
        self.statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        super.init()

        configureStatusItem()
        buildMenuSkeleton()
        statusItem.menu = menu

        // Initial render and subscription. We observe the AppManager rather
        // than every @Published property; an AppManager publish triggers
        // refreshFromState(), which de-dupes against `lastSnapshot`.
        refreshFromState()
        manager.objectWillChange
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in
                // objectWillChange fires *before* the new value is assigned;
                // hop another main-actor tick so we read the post-assignment state.
                Task { @MainActor [weak self] in
                    self?.refreshFromState()
                }
            }
            .store(in: &cancellables)
    }

    // MARK: - Setup

    private func configureStatusItem() {
        guard let button = statusItem.button else { return }
        if let image = NSImage(named: "TrayIcon") {
            image.isTemplate = true
            button.image = image
        }
        button.toolTip = "Nostr VPN"
    }

    private func buildMenuSkeleton() {
        openItem.title = "Open Nostr VPN"
        openItem.target = self
        openItem.action = #selector(handleOpenMain)

        exitNodeStatusItem.isEnabled = false
        exitNodeStatusItem.isHidden = true

        vpnToggleItem.target = self
        vpnToggleItem.action = #selector(handleToggleVpn)

        advertiseExitItem.target = self
        advertiseExitItem.action = #selector(handleToggleAdvertiseExit)

        copyDeviceItem.title = "Copy This Device"
        copyDeviceItem.target = self
        copyDeviceItem.action = #selector(handleCopyDevice)

        networkSubmenuItem.title = "Network Devices"
        networkSubmenuItem.submenu = networkSubmenu
        networkSubmenuItem.isHidden = true

        exitNodeSubmenuItem.title = "Exit Node"
        exitNodeSubmenuItem.submenu = exitNodeSubmenu
        exitNodeSubmenuItem.isHidden = true

        refreshItem.title = "Refresh"
        refreshItem.target = self
        refreshItem.action = #selector(handleRefresh)

        quitItem.title = "Quit"
        quitItem.target = self
        quitItem.action = #selector(handleQuit)
        quitItem.keyEquivalent = "q"

        menu.addItem(openItem)
        menu.addItem(.separator())
        menu.addItem(exitNodeStatusItem)
        menu.addItem(vpnToggleItem)
        menu.addItem(advertiseExitItem)
        menu.addItem(.separator())
        menu.addItem(copyDeviceItem)
        menu.addItem(networkSubmenuItem)
        menu.addItem(exitNodeSubmenuItem)
        menu.addItem(.separator())
        menu.addItem(refreshItem)
        menu.addItem(quitItem)
    }

    // MARK: - Update from state

    private func refreshFromState() {
        let snapshot = MenuSnapshot.capture(from: manager)
        if snapshot == lastSnapshot {
            return
        }
        lastSnapshot = snapshot

        exitNodeStatusItem.title = snapshot.exitNodeStatusText
        exitNodeStatusItem.isHidden = snapshot.exitNodeStatusText.isEmpty

        vpnToggleItem.title = snapshot.vpnEnabled ? "Turn VPN Off" : "Turn VPN On"
        vpnToggleItem.isEnabled = snapshot.vpnTogglable

        advertiseExitItem.title =
            snapshot.advertiseExitNode ? "Stop Offering Exit" : "Offer Private Exit"

        copyDeviceItem.isEnabled = !snapshot.copyDeviceValue.isEmpty

        networkSubmenuItem.title = snapshot.networkTitle ?? "Network Devices"
        networkSubmenuItem.isHidden = snapshot.networkTitle == nil
        rebuildSubmenu(networkSubmenu, items: snapshot.networkItems) { [weak self] item in
            self?.manager.copy(item.npub, as: .peerNpub, peerNpub: item.npub)
        }

        exitNodeSubmenuItem.isHidden = snapshot.networkTitle == nil
        rebuildSubmenu(exitNodeSubmenu, items: snapshot.exitNodeItems) { [weak self] item in
            self?.manager.setExitNode(item.npub)
        }

        statusItem.button?.toolTip = snapshot.tooltip
    }

    /// Replaces a submenu's items only when the *visible* content changed.
    /// Each callback is wired to a fresh closure target so the captured npub
    /// matches the row's current label.
    private func rebuildSubmenu<T: Equatable>(
        _ submenu: NSMenu,
        items: [SubmenuItem<T>],
        action: @escaping (SubmenuItem<T>) -> Void
    ) {
        // Compare currently-installed items vs target.
        let current: [SubmenuItem<T>] = submenu.items.compactMap { item in
            (item.representedObject as? SubmenuItem<T>)
        }
        if current == items {
            return
        }
        submenu.removeAllItems()
        for item in items {
            let menuItem = NSMenuItem(
                title: item.title, action: #selector(handleSubmenuClick(_:)), keyEquivalent: "")
            menuItem.target = self
            menuItem.representedObject = SubmenuClickPayload(item: item, action: action)
            submenu.addItem(menuItem)
        }
    }

    // MARK: - Action handlers

    @objc private func handleOpenMain() {
        openMainWindow()
    }

    @objc private func handleToggleVpn() {
        manager.toggleVpn()
    }

    @objc private func handleToggleAdvertiseExit() {
        manager.setAdvertiseExitNode(!manager.state.advertiseExitNode)
    }

    @objc private func handleCopyDevice() {
        let value = manager.state.ownNpub.isEmpty ? manager.state.tunnelIp : manager.state.ownNpub
        if !value.isEmpty {
            manager.copy(value, as: .pubkey)
        }
    }

    @objc private func handleSubmenuClick(_ sender: NSMenuItem) {
        guard let payload = sender.representedObject as? AnySubmenuClickPayload else { return }
        payload.invoke()
    }

    @objc private func handleRefresh() {
        manager.refresh()
    }

    @objc private func handleQuit() {
        NSApp.terminate(nil)
    }
}

// MARK: - Menu snapshot

/// Visible-state snapshot used to decide whether the menu actually needs to
/// be touched on a given refresh tick. Counters that don't show up in the
/// menu (srtt, byte counts, etc.) are intentionally absent so they don't
/// trigger a no-op rebuild.
private struct MenuSnapshot: Equatable {
    let exitNodeStatusText: String
    let vpnEnabled: Bool
    let vpnTogglable: Bool
    let advertiseExitNode: Bool
    let copyDeviceValue: String
    let networkTitle: String?
    let networkItems: [SubmenuItem<NetworkRow>]
    let exitNodeItems: [SubmenuItem<ExitNodeRow>]
    let tooltip: String

    @MainActor
    static func capture(from manager: AppManager) -> MenuSnapshot {
        let state = manager.state
        let activeNetwork = manager.activeNetwork

        var networkTitle: String? = nil
        var networkItems: [SubmenuItem<NetworkRow>] = []
        var exitNodeItems: [SubmenuItem<ExitNodeRow>] = []

        if let activeNetwork {
            networkTitle = activeNetwork.name.isEmpty ? "Network Devices" : activeNetwork.name
            networkItems = activeNetwork.participants.map { p in
                SubmenuItem<NetworkRow>(
                    title: participantMenuTitle(p),
                    npub: p.npub,
                    payload: NetworkRow(pubkeyHex: p.pubkeyHex)
                )
            }
            exitNodeItems = [
                SubmenuItem<ExitNodeRow>(
                    title: "No exit node", npub: "", payload: ExitNodeRow(pubkeyHex: ""))
            ]
            exitNodeItems.append(
                contentsOf: activeNetwork.participants.filter { $0.offersExitNode }
                    .map { p in
                        SubmenuItem<ExitNodeRow>(
                            title: p.magicDnsName.isEmpty ? p.alias : p.magicDnsName,
                            npub: p.npub,
                            payload: ExitNodeRow(pubkeyHex: p.pubkeyHex)
                        )
                    })
        }

        let copyValue = state.ownNpub.isEmpty ? state.tunnelIp : state.ownNpub

        let tooltip: String = {
            if !state.exitNodeStatusText.isEmpty { return state.exitNodeStatusText }
            if !state.vpnStatus.isEmpty { return state.vpnStatus }
            return "Nostr VPN"
        }()

        return MenuSnapshot(
            exitNodeStatusText: state.exitNodeStatusText,
            vpnEnabled: state.vpnEnabled,
            vpnTogglable: !manager.actionInFlight && state.vpnControlSupported,
            advertiseExitNode: state.advertiseExitNode,
            copyDeviceValue: copyValue,
            networkTitle: networkTitle,
            networkItems: networkItems,
            exitNodeItems: exitNodeItems,
            tooltip: tooltip
        )
    }
}

private func participantMenuTitle(_ participant: NativeParticipantState) -> String {
    let name = participant.magicDnsName.isEmpty ? participant.alias : participant.magicDnsName
    if participant.tunnelIp.isEmpty || participant.tunnelIp == "-" {
        return name
    }
    return "\(name) (\(participant.tunnelIp))"
}

private struct NetworkRow: Equatable { let pubkeyHex: String }
private struct ExitNodeRow: Equatable { let pubkeyHex: String }

private struct SubmenuItem<Payload: Equatable>: Equatable {
    let title: String
    let npub: String
    let payload: Payload
}

/// Type-erased click handler so we can stash any SubmenuItem<T> in an
/// NSMenuItem.representedObject and dispatch it from one Obj-C selector.
private protocol AnySubmenuClickPayload {
    func invoke()
}

private struct SubmenuClickPayload<T: Equatable>: AnySubmenuClickPayload {
    let item: SubmenuItem<T>
    let action: (SubmenuItem<T>) -> Void
    func invoke() { action(item) }
}
