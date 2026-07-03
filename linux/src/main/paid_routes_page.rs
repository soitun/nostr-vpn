fn paid_internet_available(state: &NativeAppState) -> bool {
    state.paid_route_market.supported || state.paid_exit_seller.supported
}

fn build_paid_routes_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Buy Internet", "network-wireless-symbolic");
    build_paid_route_market_card(app, page, state);
    if state.paid_exit_seller.supported {
        build_paid_exit_seller_card(app, page, state);
    }
}

fn build_paid_route_wallet_page(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    page_title(page, "Wallet", "wallet-symbolic");
    build_paid_route_wallet_card(app, page, state);
}

fn build_paid_route_market_card(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let market = &state.paid_route_market;
    let buyer = card();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    section_header(&header, "Internet Sellers", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    let find = icon_text_button("Find", "system-search-symbolic");
    find.set_sensitive(market.supported);
    {
        let app = app.clone();
        find.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::DiscoverPaidRouteOffers { duration_secs: 5 },
            );
        });
    }
    header.append(&find);
    let pay = icon_text_button("Pay", "mail-send-symbolic");
    pay.set_sensitive(market.sessions.iter().any(paid_route_session_can_sign_payment));
    {
        let app = app.clone();
        pay.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::StreamPaidRoutePayments {
                    publish: true,
                    min_increment_msat: 1,
                    limit: 0,
                },
            );
        });
    }
    header.append(&pay);
    buyer.append(&header);

    detail_row(
        &buyer,
        "Wallet",
        &non_empty_or(
            &market.wallet.total_balance_text,
            &format_paid_route_msat(market.wallet.total_balance_msat),
        ),
    );
    detail_row(&buyer, "Status", &market.status_text);
    detail_row(
        &buyer,
        "Payments",
        &paid_route_payment_action_text(&market.last_payment_action),
    );
    if !market.supported {
        empty_row(&buyer, "Buying internet is not supported on this platform");
        page.append(&buyer);
        return;
    }

    build_paid_route_filter(app, &buyer);

    section_header(&buyer, "Available", "");
    let offers = if market.hidden_offer_count > 0 || !market.visible_offers.is_empty() {
        &market.visible_offers
    } else {
        &market.offers
    };
    if market.offers.is_empty() {
        empty_row(&buyer, "No internet sellers found");
    } else if offers.is_empty() {
        empty_row(&buyer, "No matching sellers");
    } else {
        if market.hidden_offer_count > 0 {
            buyer.append(&badge(
                &format!("{} hidden by filters", market.hidden_offer_count),
                "muted",
            ));
        }
        for offer in offers.iter().take(8) {
            paid_route_offer_row(app, &buyer, offer);
        }
    }

    section_header(&buyer, "Your Paid Internet", "");
    if market.sessions.is_empty() {
        empty_row(&buyer, "No seller selected");
    } else {
        for session in &market.sessions {
            paid_route_session_row(
                app,
                &buyer,
                session,
                market.last_payment_action.envelope_json.as_str(),
                false,
            );
        }
    }

    page.append(&buyer);
}

fn build_paid_route_filter(app: &AppRef, parent: &gtk::Box) {
    let filter = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let country = entry("Country", &app.borrow().drafts.paid_route_country);
    {
        let app = app.clone();
        country.connect_changed(move |entry| {
            app.borrow_mut().drafts.paid_route_country = entry.text().to_string();
        });
    }
    let network_class = entry("Class", &app.borrow().drafts.paid_route_network_class);
    {
        let app = app.clone();
        network_class.connect_changed(move |entry| {
            app.borrow_mut().drafts.paid_route_network_class = entry.text().to_string();
        });
    }
    let apply = icon_text_button("Filter", "view-filter-symbolic");
    {
        let app = app.clone();
        apply.connect_clicked(move |_| {
            let drafts = app.borrow().drafts.clone();
            dispatch(
                &app,
                NativeAppAction::SetPaidRouteMarketFilter {
                    query: String::new(),
                    country_code: drafts.paid_route_country.trim().to_string(),
                    network_class: drafts.paid_route_network_class.trim().to_string(),
                    mint_url: String::new(),
                    require_ipv4: false,
                    require_ipv6: false,
                    sort: "quality".to_string(),
                },
            );
        });
    }
    let clear = icon_text_button("Clear", "edit-clear-symbolic");
    {
        let app = app.clone();
        clear.connect_clicked(move |_| {
            {
                let mut model = app.borrow_mut();
                model.drafts.paid_route_country.clear();
                model.drafts.paid_route_network_class.clear();
            }
            dispatch(
                &app,
                NativeAppAction::SetPaidRouteMarketFilter {
                    query: String::new(),
                    country_code: String::new(),
                    network_class: String::new(),
                    mint_url: String::new(),
                    require_ipv4: false,
                    require_ipv6: false,
                    sort: "quality".to_string(),
                },
            );
        });
    }
    filter.append(&country);
    filter.append(&network_class);
    filter.append(&apply);
    filter.append(&clear);
    parent.append(&filter);
}

