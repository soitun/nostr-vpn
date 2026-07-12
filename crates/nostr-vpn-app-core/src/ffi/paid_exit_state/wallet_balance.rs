fn compact_wallet_balance_text(total_balance_msat: u64) -> String {
    let sats = total_balance_msat / 1_000;
    if sats < 1_000 {
        return format!("{sats}₿");
    }

    let (divisor, suffix) = if sats < 1_000_000 {
        (1_000, "K")
    } else {
        (1_000_000, "M")
    };
    format!("{}{suffix}₿", compact_decimal(sats, divisor))
}

fn compact_decimal(value: u64, divisor: u64) -> String {
    let whole = value / divisor;
    let decimals = if whole >= 100 {
        0
    } else if whole >= 10 {
        1
    } else {
        2
    };
    if decimals == 0 {
        return whole.to_string();
    }

    let scale = 10_u128.pow(decimals);
    let scaled = (u128::from(value) * scale + u128::from(divisor / 2))
        / u128::from(divisor);
    let rounded_whole = scaled / scale;
    let fraction = scaled % scale;
    if fraction == 0 {
        return rounded_whole.to_string();
    }
    let fraction = format!("{fraction:0>width$}", width = decimals as usize)
        .trim_end_matches('0')
        .to_string();
    format!("{rounded_whole}.{fraction}")
}
