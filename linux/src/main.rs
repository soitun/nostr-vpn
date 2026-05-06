mod qr;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use nostr_vpn_app_core::{
    FfiApp, NativeAppAction, NativeAppState, NativeNetworkState, NativeParticipantState,
    NativeRelayState, SettingsPatch,
};

const APP_ID: &str = "to.iris.nvpn";

type AppRef = Rc<RefCell<AppModel>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Devices,
    Share,
    Routing,
    Settings,
}

#[derive(Clone, Default)]
struct Drafts {
    invite: String,
    participant_npub: String,
    participant_alias: String,
    relay: String,
    network_name: String,
    new_network_name: String,
    node_name: String,
    endpoint: String,
    tunnel_ip: String,
    listen_port: String,
    magic_dns_suffix: String,
    advertised_routes: String,
    exit_search: String,
}

impl Drafts {
    fn sync_from_state(&mut self, state: &NativeAppState) {
        self.node_name = state.node_name.clone();
        self.endpoint = state.endpoint.clone();
        self.tunnel_ip = state.tunnel_ip.clone();
        self.listen_port = state.listen_port.to_string();
        self.magic_dns_suffix = state.magic_dns_suffix.clone();
        self.advertised_routes = state.advertised_routes.join(", ");
        self.network_name = active_network(state)
            .map(display_network_name)
            .unwrap_or_else(|| "Nostr VPN".to_string());
    }
}

struct AppModel {
    core: Arc<FfiApp>,
    state: NativeAppState,
    page: Page,
    sidebar: gtk::Box,
    content: gtk::Box,
    drafts: Drafts,
}

impl AppModel {
    fn new(sidebar: gtk::Box, content: gtk::Box) -> Self {
        let core = FfiApp::new(default_data_dir(), env!("CARGO_PKG_VERSION").to_string());
        let state = core.state();
        let mut drafts = Drafts::default();
        drafts.sync_from_state(&state);
        Self {
            core,
            state,
            page: Page::Devices,
            sidebar,
            content,
            drafts,
        }
    }
}

fn main() -> glib::ExitCode {
    bootstrap_session_bus();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        install_css();
        gtk::Window::set_default_icon_name("nostr-vpn");
    });
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    if let Some(window) = app
        .active_window()
        .or_else(|| app.windows().into_iter().next())
    {
        window.present();
        return;
    }

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1040)
        .default_height(720)
        .title("Nostr VPN")
        .build();
    window.add_css_class("nvpn-root");

    let header = adw::HeaderBar::new();
    let title = gtk::Label::new(Some("Nostr VPN"));
    title.add_css_class("heading");
    title.set_halign(gtk::Align::Start);
    header.set_title_widget(Some(&title));

    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Refresh"));
    header.pack_end(&refresh_button);

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 10);
    sidebar.add_css_class("nvpn-sidebar");
    sidebar.set_width_request(210);
    sidebar.set_margin_top(14);
    sidebar.set_margin_bottom(14);
    sidebar.set_margin_start(14);
    sidebar.set_margin_end(10);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.add_css_class("nvpn-content");

    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shell.set_hexpand(true);
    shell.set_vexpand(true);
    shell.append(&sidebar);
    shell.append(&content);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&shell));
    window.set_content(Some(&toolbar));

    let model = Rc::new(RefCell::new(AppModel::new(
        sidebar.clone(),
        content.clone(),
    )));

    {
        let model = model.clone();
        refresh_button.connect_clicked(move |_| refresh_now(&model));
    }

    render(&model);

    {
        let model = model.clone();
        glib::timeout_add_seconds_local(2, move || {
            refresh_now(&model);
            glib::ControlFlow::Continue
        });
    }

    window.present();
}

fn refresh_now(app: &AppRef) {
    let core = app.borrow().core.clone();
    let state = core.refresh();
    app.borrow_mut().state = state;
    render(app);
}

fn dispatch(app: &AppRef, action: NativeAppAction) {
    let core = app.borrow().core.clone();
    let state = core.dispatch(action);
    app.borrow_mut().state = state;
    render(app);
}

fn set_page(app: &AppRef, page: Page) {
    app.borrow_mut().page = page;
    render(app);
}

fn render(app: &AppRef) {
    let (sidebar, content, state, page) = {
        let model = app.borrow();
        (
            model.sidebar.clone(),
            model.content.clone(),
            model.state.clone(),
            model.page,
        )
    };

    clear_box(&sidebar);
    clear_box(&content);
    build_sidebar(app, &sidebar, &state, page);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_vexpand(true);
    scroll.set_hexpand(true);

    let page_box = gtk::Box::new(gtk::Orientation::Vertical, 20);
    page_box.set_margin_top(28);
    page_box.set_margin_bottom(32);
    page_box.set_margin_start(28);
    page_box.set_margin_end(28);
    page_box.set_hexpand(true);
    page_box.set_valign(gtk::Align::Start);
    page_box.set_size_request(560, -1);

    match page {
        Page::Devices => build_devices_page(app, &page_box, &state),
        Page::Share => build_share_page(app, &page_box, &state),
        Page::Routing => build_routing_page(app, &page_box, &state),
        Page::Settings => build_settings_page(app, &page_box, &state),
    }

    scroll.set_child(Some(&page_box));
    content.append(&scroll);
}