fn paid_route_offer_row(app: &AppRef, parent: &gtk::Box, offer: &NativePaidRouteOfferState) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);

    let title = gtk::Label::new(Some(&paid_route_offer_title(offer)));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    text.append(&title);
    let status = gtk::Label::new(Some(&non_empty_or(&offer.status_text, &offer.seller_npub)));
    status.add_css_class("caption");
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    status.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    text.append(&status);
    let metrics = paid_route_metric_text(
        &non_empty_or(
            &offer.quality_text,
            &paid_route_quality_text(offer.latency_ms, offer.jitter_ms, offer.packet_loss_ppm),
        ),
        &offer.bandwidth_text,
    );
    if !metrics.is_empty() {
        let label = gtk::Label::new(Some(&metrics));
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        label.set_xalign(0.0);
        text.append(&label);
    }
    row.append(&text);

    let connect = icon_text_button("Connect", "go-next-symbolic");
    connect.set_sensitive(!offer.key.is_empty());
    {
        let app = app.clone();
        let offer_key = offer.key.clone();
        connect.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::BuyPaidRouteOffer {
                    offer_key: offer_key.clone(),
                    mint_url: None,
                    channel_capacity_sat: None,
                },
            );
        });
    }
    row.append(&connect);
    parent.append(&row);
}

fn paid_route_session_row(
    app: &AppRef,
    parent: &gtk::Box,
    session: &NativePaidRouteSessionState,
    envelope_json: &str,
    seller_view: bool,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);

    let title_text = if seller_view {
        paid_exit_seller_session_title(session)
    } else {
        paid_route_buyer_session_title(session)
    };
    let title = gtk::Label::new(Some(&title_text));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    text.append(&title);

    for line in paid_route_session_lines(session) {
        let label = gtk::Label::new(Some(&line));
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&label);
    }
    row.append(&text);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if seller_view {
        let collect = icon_text_button(
            &non_empty_or(&session.collect_action_text, "Collect"),
            "folder-download-symbolic",
        );
        collect.set_sensitive(paid_exit_seller_session_can_collect(session));
        {
            let app = app.clone();
            let channel_id = session.channel_id.clone();
            collect.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::CollectPaidExitChannel {
                        channel_id: channel_id.clone(),
                    },
                );
            });
        }
        buttons.append(&collect);
    } else {
        let connect = icon_text_button("Connect", "go-next-symbolic");
        {
            let app = app.clone();
            let session_id = session.session_id.clone();
            connect.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::SelectPaidRouteSession {
                        session_id: session_id.clone(),
                        connect: true,
                    },
                );
            });
        }
        buttons.append(&connect);

        let probe = icon_text_button("Probe", "network-wireless-symbolic");
        {
            let app = app.clone();
            let session_id = session.session_id.clone();
            probe.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::ProbePaidRouteSession {
                        session_id: session_id.clone(),
                        timeout_secs: 5,
                    },
                );
            });
        }
        buttons.append(&probe);

        if paid_route_session_can_open_channel(session) {
            let fund = icon_text_button("Fund", "wallet-symbolic");
            {
                let app = app.clone();
                let session_id = session.session_id.clone();
                fund.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        NativeAppAction::OpenPaidRouteChannelFromWallet {
                            session_id: session_id.clone(),
                            mint_url: None,
                            paid_msat: None,
                            max_amount_per_output: None,
                            keyset_id: None,
                        },
                    );
                });
            }
            buttons.append(&fund);
        }
        if paid_route_session_can_sign_payment(session) {
            let pay = icon_text_button("Pay", "mail-send-symbolic");
            {
                let app = app.clone();
                let session_id = session.session_id.clone();
                pay.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        NativeAppAction::SignPaidRoutePaymentEnvelopeFromWallet {
                            session_id: session_id.clone(),
                            kind: "balance-update".to_string(),
                            delivered_units: None,
                            paid_msat: None,
                        },
                    );
                });
            }
            buttons.append(&pay);
        }
        if paid_route_session_can_close_channel(session) {
            let settle = icon_text_button("Settle", "emblem-ok-symbolic");
            {
                let app = app.clone();
                let session_id = session.session_id.clone();
                settle.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        NativeAppAction::ClosePaidRouteChannelFromWallet {
                            session_id: session_id.clone(),
                            publish: true,
                        },
                    );
                });
            }
            buttons.append(&settle);
        }
        if !envelope_json.is_empty() {
            let send = icon_text_button("Send", "mail-send-symbolic");
            {
                let app = app.clone();
                let envelope_json = envelope_json.to_string();
                send.connect_clicked(move |_| {
                    dispatch(
                        &app,
                        NativeAppAction::SendPaidRoutePaymentEnvelope {
                            envelope_json: envelope_json.clone(),
                        },
                    );
                });
            }
            buttons.append(&send);
        }
    }
    row.append(&buttons);
    parent.append(&row);
}

