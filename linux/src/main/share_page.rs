fn build_network_setup(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
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

    let join_card = card();
    section_header(&join_card, "Join Network", "go-down-symbolic");
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
    let camera = icon_text_button("Scan", "camera-photo-symbolic");
    {
        let app = app.clone();
        camera.connect_clicked(move |button| scan_invite_qr(&app, button));
    }
    let image = icon_text_button("From file", "insert-image-symbolic");
    {
        let app = app.clone();
        image.connect_clicked(move |button| choose_invite_qr_image(&app, button));
    }
    import_row.append(&invite_entry);
    import_row.append(&import);
    import_row.append(&camera);
    import_row.append(&image);
    join_card.append(&import_row);
    append_manual_join(app, &join_card);
    append_notice(app, &join_card, "Join");
    page.append(&join_card);

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

    let add_network_join_status = app.borrow().add_network_join_status.clone();
    if !add_network_join_status.trim().is_empty() || network.outbound_join_request.is_some() {
        column.append(&badge("Join request sent", "warn"));
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
        column.append(&request);
    }

    row.append(&column);
    invite.append(&row);
    page.append(&invite);

    append_join_requests(app, page, &network);

    let join_card = card();
    section_header(&join_card, "Join Network", "go-down-symbolic");

    let import_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let invite_entry = entry("Paste invite", &app.borrow().drafts.invite);
    {
        let app = app.clone();
        invite_entry.connect_changed(move |entry| {
            let value = entry.text().to_string();
            app.borrow_mut().drafts.invite.clone_from(&value);
            // Auto-import as soon as the entry holds a valid invite URL —
            // no extra click required. Mirrors the Windows / mobile UX.
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
    let image = icon_text_button("From file", "insert-image-symbolic");
    {
        let app = app.clone();
        image.connect_clicked(move |button| choose_invite_qr_image(&app, button));
    }
    let camera = icon_text_button("Scan", "camera-photo-symbolic");
    {
        let app = app.clone();
        camera.connect_clicked(move |button| scan_invite_qr(&app, button));
    }
    import_row.append(&invite_entry);
    import_row.append(&import);
    import_row.append(&camera);
    import_row.append(&image);
    join_card.append(&import_row);
    append_manual_join(app, &join_card);
    append_notice(app, &join_card, "Import");

    page.append(&join_card);

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
                    import_invite(&app, invite.clone());
                });
            }
            row.append(&join);
            nearby.append(&row);
        }
    }
    page.append(&nearby);
}

fn choose_invite_qr_image(app: &AppRef, button: &gtk::Button) {
    let parent = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    let dialog = gtk::FileDialog::builder()
        .title("Import QR image")
        .accept_label("Import")
        .build();
    let app = app.clone();
    dialog.open(parent.as_ref(), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else {
            return;
        };
        let Some(path) = file.path() else {
            set_notice(&app, "Could not open image");
            return;
        };
        match qr_scan::decode_from_path(&path) {
            Ok(invite) => import_invite(&app, invite),
            Err(error) => set_notice(&app, error),
        }
    });
}

fn scan_invite_qr(app: &AppRef, button: &gtk::Button) {
    let parent = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    let app_for_result = app.clone();
    let app_for_error = app.clone();
    qr_scan::open_scanner(
        parent.as_ref(),
        move |invite| import_invite(&app_for_result, invite),
        move |error| set_notice(&app_for_error, error),
    );
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