fn build_sidebar(app: &AppRef, sidebar: &gtk::Box, state: &NativeAppState, page: Page) {
    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    brand.set_margin_bottom(6);
    let label = gtk::Label::new(Some("Nostr VPN"));
    label.add_css_class("heading");
    label.set_xalign(0.0);
    brand.append(&label);
    sidebar.append(&brand);

    for (target, title, icon) in [
        (Page::Devices, "Devices", ""),
        (Page::Share, "Share", "emblem-shared-symbolic"),
        (Page::Routing, "Routing", ""),
        (Page::Settings, "Settings", "emblem-system-symbolic"),
    ] {
        let button = nav_button(title, icon, page == target);
        let app = app.clone();
        button.connect_clicked(move |_| set_page(&app, target));
        sidebar.append(&button);
    }

    sidebar.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    if let Some(network) = active_network(state) {
        let summary = gtk::Box::new(gtk::Orientation::Vertical, 4);
        summary.add_css_class("nvpn-sidebar-summary");

        let name = gtk::Label::new(Some(&display_network_name(network)));
        name.add_css_class("caption-heading");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary.append(&name);

        let count = gtk::Label::new(Some(&format!(
            "{} of {} connected",
            state.connected_peer_count, state.expected_peer_count
        )));
        count.add_css_class("caption");
        count.add_css_class("dim-label");
        count.set_xalign(0.0);
        summary.append(&count);

        sidebar.append(&summary);
    }

    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    sidebar.append(&spacer);

    let status = gtk::Label::new(Some(&state.session_status));
    status.add_css_class("caption");
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    status.set_wrap(true);
    sidebar.append(&status);
}

fn build_devices_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    build_network_hero(app, page, state);

    if !state.error.trim().is_empty() {
        let card = card();
        row_label(&card, "Status", &state.error, "dialog-warning-symbolic");
        page.append(&card);
    }

    let Some(network) = active_network(state).cloned() else {
        let card = card();
        row_label(&card, "No network", "Create a network in Settings.", "");
        page.append(&card);
        return;
    };

    let devices = card();
    section_header(&devices, "Devices", "");

    let mut participants = network.participants.clone();
    participants.sort_by_key(|participant| {
        (
            !is_self(participant, state),
            !participant.reachable,
            device_name(participant),
        )
    });

    if participants.is_empty() {
        empty_row(&devices, "No devices yet");
    } else {
        for participant in participants {
            device_row(app, &devices, &network, &participant, state);
        }
    }

    if network.local_is_admin {
        let expander = gtk::Expander::new(Some("Manage devices"));
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.set_margin_top(10);

        let input_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let npub = entry("npub", &app.borrow().drafts.participant_npub);
        {
            let app = app.clone();
            npub.connect_changed(move |entry| {
                app.borrow_mut().drafts.participant_npub = entry.text().to_string();
            });
        }
        let alias = entry("Name", &app.borrow().drafts.participant_alias);
        alias.set_width_chars(16);
        {
            let app = app.clone();
            alias.connect_changed(move |entry| {
                app.borrow_mut().drafts.participant_alias = entry.text().to_string();
            });
        }
        let add = icon_text_button("Add", "list-add-symbolic");
        {
            let app = app.clone();
            let network_id = network.id.clone();
            add.connect_clicked(move |_| {
                let (npub, alias) = {
                    let model = app.borrow();
                    (
                        model.drafts.participant_npub.trim().to_string(),
                        model.drafts.participant_alias.trim().to_string(),
                    )
                };
                if npub.is_empty() {
                    return;
                }
                {
                    let mut model = app.borrow_mut();
                    model.drafts.participant_npub.clear();
                    model.drafts.participant_alias.clear();
                }
                dispatch(
                    &app,
                    NativeAppAction::AddParticipant {
                        network_id: network_id.clone(),
                        npub,
                        alias: (!alias.is_empty()).then_some(alias),
                    },
                );
            });
        }
        input_row.append(&npub);
        input_row.append(&alias);
        input_row.append(&add);
        body.append(&input_row);

        expander.set_child(Some(&body));
        devices.append(&expander);
    }

    page.append(&devices);

    if !network.inbound_join_requests.is_empty() {
        let requests = card();
        section_header(&requests, "Join Requests", "contact-new-symbolic");
        for request in &network.inbound_join_requests {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.set_valign(gtk::Align::Center);

            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let name = if request.requester_node_name.trim().is_empty() {
                "New device".to_string()
            } else {
                request.requester_node_name.clone()
            };
            let title = gtk::Label::new(Some(&name));
            title.set_xalign(0.0);
            title.add_css_class("heading");
            text.append(&title);
            let sub = gtk::Label::new(Some(&format!(
                "{}  {}",
                short_text(&request.requester_npub, 18),
                request.requested_at_text
            )));
            sub.add_css_class("caption");
            sub.add_css_class("dim-label");
            sub.set_xalign(0.0);
            text.append(&sub);
            text.set_hexpand(true);
            row.append(&text);

            let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy.set_tooltip_text(Some("Copy npub"));
            {
                let npub = request.requester_npub.clone();
                copy.connect_clicked(move |_| copy_text(&npub));
            }
            row.append(&copy);

            let accept = icon_text_button("Accept", "emblem-ok-symbolic");
            accept.add_css_class("suggested-action");
            {
                let app = app.clone();
                let network_id = network.id.clone();
                let requester_npub = request.requester_npub.clone();
                accept.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        NativeAppAction::AcceptJoinRequest {
                            network_id: network_id.clone(),
                            requester_npub: requester_npub.clone(),
                        },
                    );
                });
            }
            row.append(&accept);
            requests.append(&row);
        }
        page.append(&requests);
    }
}