fn build_paid_route_wallet_card(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let wallet = &state.paid_route_market.wallet;
    let card = card();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    section_header(&header, "Cashu Wallet", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    let refresh = icon_text_button("Refresh", "view-refresh-symbolic");
    {
        let app = app.clone();
        refresh.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::RefreshPaidRouteWallet { refresh: true },
            );
        });
    }
    header.append(&refresh);
    card.append(&header);

    detail_row(
        &card,
        "Balance",
        &non_empty_or(
            &wallet.total_balance_text,
            &format_paid_route_msat(wallet.total_balance_msat),
        ),
    );
    detail_row(
        &card,
        "Status",
        &paid_route_wallet_action_text(&wallet.last_action),
    );

    wallet_form_row(app, &card, "Mint URL", "Add", "list-add-symbolic", |drafts| {
        drafts.paid_route_mint_url.clone()
    });
    wallet_amount_row(app, &card, "Top-up sats", "Top Up", "go-up-symbolic", WalletAction::TopUp);
    wallet_amount_row(
        app,
        &card,
        "Export sats",
        "Export",
        "document-send-symbolic",
        WalletAction::Send,
    );
    wallet_token_row(app, &card);
    wallet_withdraw_row(app, &card);

    section_header(&card, "Mints", "");
    if wallet.mints.is_empty() {
        empty_row(&card, "No wallet mints");
    } else {
        for mint in &wallet.mints {
            paid_route_mint_row(app, &card, mint);
        }
    }
    page.append(&card);
}

fn wallet_form_row<F>(app: &AppRef, parent: &gtk::Box, placeholder: &str, button: &str, icon: &str, value: F)
where
    F: Fn(&Drafts) -> String,
{
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let input = entry(placeholder, &value(&app.borrow().drafts));
    {
        let app = app.clone();
        input.connect_changed(move |entry| {
            app.borrow_mut().drafts.paid_route_mint_url = entry.text().to_string();
        });
    }
    let add = icon_text_button(button, icon);
    {
        let app = app.clone();
        add.connect_clicked(move |_| {
            let url = app.borrow().drafts.paid_route_mint_url.trim().to_string();
            if !url.is_empty() {
                dispatch(
                    &app,
                    NativeAppAction::AddPaidRouteWalletMint { url, label: None },
                );
            }
        });
    }
    row.append(&input);
    row.append(&add);
    parent.append(&row);
}

