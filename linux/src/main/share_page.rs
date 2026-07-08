fn build_network_setup(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    match app.borrow().network_setup_mode {
        None => append_setup_choices(app, page),
        Some(NetworkSetupMode::Create) => {
            append_setup_back(app, page);
            append_create_network_card(app, page);
        }
        Some(NetworkSetupMode::Join) => {
            append_setup_back(app, page);
            append_join_network_card(app, page, state, None);
            append_nearby_card(app, page, state);
        }
    }
}

fn append_setup_choices(app: &AppRef, page: &gtk::Box) {
    let choices = card();
    let create = icon_text_button("Create Network", "list-add-symbolic");
    {
        let app = app.clone();
        create.connect_clicked(move |_| {
            app.borrow_mut().network_setup_mode = Some(NetworkSetupMode::Create);
            render(&app);
        });
    }
    choices.append(&create);
    let join = icon_text_button("Join Network", "go-down-symbolic");
    {
        let app = app.clone();
        join.connect_clicked(move |_| {
            app.borrow_mut().network_setup_mode = Some(NetworkSetupMode::Join);
            render(&app);
        });
    }
    choices.append(&join);
    page.append(&choices);
}

fn append_setup_back(app: &AppRef, page: &gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let back = icon_text_button("Back", "go-previous-symbolic");
    {
        let app = app.clone();
        back.connect_clicked(move |_| {
            app.borrow_mut().network_setup_mode = None;
            render(&app);
        });
    }
    row.append(&back);
    page.append(&row);
}

fn append_create_network_card(app: &AppRef, page: &gtk::Box) {
    let create_card = card();
    section_header(&create_card, "Create Network", "list-add-symbolic");
    let create_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = entry("Network name", &app.borrow().drafts.new_network_name);
    {
        let app = app.clone();
        name.connect_changed(move |entry| {
            app.borrow_mut().drafts.new_network_name = entry.text().to_string();
        });
    }
    let create = icon_text_button("Create", "list-add-symbolic");
    {
        let app = app.clone();
        create.connect_clicked(move |_| {
            let name = {
                let mut model = app.borrow_mut();
                let name = model.drafts.new_network_name.trim().to_string();
                model.drafts.new_network_name.clear();
                name
            };
            create_network(&app, name);
        });
    }
    create_row.append(&name);
    create_row.append(&create);
    create_card.append(&create_row);
    page.append(&create_card);
}

fn append_join_network_card(
    app: &AppRef,
    page: &gtk::Box,
    state: &NativeAppState,
    request_network: Option<&NativeNetworkState>,
) {
    let request_network = request_network.or_else(|| {
        state.networks.iter().find(|network| {
            network.outbound_join_request.is_some()
                || !network.join_request_qr_code_or_link.is_empty()
                || !network.invite_inviter_npub.is_empty()
        })
    });
    let join_request = if state.join_request_qr_code_or_link.is_empty() {
        request_network
            .map(|network| network.join_request_qr_code_or_link.as_str())
            .unwrap_or("")
    } else {
        &state.join_request_qr_code_or_link
    };

    let join_card = card();
    section_header(&join_card, "Join Network", "go-down-symbolic");
    if !join_request.is_empty() {
        join_card.append(&qr::build(join_request, 220));
        let copy = icon_text_button("Copy request", "edit-copy-symbolic");
        {
            let request = join_request.to_string();
            copy.connect_clicked(move |_| copy_text(&request));
        }
        join_card.append(&copy);
    }

    let legacy = gtk::Expander::new(Some("Legacy invite link"));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_top(8);
    let import_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let invite_entry = entry("Paste invite", &app.borrow().drafts.invite);
    {
        let app = app.clone();
        invite_entry.connect_changed(move |entry| {
            let value = entry.text().to_string();
            app.borrow_mut().drafts.invite.clone_from(&value);
            let trimmed = value.trim();
            if trimmed.starts_with("nvpn://invite/") {
                import_invite(&app, trimmed.to_string());
            }
        });
    }
    let import = icon_text_button("Import", "go-down-symbolic");
    {
        let app = app.clone();
        import.connect_clicked(move |_| {
            let invite = app.borrow().drafts.invite.trim().to_string();
            import_invite(&app, invite);
        });
    }
    import_row.append(&invite_entry);
    import_row.append(&import);
    body.append(&import_row);
    if let Some(network) = request_network {
        let add_network_join_status = app.borrow().add_network_join_status.clone();
        if !add_network_join_status.trim().is_empty() || network.outbound_join_request.is_some() {
            body.append(&badge("Join request sent", "warn"));
        } else if !network.invite_inviter_npub.is_empty() {
            let request = icon_text_button("Request Access", "contact-new-symbolic");
            {
                let app = app.clone();
                let network_id = network.id.clone();
                request.connect_clicked(move |_| {
                    let state = dispatch(
                        &app,
                        NativeAppAction::RequestNetworkJoin {
                            network_id: network_id.clone(),
                        },
                    );
                    if state.error.trim().is_empty() {
                        app.borrow_mut().add_network_join_status = "Join request sent".to_string();
                        render(&app);
                    }
                });
            }
            body.append(&request);
        }
    }
    legacy.set_child(Some(&body));
    join_card.append(&legacy);
    append_manual_join(app, &join_card);
    append_notice(app, &join_card, "Join");
    page.append(&join_card);
}