fn build_network_hero(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let hero = card();
    hero.add_css_class("nvpn-hero");

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    top.set_valign(gtk::Align::Center);

    let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
    status.add_css_class(if state.mesh_ready {
        "nvpn-status-ready"
    } else if state.session_active {
        "nvpn-status-active"
    } else {
        "nvpn-status-off"
    });
    top.append(&status);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);
    let network_name = active_network(state)
        .map(display_network_name)
        .unwrap_or_else(|| "Nostr VPN".to_string());
    let title = gtk::Label::new(Some(&network_name));
    title.add_css_class("title-1");
    title.set_xalign(0.0);
    title.set_wrap(true);
    text.append(&title);

    let subtitle = gtk::Label::new(Some(&hero_subtitle(state)));
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    text.append(&subtitle);
    top.append(&text);

    let connect = icon_text_button(
        if state.session_active {
            "Connected"
        } else {
            "Connect"
        },
        if state.session_active {
            "media-playback-stop-symbolic"
        } else {
            "media-playback-start-symbolic"
        },
    );
    connect.add_css_class("suggested-action");
    connect.set_sensitive(state.vpn_session_control_supported);
    {
        let app = app.clone();
        let active = state.session_active;
        connect.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::DisconnectSession
                } else {
                    NativeAppAction::ConnectSession
                },
            );
        });
    }
    top.append(&connect);
    hero.append(&top);

    page.append(&hero);
}

fn build_share_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Share", "emblem-shared-symbolic");

    let Some(network) = active_network(state).cloned() else {
        return;
    };

    let invite = card();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    row.set_valign(gtk::Align::Start);
    row.append(&qr::build(&state.active_network_invite, 150));

    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_hexpand(true);
    section_header(&column, "Invite Devices", "emblem-shared-symbolic");

    let invite_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let code = gtk::Entry::new();
    code.set_text(&state.active_network_invite);
    code.set_editable(false);
    code.set_hexpand(true);
    code.set_placeholder_text(Some("No invite"));
    invite_row.append(&code);
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some("Copy invite"));
    copy.set_sensitive(!state.active_network_invite.is_empty());
    {
        let invite = state.active_network_invite.clone();
        copy.connect_clicked(move |_| copy_text(&invite));
    }
    invite_row.append(&copy);
    column.append(&invite_row);

    let import_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let invite_entry = entry("Paste invite", &app.borrow().drafts.invite);
    {
        let app = app.clone();
        invite_entry.connect_changed(move |entry| {
            app.borrow_mut().drafts.invite = entry.text().to_string();
        });
    }
    let import = icon_text_button("Import", "go-down-symbolic");
    {
        let app = app.clone();
        import.connect_clicked(move |_| {
            let invite = app.borrow().drafts.invite.trim().to_string();
            if invite.is_empty() {
                return;
            }
            app.borrow_mut().drafts.invite.clear();
            dispatch(&app, NativeAppAction::ImportNetworkInvite { invite });
        });
    }
    import_row.append(&invite_entry);
    import_row.append(&import);
    column.append(&import_row);

    if network.outbound_join_request.is_some() {
        column.append(&badge("Join requested", "warn"));
    } else if !network.invite_inviter_npub.is_empty() {
        let request = icon_text_button("Request Access", "contact-new-symbolic");
        {
            let app = app.clone();
            let network_id = network.id.clone();
            request.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::RequestNetworkJoin {
                        network_id: network_id.clone(),
                    },
                );
            });
        }
        column.append(&request);
    }

    row.append(&column);
    invite.append(&row);
    page.append(&invite);

    let nearby = card();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    section_header(&header, "Nearby Devices", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    let lan_label = if state.lan_pairing_active {
        format!("{}s", state.lan_pairing_remaining_secs)
    } else {
        "Pair Nearby".to_string()
    };
    let lan = icon_text_button(
        &lan_label,
        if state.lan_pairing_active {
            "media-playback-stop-symbolic"
        } else {
            "list-add-symbolic"
        },
    );
    {
        let app = app.clone();
        let active = state.lan_pairing_active;
        lan.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::StopLanPairing
                } else {
                    NativeAppAction::StartLanPairing
                },
            );
        });
    }
    header.append(&lan);
    nearby.append(&header);

    if state.lan_peers.is_empty() {
        empty_row(&nearby, "No nearby invites");
    } else {
        for peer in &state.lan_peers {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let name = if peer.node_name.trim().is_empty() {
                short_text(&peer.npub, 20)
            } else {
                peer.node_name.clone()
            };
            let title = gtk::Label::new(Some(&name));
            title.set_xalign(0.0);
            title.add_css_class("heading");
            text.append(&title);
            let sub = gtk::Label::new(Some(&format!(
                "{}  {}",
                peer.network_name, peer.last_seen_text
            )));
            sub.add_css_class("caption");
            sub.add_css_class("dim-label");
            sub.set_xalign(0.0);
            text.append(&sub);
            text.set_hexpand(true);
            row.append(&text);

            let join = icon_text_button("Join", "go-next-symbolic");
            {
                let app = app.clone();
                let invite = peer.invite.clone();
                join.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        NativeAppAction::ImportNetworkInvite {
                            invite: invite.clone(),
                        },
                    );
                });
            }
            row.append(&join);
            nearby.append(&row);
        }
    }
    page.append(&nearby);
}