#[derive(Clone, Copy)]
enum WalletAction {
    TopUp,
    Send,
}

fn wallet_amount_row(
    app: &AppRef,
    parent: &gtk::Box,
    placeholder: &str,
    button: &str,
    icon: &str,
    action: WalletAction,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let value = {
        let drafts = &app.borrow().drafts;
        match action {
            WalletAction::TopUp => drafts.paid_route_top_up_amount.clone(),
            WalletAction::Send => drafts.paid_route_send_amount.clone(),
        }
    };
    let input = entry(placeholder, &value);
    {
        let app = app.clone();
        input.connect_changed(move |entry| {
            let mut model = app.borrow_mut();
            match action {
                WalletAction::TopUp => {
                    model.drafts.paid_route_top_up_amount = entry.text().to_string();
                }
                WalletAction::Send => {
                    model.drafts.paid_route_send_amount = entry.text().to_string();
                }
            }
        });
    }
    let submit = icon_text_button(button, icon);
    {
        let app = app.clone();
        submit.connect_clicked(move |_| {
            let (mint_url, amount) = {
                let model = app.borrow();
                let amount_text = match action {
                    WalletAction::TopUp => &model.drafts.paid_route_top_up_amount,
                    WalletAction::Send => &model.drafts.paid_route_send_amount,
                };
                (
                    optional_trimmed(&model.drafts.paid_route_mint_url),
                    parse_positive_u64(amount_text),
                )
            };
            let Some(amount_sat) = amount else {
                return;
            };
            dispatch(
                &app,
                match action {
                    WalletAction::TopUp => NativeAppAction::TopUpPaidRouteWallet {
                        mint_url,
                        amount_sat,
                    },
                    WalletAction::Send => NativeAppAction::SendPaidRouteWalletToken {
                        mint_url,
                        amount_sat,
                    },
                },
            );
        });
    }
    row.append(&input);
    row.append(&submit);
    parent.append(&row);
}

fn wallet_token_row(app: &AppRef, parent: &gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let input = entry("Cashu token", &app.borrow().drafts.paid_route_token);
    {
        let app = app.clone();
        input.connect_changed(move |entry| {
            app.borrow_mut().drafts.paid_route_token = entry.text().to_string();
        });
    }
    let import = icon_text_button("Import", "document-open-symbolic");
    {
        let app = app.clone();
        import.connect_clicked(move |_| {
            let token = app.borrow().drafts.paid_route_token.trim().to_string();
            if !token.is_empty() {
                dispatch(&app, NativeAppAction::ReceivePaidRouteWalletToken { token });
            }
        });
    }
    row.append(&input);
    row.append(&import);
    parent.append(&row);
}

fn wallet_withdraw_row(app: &AppRef, parent: &gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let input = entry(
        "Lightning invoice",
        &app.borrow().drafts.paid_route_withdraw_invoice,
    );
    {
        let app = app.clone();
        input.connect_changed(move |entry| {
            app.borrow_mut().drafts.paid_route_withdraw_invoice = entry.text().to_string();
        });
    }
    let withdraw = icon_text_button("Withdraw", "go-down-symbolic");
    {
        let app = app.clone();
        withdraw.connect_clicked(move |_| {
            let (mint_url, invoice) = {
                let model = app.borrow();
                (
                    optional_trimmed(&model.drafts.paid_route_mint_url),
                    model.drafts.paid_route_withdraw_invoice.trim().to_string(),
                )
            };
            if !invoice.is_empty() {
                dispatch(
                    &app,
                    NativeAppAction::WithdrawPaidRouteWalletLightning { mint_url, invoice },
                );
            }
        });
    }
    row.append(&input);
    row.append(&withdraw);
    parent.append(&row);
}

