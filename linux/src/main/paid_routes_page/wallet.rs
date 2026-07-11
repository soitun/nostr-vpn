pub(super) fn build_paid_route_wallet_card(app: &AppRef, page: &gtk::Box, state: &NativeAppState) {
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

    wallet_form_row(
        app,
        &card,
        "Mint URL",
        "Add",
        "list-add-symbolic",
        |drafts| drafts.paid_route_mint_url.clone(),
    );
    wallet_amount_row(
        app,
        &card,
        "Top-up sats",
        "Top Up",
        "go-up-symbolic",
        WalletAction::TopUp,
    );
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

fn wallet_form_row<F>(
    app: &AppRef,
    parent: &gtk::Box,
    placeholder: &str,
    button: &str,
    icon: &str,
    value: F,
) where
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
        non_empty_or(
            &mint.balance_text,
            &format_paid_route_msat(mint.balance_msat),
        )
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