fn build_routing_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Routing", "");

    let Some(network) = active_network(state).cloned() else {
        return;
    };

    let exit = card();
    section_header(&exit, "Exit Node", "");

    let search = entry("Search devices", &app.borrow().drafts.exit_search);
    {
        let app = app.clone();
        search.connect_changed(move |entry| {
            app.borrow_mut().drafts.exit_search = entry.text().to_string();
        });
    }
    exit.append(&search);

    route_choice(
        app,
        &exit,
        "Direct",
        "Use normal internet routing",
        state.exit_node.is_empty(),
        true,
        None,
    );

    let query = app.borrow().drafts.exit_search.to_ascii_lowercase();
    for participant in exit_node_candidates(&network)
        .into_iter()
        .filter(|participant| {
            query.is_empty()
                || device_name(participant)
                    .to_ascii_lowercase()
                    .contains(&query)
                || participant.npub.to_ascii_lowercase().contains(&query)
        })
    {
        route_choice(
            app,
            &exit,
            &device_name(&participant),
            if participant.offers_exit_node {
                non_empty_or(&participant.status_text, "Exit node")
            } else {
                "Exit not offered".to_string()
            }
            .as_str(),
            state.exit_node == participant.npub,
            participant.offers_exit_node,
            Some(participant.npub.clone()),
        );
    }
    page.append(&exit);

    let subnet = card();
    section_header(&subnet, "Subnet Routes", "");
    switch_row(
        app,
        &subnet,
        "Offer this device as an exit node",
        state.advertise_exit_node,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                advertise_exit_node: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );

    let routes = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = entry("Advertised routes", &app.borrow().drafts.advertised_routes);
    {
        let app = app.clone();
        entry.connect_changed(move |entry| {
            app.borrow_mut().drafts.advertised_routes = entry.text().to_string();
        });
    }
    let save = gtk::Button::from_icon_name("emblem-ok-symbolic");
    save.set_tooltip_text(Some("Save routes"));
    {
        let app = app.clone();
        save.connect_clicked(move |_| {
            let advertised_routes = app.borrow().drafts.advertised_routes.clone();
            dispatch(
                &app,
                NativeAppAction::UpdateSettings {
                    patch: SettingsPatch {
                        advertised_routes: Some(advertised_routes),
                        ..SettingsPatch::default()
                    },
                },
            );
        });
    }
    routes.append(&entry);
    routes.append(&save);
    subnet.append(&routes);
    page.append(&subnet);
}

fn route_choice(
    app: &AppRef,
    parent: &gtk::Box,
    title: &str,
    subtitle: &str,
    selected: bool,
    enabled: bool,
    exit_node: Option<String>,
) {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("nvpn-route-choice");
    button.set_sensitive(enabled);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let icon = gtk::Image::from_icon_name(if selected {
        "emblem-ok-symbolic"
    } else {
        "radio-symbolic"
    });
    if selected {
        icon.add_css_class("success");
    }
    row.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title_label = gtk::Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.add_css_class("heading");
    text.append(&title_label);
    let subtitle_label = gtk::Label::new(Some(subtitle));
    subtitle_label.set_xalign(0.0);
    subtitle_label.add_css_class("caption");
    subtitle_label.add_css_class("dim-label");
    text.append(&subtitle_label);
    row.append(&text);

    button.set_child(Some(&row));
    {
        let app = app.clone();
        button.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::UpdateSettings {
                    patch: SettingsPatch {
                        exit_node: Some(exit_node.clone().unwrap_or_default()),
                        ..SettingsPatch::default()
                    },
                },
            );
        });
    }
    parent.append(&button);
}