fn paid_route_mint_row(app: &AppRef, parent: &gtk::Box, mint: &NativePaidRouteWalletMintState) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_valign(gtk::Align::Center);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(&non_empty_or(&mint.label, &mint.url)));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    text.append(&title);
    let status = if mint.balance_known {
        non_empty_or(&mint.balance_text, &format_paid_route_msat(mint.balance_msat))
    } else {
        "Balance unknown".to_string()
    };
    let status = gtk::Label::new(Some(&status));
    status.add_css_class("caption");
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    text.append(&status);
    row.append(&text);
    if mint.is_default {
        row.append(&badge("Default", "ok"));
    } else {
        let make_default = icon_text_button("Default", "object-select-symbolic");
        {
            let app = app.clone();
            let url = mint.url.clone();
            make_default.connect_clicked(move |_| {
                dispatch(
                    &app,
                    NativeAppAction::SetPaidRouteDefaultMint { url: url.clone() },
                );
            });
        }
        row.append(&make_default);
    }
    let remove = icon_text_button("Remove", "edit-delete-symbolic");
    {
        let app = app.clone();
        let url = mint.url.clone();
        remove.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::RemovePaidRouteWalletMint { url: url.clone() },
            );
        });
    }
    row.append(&remove);
    parent.append(&row);
}

