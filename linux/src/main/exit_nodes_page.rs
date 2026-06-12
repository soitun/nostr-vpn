fn build_exit_nodes_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Exit Nodes", "");

    let Some(network) = active_network(state).cloned() else {
        build_wireguard_settings_card(app, page, state);
        return;
    };

    let exit = card();
    section_header(&exit, "Exit Node", "");

    let all_exit_candidates = exit_node_candidates(&network, state);
    let show_search = all_exit_candidates.len() > SEARCH_VISIBILITY_THRESHOLD;
    if show_search {
        let search = entry("Search devices", &app.borrow().drafts.exit_search);
        {
            let app = app.clone();
            search.connect_changed(move |entry| {
                app.borrow_mut().drafts.exit_search = entry.text().to_string();
            });
        }
        exit.append(&search);
    }

    let direct_selected = !state.wireguard_exit_enabled && state.exit_node.is_empty();
    route_choice(
        app,
        &exit,
        "Direct",
        "Use normal internet routing",
        direct_selected,
        true,
        ExitChoice::Direct,
    );

    let wg_subtitle = if !state.wireguard_exit_configured {
        "No WireGuard config saved yet".to_string()
    } else if state.wireguard_exit_endpoint.is_empty() {
        "Configured".to_string()
    } else {
        state.wireguard_exit_endpoint.clone()
    };
    route_choice(
        app,
        &exit,
        "WireGuard upstream",
        &wg_subtitle,
        state.wireguard_exit_enabled,
        state.wireguard_exit_configured,
        ExitChoice::WireGuard,
    );

    let query = if show_search {
        app.borrow().drafts.exit_search.trim().to_ascii_lowercase()
    } else {
        String::new()
    };
    let exit_candidates = all_exit_candidates
        .into_iter()
        .filter(|participant| {
            query.is_empty()
                || device_name(participant)
                    .to_ascii_lowercase()
                    .contains(&query)
                || participant.npub.to_ascii_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    if exit_candidates.is_empty() {
        empty_row(
            &exit,
            if query.is_empty() {
                "No exit nodes offered"
            } else {
                "No exit nodes found"
            },
        );
    } else {
        for participant in exit_candidates {
            let peer_selected =
                !state.wireguard_exit_enabled && state.exit_node == participant.npub;
            route_choice(
                app,
                &exit,
                &device_name(&participant),
                non_empty_or(&participant.status_text, "Exit node").as_str(),
                peer_selected,
                true,
                ExitChoice::Peer(participant.npub.clone()),
            );
        }
    }
    page.append(&exit);

    let offer = card();
    switch_row(
        app,
        &offer,
        "Offer this device as an exit node",
        state.advertise_exit_node,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                advertise_exit_node: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    switch_row(
        app,
        &offer,
        "Block internet if exit node disconnects",
        state.exit_node_leak_protection,
        |enabled| NativeAppAction::UpdateSettings {
            patch: SettingsPatch {
                exit_node_leak_protection: Some(enabled),
                ..SettingsPatch::default()
            },
        },
    );
    page.append(&offer);
    build_wireguard_settings_card(app, page, state);
}

#[derive(Clone)]
enum ExitChoice {
    Direct,
    WireGuard,
    Peer(String),
}

fn route_choice(
    app: &AppRef,
    parent: &gtk::Box,
    title: &str,
    subtitle: &str,
    selected: bool,
    enabled: bool,
    choice: ExitChoice,
) {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("nvpn-route-choice");
    button.set_sensitive(enabled);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let icon = gtk::Image::from_icon_name(if selected {
        "object-select-symbolic"
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
        let choice = choice.clone();
        button.connect_clicked(move |_| {
            // The daemon enforces mutual exclusion (peer vs WG), so
            // each non-direct row only sends the field it owns.
            // Direct needs to flip both because there's nothing to
            // conflict with — it means "neither".
            let patch = match choice.clone() {
                ExitChoice::Direct => SettingsPatch {
                    exit_node: Some(String::new()),
                    wireguard_exit_enabled: Some(false),
                    ..SettingsPatch::default()
                },
                ExitChoice::WireGuard => SettingsPatch {
                    wireguard_exit_enabled: Some(true),
                    ..SettingsPatch::default()
                },
                ExitChoice::Peer(npub) => SettingsPatch {
                    exit_node: Some(npub),
                    ..SettingsPatch::default()
                },
            };
            dispatch(&app, NativeAppAction::UpdateSettings { patch });
        });
    }
    parent.append(&button);
}