fn append_nearby_card(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let nearby = card();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    section_header(&header, "Nearby invites", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    let nearby_label = if state.nearby_discovery_active {
        format!(
            "Finding nearby · {}",
            remaining_text(state.nearby_discovery_remaining_secs)
        )
    } else {
        "Find nearby".to_string()
    };
    let lan = icon_text_button(
        &nearby_label,
        if state.nearby_discovery_active {
            "media-playback-stop-symbolic"
        } else {
            "system-search-symbolic"
        },
    );
    {
        let app = app.clone();
        let active = state.nearby_discovery_active;
        lan.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::StopNearbyDiscovery
                } else {
                    NativeAppAction::StartNearbyDiscovery
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
            let title = gtk::Label::new(Some(if peer.node_name.trim().is_empty() {
                &peer.network_name
            } else {
                &peer.node_name
            }));
            title.set_xalign(0.0);
            title.add_css_class("heading");
            text.append(&title);
            let sub = gtk::Label::new(Some(&peer.last_seen_text));
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
                join.connect_clicked(move |_| import_invite(&app, invite.clone()));
            }
            row.append(&join);
            nearby.append(&row);
        }
    }
    page.append(&nearby);
}

fn build_share_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Share", "emblem-shared-symbolic");

    if state.paid_exit_seller.supported {
        build_paid_exit_seller_card(app, page, state);
    }

    let Some(network) = active_network(state).cloned() else {
        return;
    };

    let invite = card();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    row.set_valign(gtk::Align::Start);
    row.append(&qr::build(&state.active_network_invite, 260));

    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_hexpand(true);
    section_header(&column, "Invite Devices", "emblem-shared-symbolic");

    let invite_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let copy = icon_text_button("Copy Link", "edit-copy-symbolic");
    copy.set_sensitive(!state.active_network_invite.is_empty());
    {
        let invite = state.active_network_invite.clone();
        copy.connect_clicked(move |_| copy_text(&invite));
    }
    invite_row.append(&copy);
    let reset = icon_text_button("Reset", "view-refresh-symbolic");
    reset.set_sensitive(network.local_is_admin && network.enabled);
    {
        let app = app.clone();
        let network_id = network.id.clone();
        reset.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::ResetNetworkInvite {
                    network_id: network_id.clone(),
                },
            );
        });
    }
    invite_row.append(&reset);
    let broadcast_label = if state.invite_broadcast_active {
        format!(
            "Sharing nearby · {}",
            remaining_text(state.invite_broadcast_remaining_secs)
        )
    } else {
        "Share invite nearby".to_string()
    };
    let broadcast = icon_text_button(
        &broadcast_label,
        if state.invite_broadcast_active {
            "media-playback-stop-symbolic"
        } else {
            "network-wireless-symbolic"
        },
    );
    {
        let app = app.clone();
        let active = state.invite_broadcast_active;
        broadcast.connect_clicked(move |_| {
            dispatch(
                &app,
                if active {
                    NativeAppAction::StopInviteBroadcast
                } else {
                    NativeAppAction::StartInviteBroadcast
                },
            );
        });
    }
    invite_row.append(&broadcast);
    column.append(&invite_row);
    switch_row_enabled(
        app,
        &column,
        "Allow join requests",
        network.join_requests_enabled,
        network.local_is_admin,
        {
            let network_id = network.id.clone();
            move |enabled| NativeAppAction::SetNetworkJoinRequestsEnabled {
                network_id: network_id.clone(),
                enabled,
            }
        },
    );

    row.append(&column);
    invite.append(&row);
    page.append(&invite);

    append_join_requests(app, page, &network);
    append_join_network_card(app, page, state, None);
    append_nearby_card(app, page, state);
}

fn append_notice(app: &AppRef, parent: &gtk::Box, title: &str) {
    let notice = app.borrow().notice.clone();
    if !notice.trim().is_empty() {
        row_label(parent, title, &notice, "dialog-warning-symbolic");
    }
}

fn append_manual_join(app: &AppRef, parent: &gtk::Box) {
    let manual = gtk::Expander::new(Some("Add manually"));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_top(8);

    let admin = entry("Admin Device ID", &app.borrow().drafts.manual_join_admin_id);
    {
        let app = app.clone();
        admin.connect_changed(move |entry| {
            app.borrow_mut().drafts.manual_join_admin_id = entry.text().to_string();
        });
    }
    body.append(&admin);

    let network = entry(
        "Network ID",
        &display_network_id(&app.borrow().drafts.manual_join_network_id),
    );
    {
        let app = app.clone();
        network.connect_changed(move |entry| {
            app.borrow_mut().drafts.manual_join_network_id = entry.text().to_string();
        });
    }
    body.append(&network);

    let add = icon_text_button("Add", "list-add-symbolic");
    add.set_halign(gtk::Align::Start);
    {
        let app = app.clone();
        add.connect_clicked(move |_| manual_add_network(&app));
    }
    body.append(&add);

    manual.set_child(Some(&body));
    parent.append(&manual);
}

fn manual_add_network(app: &AppRef) {
    let (admin_npub, mesh_network_id) = {
        let model = app.borrow();
        (
            model.drafts.manual_join_admin_id.trim().to_string(),
            normalize_network_id_input(&model.drafts.manual_join_network_id),
        )
    };
    if admin_npub.is_empty() || mesh_network_id.is_empty() {
        return;
    }
    if !is_valid_device_id(&admin_npub) {
        set_notice(app, "Not a valid device ID");
        return;
    }
    {
        let mut model = app.borrow_mut();
        model.drafts.manual_join_admin_id.clear();
        model.drafts.manual_join_network_id.clear();
        model.notice.clear();
    }
    dispatch(
        app,
        NativeAppAction::ManualAddNetwork {
            admin_npub,
            mesh_network_id,
        },
    );
}