fn build_settings_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Settings", "emblem-system-symbolic");

    let device = card();
    section_header(&device, "This Device", "");
    setting_entry(app, &device, "Name", "node_name");
    setting_entry(app, &device, "Tunnel IP", "tunnel_ip");
    setting_entry(app, &device, "Endpoint", "endpoint");
    setting_entry(app, &device, "Listen Port", "listen_port");
    setting_entry(app, &device, "DNS Suffix", "magic_dns_suffix");

    let save = icon_text_button("Save", "emblem-ok-symbolic");
    save.add_css_class("suggested-action");
    {
        let app = app.clone();
        save.connect_clicked(move |_| save_device_settings(&app));
    }
    device.append(&save);
    page.append(&device);

    let network = card();
    section_header(&network, "Network", "");
    if let Some(active) = active_network(state).cloned() {
        let rename = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let input = entry("Network name", &app.borrow().drafts.network_name);
        {
            let app = app.clone();
            input.connect_changed(move |entry| {
                app.borrow_mut().drafts.network_name = entry.text().to_string();
            });
        }
        let save = gtk::Button::from_icon_name("emblem-ok-symbolic");
        save.set_tooltip_text(Some("Rename network"));
        {
            let app = app.clone();
            let network_id = active.id.clone();
            save.connect_clicked(move |_| {
                let name = app.borrow().drafts.network_name.trim().to_string();
                if !name.is_empty() {
                    dispatch(
                        &app,
                        NativeAppAction::RenameNetwork {
                            network_id: network_id.clone(),
                            name,
                        },
                    );
                }
            });
        }
        rename.append(&input);
        rename.append(&save);
        network.append(&rename);

        switch_row(app, &network, "Enabled", active.enabled, {
            let network_id = active.id.clone();
            move |enabled| NativeAppAction::SetNetworkEnabled {
                network_id: network_id.clone(),
                enabled,
            }
        });

        switch_row(
            app,
            &network,
            "Join requests",
            active.join_requests_enabled,
            {
                let network_id = active.id.clone();
                move |enabled| NativeAppAction::SetNetworkJoinRequestsEnabled {
                    network_id: network_id.clone(),
                    enabled,
                }
            },
        );
    }

    let add_network = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let new_name = entry("New network", &app.borrow().drafts.new_network_name);
    {
        let app = app.clone();
        new_name.connect_changed(move |entry| {
            app.borrow_mut().drafts.new_network_name = entry.text().to_string();
        });
    }
    let add = icon_text_button("Add", "list-add-symbolic");
    {
        let app = app.clone();
        add.connect_clicked(move |_| {
            let name = app.borrow().drafts.new_network_name.trim().to_string();
            if name.is_empty() {
                return;
            }
            app.borrow_mut().drafts.new_network_name.clear();
            dispatch(&app, NativeAppAction::AddNetwork { name });
        });
    }
    add_network.append(&new_name);
    add_network.append(&add);
    network.append(&add_network);
    page.append(&network);

    let system = card();
    section_header(&system, "System", "");
    switch_row(app, &system, "Autoconnect", state.autoconnect, |enabled| {
        NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                autoconnect: Some(enabled),
                ..SettingsPatch::default()
            },
        }
    });
    if state.startup_settings_supported {
        switch_row(
            app,
            &system,
            "Launch on startup",
            state.launch_on_startup,
            |enabled| NativeAppAction::UpdateSettings {
                patch: SettingsPatch {
                    launch_on_startup: Some(enabled),
                    ..SettingsPatch::default()
                },
            },
        );
    }
    if state.tray_behavior_supported {
        switch_row(
            app,
            &system,
            "Tray on close",
            state.close_to_tray_on_close,
            |enabled| NativeAppAction::UpdateSettings {
                patch: SettingsPatch {
                    close_to_tray_on_close: Some(enabled),
                    ..SettingsPatch::default()
                },
            },
        );
    }

    let service_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    service_row.set_halign(gtk::Align::Start);
    let cli = icon_text_button(
        if state.cli_installed {
            "CLI Installed"
        } else {
            "Install CLI"
        },
        "utilities-terminal-symbolic",
    );
    cli.set_sensitive(state.cli_install_supported && !state.cli_installed);
    {
        let app = app.clone();
        cli.connect_clicked(move |_| dispatch(&app, NativeAppAction::InstallCli));
    }
    service_row.append(&cli);

    if state.service_supported {
        let service = icon_text_button(
            if state.service_installed {
                "Service Installed"
            } else {
                "Install Service"
            },
            "system-run-symbolic",
        );
        service.set_sensitive(!state.service_installed);
        {
            let app = app.clone();
            service.connect_clicked(move |_| dispatch(&app, NativeAppAction::InstallSystemService));
        }
        service_row.append(&service);
    }
    system.append(&service_row);
    page.append(&system);

    let advanced = gtk::Expander::new(Some("Advanced"));
    let advanced_body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    advanced_body.set_margin_top(10);
    build_relays(app, &advanced_body, state);
    build_diagnostics(&advanced_body, state);
    advanced.set_child(Some(&advanced_body));
    page.append(&advanced);
}