fn build_paid_exit_seller_card(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
    let seller = &state.paid_exit_seller;
    let seller_card = card();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    section_header(&header, "Sell Internet", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    let enabled = gtk::Switch::builder().active(seller.enabled).build();
    enabled.set_sensitive(seller.supported);
    {
        let app = app.clone();
        enabled.connect_active_notify(move |switch| {
            dispatch(
                &app,
                NativeAppAction::UpdateSettings {
                    patch: SettingsPatch {
                        paid_exit_enabled: Some(switch.is_active()),
                        ..SettingsPatch::default()
                    },
                },
            );
        });
    }
    header.append(&enabled);
    seller_card.append(&header);

    detail_row(&seller_card, "Status", &paid_exit_seller_status_text(seller));
    detail_row(&seller_card, "Internet", &paid_exit_seller_internet_text(seller));
    detail_row(
        &seller_card,
        "Pricing",
        &format!(
            "{} · {} · {}",
            non_empty_or(&seller.country_code, "Country unset"),
            paid_route_network_class_title(&seller.network_class),
            non_empty_or(
                &seller.price_text,
                &paid_route_price_text(
                    seller.price_msat,
                    seller.per_units,
                    &seller.meter,
                    &seller.per_units_text,
                ),
            )
        ),
    );
    detail_row(
        &seller_card,
        "Trial",
        &format!(
            "Free {} · grace {}",
            non_empty_or(
                &seller.free_probe_text,
                &paid_route_traffic_unit_text(seller.free_probe_units, &seller.meter),
            ),
            non_empty_or(
                &seller.grace_text,
                &paid_route_traffic_unit_text(seller.grace_units, &seller.meter),
            )
        ),
    );
    detail_row(&seller_card, "Public IP", &seller.public_ip_text);
    detail_row(&seller_card, "Settlement", &seller.settlement_text);
    detail_row(
        &seller_card,
        "Credit",
        &format!(
            "{} {}",
            non_empty_or(&seller.channel_credit_title_text, "Pending buyer credit"),
            non_empty_or(
                &seller.channel_credit_text,
                &format_paid_route_msat(seller.channel_credit_msat),
            )
        ),
    );
    detail_row(
        &seller_card,
        "Totals",
        &paid_exit_seller_totals_text(seller),
    );
    if seller.total_unpaid_msat > 0 {
        seller_card.append(&badge(
            &format!(
                "{} behind",
                non_empty_or(
                    &seller.total_unpaid_text,
                    &format_paid_route_msat(seller.total_unpaid_msat),
                )
            ),
            "warn",
        ));
    }

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let publish = icon_text_button("Publish", "document-send-symbolic");
    publish.set_sensitive(seller.supported && seller.enabled);
    {
        let app = app.clone();
        publish.connect_clicked(move |_| {
            dispatch(&app, NativeAppAction::PublishPaidExitOffer);
        });
    }
    actions.append(&publish);
    let receive = icon_text_button("Receive", "mail-receive-symbolic");
    receive.set_sensitive(seller.supported && seller.enabled);
    {
        let app = app.clone();
        receive.connect_clicked(move |_| {
            dispatch(
                &app,
                NativeAppAction::ReceivePaidRoutePayments { duration_secs: 5 },
            );
        });
    }
    actions.append(&receive);
    let collect = icon_text_button("Collect due", "folder-download-symbolic");
    collect.set_sensitive(seller.supported && seller.enabled);
    {
        let app = app.clone();
        collect.connect_clicked(move |_| {
            dispatch(&app, NativeAppAction::CollectDuePaidExitChannels);
        });
    }
    actions.append(&collect);
    seller_card.append(&actions);

    section_header(&seller_card, "Customers", "");
    if seller.sessions.is_empty() {
        empty_row(&seller_card, "No customers connected");
    } else {
        for session in &seller.sessions {
            paid_route_session_row(app, &seller_card, session, "", true);
        }
    }

    page.append(&seller_card);
}

fn paid_route_session_lines(session: &NativePaidRouteSessionState) -> Vec<String> {
    let mut lines = vec![paid_route_session_detail(session)];
    if !session.location_text.is_empty() {
        lines.push(session.location_text.clone());
    } else if !session.realized_exit_ip.is_empty() {
        lines.push(format!(
            "{} · {}",
            session.realized_exit_ip,
            paid_route_country_claim_text(session),
        ));
    }
    let metric = paid_route_metric_text(
        &non_empty_or(
            &session.quality_text,
            &paid_route_quality_text(
                session.latency_ms,
                session.jitter_ms,
                session.packet_loss_ppm,
            ),
        ),
        &session.bandwidth_text,
    );
    if !metric.is_empty() {
        lines.push(metric);
    }
    if !session.settlement_text.is_empty() {
        lines.push(session.settlement_text.clone());
    }
    lines.push(format!(
        "{} · {}",
        non_empty_or(
            &session.paid_text,
            &format!("{} paid", format_paid_route_msat(session.paid_msat)),
        ),
        if session.unpaid_msat > 0 {
            non_empty_or(
                &session.unpaid_text,
                &format!("{} behind", format_paid_route_msat(session.unpaid_msat)),
            )
        } else {
            non_empty_or(
                &session.amount_due_text,
                &format!("{} due", format_paid_route_msat(session.amount_due_msat)),
            )
        }
    ));
    lines
}

fn paid_route_buyer_session_title(session: &NativePaidRouteSessionState) -> String {
    if !session.title_text.is_empty() {
        session.title_text.clone()
    } else if session.allow_routing {
        "Ready".to_string()
    } else if session.unpaid_msat > 0 {
        "Payment needed".to_string()
    } else if !session.payment_channel_ready {
        "Needs funds".to_string()
    } else {
        paid_route_plain_status(
            &non_empty_or(&session.status_text, &session.lifecycle_status),
            "Session",
        )
    }
}

fn paid_exit_seller_session_title(session: &NativePaidRouteSessionState) -> String {
    if !session.title_text.is_empty() {
        session.title_text.clone()
    } else if session.allow_routing {
        "Connected customer".to_string()
    } else if session.unpaid_msat > 0 {
        "Customer behind".to_string()
    } else {
        paid_route_plain_status(
            &non_empty_or(&session.status_text, &session.lifecycle_status),
            "Customer",
        )
    }
}

fn paid_route_session_detail(session: &NativePaidRouteSessionState) -> String {
    if !session.detail_text.is_empty() {
        return session.detail_text.clone();
    }
    let access = paid_route_access_title(
        &session.access_state,
        &non_empty_or(&session.lifecycle_status, "session"),
    );
    let units = if session.bytes > 0 {
        format!("{} used", format_bytes(session.bytes))
    } else if session.packets > 0 {
        format!("{} packets", session.packets)
    } else {
        format!("{} units", session.delivered_units)
    };
    format!(
        "{access}, {units}, {} due",
        format_paid_route_msat(session.amount_due_msat)
    )
}

fn paid_route_session_can_open_channel(session: &NativePaidRouteSessionState) -> bool {
    !session.session_id.is_empty() && !session.payment_channel_ready
}

fn paid_route_session_can_sign_payment(session: &NativePaidRouteSessionState) -> bool {
    !session.session_id.is_empty() && session.payment_channel_ready && session.unpaid_msat > 0
}

fn paid_route_session_can_close_channel(session: &NativePaidRouteSessionState) -> bool {
    !session.session_id.is_empty()
        && session.payment_channel_ready
        && !matches!(session.lifecycle_status.as_str(), "closed" | "expired")
}

fn paid_exit_seller_session_can_collect(session: &NativePaidRouteSessionState) -> bool {
    session.payment_channel_ready
        && session.paid_msat > 0
        && !session.channel_id.is_empty()
        && (!session.collect_action_text.is_empty()
            || !matches!(session.lifecycle_status.as_str(), "closed" | "expired"))
}

fn paid_route_offer_title(offer: &NativePaidRouteOfferState) -> String {
    format!(
        "{} · {} · {}",
        non_empty_or(&offer.country_code, "Unknown country").to_uppercase(),
        paid_route_network_class_title(&offer.network_class),
        non_empty_or(
            &offer.price_text,
            &paid_route_price_text(
                offer.price_msat,
                offer.per_units,
                &offer.meter,
                &offer.per_units_text,
            ),
        )
    )
}

fn paid_exit_seller_status_text(seller: &NativePaidExitSellerState) -> String {
    if !seller.status_text.is_empty() {
        seller
            .status_text
            .replace("Paid exit selling", "Selling internet")
            .replace("paid exit selling", "selling internet")
    } else if seller.supported {
        "People can pay to use my internet".to_string()
    } else {
        "This platform cannot sell public internet access".to_string()
    }
}

fn paid_exit_seller_internet_text(seller: &NativePaidExitSellerState) -> String {
    if !seller.internet_text.is_empty() {
        seller.internet_text.clone()
    } else if matches!(
        seller.upstream.as_str(),
        "wireguard_exit" | "wireguard" | "wg" | "upstream_vpn" | "vpn"
    ) {
        "My internet through WireGuard".to_string()
    } else {
        "My internet".to_string()
    }
}

fn paid_exit_seller_totals_text(seller: &NativePaidExitSellerState) -> String {
    [
        format!("{} connected", seller.current_connection_count),
        format!("{} past", seller.past_connection_count),
        non_empty_or(
            &seller.total_traffic_text,
            &format!("{} routed", format_bytes(seller.total_billable_bytes)),
        ),
        format!(
            "{} paid",
            non_empty_or(
                &seller.total_paid_text,
                &format_paid_route_msat(seller.total_paid_msat),
            )
        ),
        format!(
            "{} due",
            non_empty_or(
                &seller.total_due_text,
                &format_paid_route_msat(seller.total_due_msat),
            )
        ),
    ]
    .join(" · ")
}

fn paid_route_payment_action_text(
    action: &nostr_vpn_app_core::native_state::NativePaidRoutePaymentActionState,
) -> String {
    if action.kind.is_empty() && action.status_text.is_empty() {
        String::new()
    } else {
        non_empty_or(
            &action.status_text,
            &paid_route_payment_action_title(&action.kind),
        )
    }
}

fn paid_route_wallet_action_text(
    action: &nostr_vpn_app_core::native_state::NativePaidRouteWalletActionState,
) -> String {
    if action.kind.is_empty() && action.status_text.is_empty() {
        String::new()
    } else {
        non_empty_or(&action.status_text, &paid_route_wallet_action_title(&action.kind))
    }
}

fn paid_route_payment_action_title(kind: &str) -> String {
    match kind {
        "send" => "Payment sent".to_string(),
        "receive" => "Payment received".to_string(),
        "apply" => "Payment applied".to_string(),
        "create" | "sign" => "Payment ready".to_string(),
        "open_channel" => "Exit funded".to_string(),
        "close" => "Channel settled".to_string(),
        "stream" => "Payments sent".to_string(),
        "probe" => "Quality checked".to_string(),
        "" => "Payment".to_string(),
        other => paid_route_plain_status(other, "Payment"),
    }
}

fn paid_route_wallet_action_title(kind: &str) -> String {
    match kind {
        "topup" => "Invoice ready".to_string(),
        "receive" => "Token imported".to_string(),
        "send" => "Token ready".to_string(),
        "withdraw" => "Invoice paid".to_string(),
        "refresh" => "Wallet refreshed".to_string(),
        "open_channel" => "Exit funded".to_string(),
        "" => "Wallet updated".to_string(),
        other => paid_route_plain_status(other, "Wallet updated"),
    }
}

fn paid_route_network_class_title(value: &str) -> String {
    match value {
        "datacenter" => "Datacenter".to_string(),
        "residential" => "Residential".to_string(),
        "mobile" => "Mobile".to_string(),
        "satellite" => "Satellite".to_string(),
        "community_mesh" => "Community mesh".to_string(),
        "" | "unknown" => "Unknown".to_string(),
        other => paid_route_plain_status(other, "Unknown"),
    }
}

fn paid_route_access_title(value: &str, fallback: &str) -> String {
    match value {
        "paid" => "Paid".to_string(),
        "free_probe" => "Free test".to_string(),
        "grace" => "Grace".to_string(),
        "suspended" => "Paused".to_string(),
        other => paid_route_plain_status(other, fallback),
    }
}

fn paid_route_plain_status(value: &str, fallback: &str) -> String {
    let raw = non_empty_or(value, fallback).replace('_', " ");
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn paid_route_quality_text(latency_ms: u32, jitter_ms: u32, packet_loss_ppm: u32) -> String {
    if latency_ms == 0 && jitter_ms == 0 && packet_loss_ppm == 0 {
        return "Quality unmeasured".to_string();
    }
    let loss = packet_loss_ppm as f64 / 10_000.0;
    format!("{latency_ms} ms · {jitter_ms} ms jitter · {loss:.2}% loss")
}

fn paid_route_metric_text(quality: &str, bandwidth: &str) -> String {
    [quality.trim(), bandwidth.trim()]
        .into_iter()
        .filter(|value| !value.is_empty() && *value != "Quality unmeasured")
        .collect::<Vec<_>>()
        .join(" · ")
}

fn paid_route_country_claim_text(session: &NativePaidRouteSessionState) -> String {
    match session.country_claim_status.as_str() {
        "match" => format!(
            "{} matches claim",
            non_empty_or(&session.observed_country_code, &session.claimed_country_code)
        ),
        "mismatch" => format!(
            "{} differs from {}",
            non_empty_or(&session.observed_country_code, "Observed country"),
            session.claimed_country_code,
        ),
        _ => non_empty_or(
            &session.observed_country_code,
            &non_empty_or(&session.claimed_country_code, "country unknown"),
        ),
    }
}

fn paid_route_price_text(price_msat: u64, per_units: u64, meter: &str, per_units_text: &str) -> String {
    format!(
        "{} / {}",
        format_paid_route_msat(price_msat),
        non_empty_or(per_units_text, &paid_route_meter_unit_text(per_units, meter)),
    )
}

fn paid_route_traffic_unit_text(units: u64, meter: &str) -> String {
    if meter == "bytes" {
        format_bytes(units)
    } else {
        paid_route_meter_unit_text(units, meter)
    }
}

fn paid_route_meter_unit_text(units: u64, meter: &str) -> String {
    let label = match meter {
        "packets" => "packet",
        "acked_tcp_bytes" => "acked TCP byte",
        "outbound_bytes" => "outbound byte",
        "bytes" => "byte",
        _ => "unit",
    };
    format!("{units} {label}{}", if units == 1 { "" } else { "s" })
}

fn format_paid_route_msat(msat: u64) -> String {
    if msat >= 1_000 {
        let sat = msat as f64 / 1_000.0;
        if (sat.fract()).abs() < f64::EPSILON {
            format!("{sat:.0} sat")
        } else {
            format!("{sat:.3} sat")
        }
    } else {
        format!("{msat} msat")
    }
}

fn parse_positive_u64(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok().filter(|value| *value > 0)
}

fn optional_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