fn build_relays(app: &AppRef, parent: &gtk::Box, state: &NativeAppState) {
    let relays = card();
    section_header(&relays, "FIPS Relays", "");

    let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let input = entry("wss://relay.example", &app.borrow().drafts.relay);
    {
        let app = app.clone();
        input.connect_changed(move |entry| {
            app.borrow_mut().drafts.relay = entry.text().to_string();
        });
    }
    let add = icon_text_button("Add", "list-add-symbolic");
    {
        let app = app.clone();
        add.connect_clicked(move |_| {
            let relay = app.borrow().drafts.relay.trim().to_string();
            if relay.is_empty() {
                return;
            }
            app.borrow_mut().drafts.relay.clear();
            dispatch(&app, NativeAppAction::AddRelay { relay });
        });
    }
    add_row.append(&input);
    add_row.append(&add);
    relays.append(&add_row);

    if state.relays.is_empty() {
        empty_row(&relays, "No relays configured");
    } else {
        for relay in &state.relays {
            relay_row(app, &relays, relay);
        }
    }
    parent.append(&relays);
}

fn build_diagnostics(parent: &gtk::Box, state: &NativeAppState) {
    let diagnostics = card();
    section_header(&diagnostics, "Diagnostics", "dialog-information-symbolic");
    detail_row(&diagnostics, "This device", &state.own_npub);
    detail_row(&diagnostics, "Tunnel IP", &clean_ip(&state.tunnel_ip));
    detail_row(&diagnostics, "Endpoint", &state.endpoint);
    detail_row(&diagnostics, "Config", &state.config_path);
    detail_row(&diagnostics, "MagicDNS", &state.magic_dns_status);
    detail_row(&diagnostics, "Service", &state.service_status_detail);
    detail_row(&diagnostics, "Runtime", &state.runtime_status_detail);

    for issue in &state.health {
        row_label(
            &diagnostics,
            &issue.summary,
            &issue.detail,
            "dialog-warning-symbolic",
        );
    }
    parent.append(&diagnostics);
}

fn setting_entry(app: &AppRef, parent: &gtk::Box, title: &str, key: &'static str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_width_chars(13);
    row.append(&label);

    let current = {
        let model = app.borrow();
        match key {
            "node_name" => model.drafts.node_name.clone(),
            "endpoint" => model.drafts.endpoint.clone(),
            "tunnel_ip" => model.drafts.tunnel_ip.clone(),
            "listen_port" => model.drafts.listen_port.clone(),
            "magic_dns_suffix" => model.drafts.magic_dns_suffix.clone(),
            _ => String::new(),
        }
    };
    let input = entry(title, &current);
    {
        let app = app.clone();
        input.connect_changed(move |entry| {
            let value = entry.text().to_string();
            let mut model = app.borrow_mut();
            match key {
                "node_name" => model.drafts.node_name = value,
                "endpoint" => model.drafts.endpoint = value,
                "tunnel_ip" => model.drafts.tunnel_ip = value,
                "listen_port" => model.drafts.listen_port = value,
                "magic_dns_suffix" => model.drafts.magic_dns_suffix = value,
                _ => {}
            }
        });
    }
    row.append(&input);
    parent.append(&row);
}

fn save_device_settings(app: &AppRef) {
    let drafts = app.borrow().drafts.clone();
    let listen_port = drafts.listen_port.trim().parse::<u16>().ok();
    dispatch(
        app,
        NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                node_name: Some(drafts.node_name),
                endpoint: Some(drafts.endpoint),
                tunnel_ip: Some(drafts.tunnel_ip),
                listen_port,
                magic_dns_suffix: Some(drafts.magic_dns_suffix),
                ..SettingsPatch::default()
            },
        },
    );
}

fn device_row(
    app: &AppRef,
    parent: &gtk::Box,
    network: &NativeNetworkState,
    participant: &NativeParticipantState,
    state: &NativeAppState,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("nvpn-device-row");
    row.set_valign(gtk::Align::Center);

    let dot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dot.add_css_class(if participant.reachable {
        "nvpn-peer-online"
    } else {
        "nvpn-peer-offline"
    });
    row.append(&dot);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);

    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = gtk::Label::new(Some(&device_name(participant)));
    name.add_css_class("heading");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name_row.append(&name);
    if participant.is_admin {
        name_row.append(&badge("Admin", "muted"));
    }
    if participant.offers_exit_node {
        name_row.append(&badge("Exit", "warn"));
    }
    text.append(&name_row);

    let subtitle = gtk::Label::new(Some(&device_subtitle(participant)));
    subtitle.add_css_class("caption");
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    text.append(&subtitle);
    row.append(&text);

    row.append(&badge(
        &device_status_text(participant),
        if participant.reachable { "ok" } else { "muted" },
    ));

    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some("Copy npub"));
    {
        let npub = participant.npub.clone();
        copy.connect_clicked(move |_| copy_text(&npub));
    }
    row.append(&copy);

    if network.local_is_admin && !is_self(participant, state) {
        let admin = gtk::Button::from_icon_name(if participant.is_admin {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        admin.set_tooltip_text(Some(if participant.is_admin {
            "Remove admin"
        } else {
            "Make admin"
        }));
        {
            let app = app.clone();
            let network_id = network.id.clone();
            let npub = participant.npub.clone();
            let is_admin = participant.is_admin;
            admin.connect_clicked(move |_| {
                dispatch(
                    &app,
                    if is_admin {
                        NativeAppAction::RemoveAdmin {
                            network_id: network_id.clone(),
                            npub: npub.clone(),
                        }
                    } else {
                        NativeAppAction::AddAdmin {
                            network_id: network_id.clone(),
                            npub: npub.clone(),
                        }
                    },
                );
            });
        }
        row.append(&admin);

        let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
        remove.set_tooltip_text(Some("Remove device"));
        remove.add_css_class("destructive-action");
        {
            let app = app.clone();
            let network_id = network.id.clone();
            let npub = participant.npub.clone();
            remove.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::RemoveParticipant {
                        network_id: network_id.clone(),
                        npub: npub.clone(),
                    },
                );
            });
        }
        row.append(&remove);
    }

    parent.append(&row);
}

fn relay_row(app: &AppRef, parent: &gtk::Box, relay: &NativeRelayState) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Center);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(&relay.url));
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title.add_css_class("heading");
    text.append(&title);
    let subtitle = gtk::Label::new(Some(&non_empty_or(&relay.status_text, &relay.state)));
    subtitle.add_css_class("caption");
    subtitle.add_css_class("dim-label");
    subtitle.set_xalign(0.0);
    text.append(&subtitle);
    row.append(&text);
    row.append(&badge(
        &relay.state,
        if relay.state == "up" { "ok" } else { "muted" },
    ));

    let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
    remove.set_tooltip_text(Some("Remove relay"));
    {
        let app = app.clone();
        let relay = relay.url.clone();
        remove.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::RemoveRelay {
                    relay: relay.clone(),
                },
            );
        });
    }
    row.append(&remove);
    parent.append(&row);
}

fn switch_row<F>(app: &AppRef, parent: &gtk::Box, title: &str, active: bool, action: F)
where
    F: Fn(bool) -> NativeAppAction + 'static,
{
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    let switch = gtk::Switch::builder().active(active).build();
    {
        let app = app.clone();
        switch.connect_active_notify(move |switch| {
            dispatch(&app, action(switch.is_active()));
        });
    }
    row.append(&switch);
    parent.append(&row);
}

fn row_label(parent: &gtk::Box, title: &str, body: &str, icon_name: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Start);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("dim-label");
        row.append(&icon);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(title));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    text.append(&title);
    let body = gtk::Label::new(Some(body));
    body.add_css_class("caption");
    body.add_css_class("dim-label");
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_selectable(true);
    text.append(&body);
    row.append(&text);
    parent.append(&row);
}

fn detail_row(parent: &gtk::Box, title: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("caption");
    title_label.add_css_class("dim-label");
    title_label.set_xalign(0.0);
    title_label.set_width_chars(13);
    row.append(&title_label);

    let value_label = gtk::Label::new(Some(value));
    value_label.set_xalign(0.0);
    value_label.set_selectable(true);
    value_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    row.append(&value_label);
    parent.append(&row);
}

fn page_title(parent: &gtk::Box, title: &str, icon_name: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("accent");
        row.append(&icon);
    }
    let label = gtk::Label::new(Some(title));
    label.add_css_class("title-1");
    label.set_xalign(0.0);
    row.append(&label);
    parent.append(&row);
}

fn section_header(parent: &gtk::Box, title: &str, icon_name: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Center);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("dim-label");
        row.append(&icon);
    }
    let label = gtk::Label::new(Some(title));
    label.add_css_class("title-3");
    label.set_xalign(0.0);
    row.append(&label);
    parent.append(&row);
}

fn empty_row(parent: &gtk::Box, text: &str) {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_xalign(0.0);
    label.set_margin_top(4);
    parent.append(&label);
}

fn card() -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("nvpn-card");
    card.set_hexpand(true);
    card.set_margin_bottom(2);
    card
}

fn nav_button(title: &str, icon_name: &str, active: bool) -> gtk::Button {
    let button = icon_text_button(title, icon_name);
    button.add_css_class("flat");
    button.add_css_class("nvpn-nav-button");
    if active {
        button.add_css_class("active");
    }
    button
}

fn icon_text_button(title: &str, icon_name: &str) -> gtk::Button {
    let button = gtk::Button::new();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.set_valign(gtk::Align::Center);
    if !icon_name.is_empty() {
        let icon = gtk::Image::from_icon_name(icon_name);
        row.append(&icon);
    }
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.0);
    row.append(&label);
    button.set_child(Some(&row));
    button
}

fn entry(placeholder: &str, value: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_text(value);
    entry.set_hexpand(true);
    entry
}

fn badge(text: &str, style: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("nvpn-badge");
    label.add_css_class(style);
    label
}

fn clear_box(parent: &gtk::Box) {
    while let Some(child) = parent.first_child() {
        parent.remove(&child);
    }
}

fn active_network(state: &NativeAppState) -> Option<&NativeNetworkState> {
    state
        .networks
        .iter()
        .find(|network| network.enabled)
        .or_else(|| state.networks.first())
}

fn display_network_name(network: &NativeNetworkState) -> String {
    if network.name.trim().is_empty() {
        "Network".to_string()
    } else {
        network.name.clone()
    }
}

fn device_name(participant: &NativeParticipantState) -> String {
    for value in [
        participant.alias.as_str(),
        participant.magic_dns_alias.as_str(),
        participant.magic_dns_name.as_str(),
    ] {
        if !value.trim().is_empty() {
            return value.to_string();
        }
    }
    short_text(&participant.npub, 18)
}

fn device_subtitle(participant: &NativeParticipantState) -> String {
    let ip = clean_ip(&participant.tunnel_ip);
    let id = short_text(&participant.npub, 18);
    if ip.is_empty() {
        id
    } else {
        format!("{id}  {ip}")
    }
}

fn device_status_text(participant: &NativeParticipantState) -> String {
    for value in [
        participant.presence_state.as_str(),
        participant.state.as_str(),
        participant.status_text.as_str(),
    ] {
        if !value.trim().is_empty() {
            return value.to_string();
        }
    }
    if participant.reachable {
        "Online".to_string()
    } else {
        "Offline".to_string()
    }
}

fn exit_node_candidates(network: &NativeNetworkState) -> Vec<NativeParticipantState> {
    let mut candidates = network.participants.clone();
    candidates.sort_by_key(device_name);
    candidates
}

fn is_self(participant: &NativeParticipantState, state: &NativeAppState) -> bool {
    (!state.own_npub.is_empty() && participant.npub == state.own_npub)
        || (!state.own_pubkey_hex.is_empty() && participant.pubkey_hex == state.own_pubkey_hex)
}

fn hero_subtitle(state: &NativeAppState) -> String {
    if state.session_active {
        format!(
            "{} of {} devices connected",
            state.connected_peer_count, state.expected_peer_count
        )
    } else if state.vpn_session_control_supported {
        "Ready to connect this device to your private network".to_string()
    } else {
        non_empty_or(
            &state.runtime_status_detail,
            "Session control is unavailable",
        )
    }
}

fn clean_ip(value: &str) -> String {
    value.split('/').next().unwrap_or(value).trim().to_string()
}

fn short_text(value: &str, keep: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= keep * 2 + 3 {
        return trimmed.to_string();
    }
    format!(
        "{}...{}",
        &trimmed[..keep],
        &trimmed[trimmed.len() - keep..]
    )
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn copy_text(value: &str) {
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(value);
    }
}

fn default_data_dir() -> String {
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data_home)
            .join("nostr-vpn")
            .to_string_lossy()
            .to_string();
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("nostr-vpn")
            .to_string_lossy()
            .to_string();
    }
    "nostr-vpn".to_string()
}

fn bootstrap_session_bus() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return;
    }
    let socket = "/tmp/nostr-vpn-dbus.sock";
    if std::path::Path::new(socket).exists() {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={socket}"));
    }
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

const CSS: &str = r#"
.nvpn-root,
.nvpn-content,
.nvpn-content viewport {
    background: @window_bg_color;
}

.nvpn-sidebar {
    padding: 8px;
    border-radius: 8px;
    background: alpha(@card_bg_color, 0.58);
}

.nvpn-sidebar-summary {
    padding: 8px 10px;
}

.nvpn-nav-button {
    padding: 8px 10px;
    border-radius: 8px;
}

.nvpn-nav-button.active {
    background: alpha(#3584e4, 0.14);
    color: @window_fg_color;
}

.nvpn-card {
    padding: 16px;
    border-radius: 8px;
    background: @card_bg_color;
    box-shadow: inset 0 0 0 1px alpha(@window_fg_color, 0.08);
}

.nvpn-hero {
    padding: 20px;
}

.nvpn-status-ready,
.nvpn-status-active,
.nvpn-status-off,
.nvpn-peer-online,
.nvpn-peer-offline {
    min-width: 14px;
    min-height: 14px;
    border-radius: 999px;
}

.nvpn-status-ready {
    min-width: 48px;
    min-height: 48px;
    background: #16a34a;
}

.nvpn-status-active {
    min-width: 48px;
    min-height: 48px;
    background: #d97706;
}

.nvpn-status-off {
    min-width: 48px;
    min-height: 48px;
    background: alpha(@window_fg_color, 0.22);
}

.nvpn-peer-online {
    background: #16a34a;
}

.nvpn-peer-offline {
    background: alpha(@window_fg_color, 0.24);
}

.nvpn-device-row {
    padding: 10px 0;
}

.nvpn-route-choice {
    padding: 0;
}

.nvpn-route-choice > box {
    padding: 10px;
    border-radius: 8px;
    background: alpha(@window_fg_color, 0.04);
}

.nvpn-badge {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 0.78em;
}

.nvpn-badge.ok {
    background: alpha(#16a34a, 0.16);
    color: #15803d;
}

.nvpn-badge.warn {
    background: alpha(#d97706, 0.16);
    color: #b45309;
}

.nvpn-badge.muted {
    background: alpha(@window_fg_color, 0.08);
    color: alpha(@window_fg_color, 0.72);
}

.success,
.accent {
    color: #15803d;
}
"#;
