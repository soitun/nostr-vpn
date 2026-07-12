package org.nostrvpn.app

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import org.json.JSONObject
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.NativeActions

internal fun androidx.compose.foundation.lazy.LazyListScope.walletPage(
    state: AppState,
    dispatch: (JSONObject) -> Unit,
) {
    item {
        Text(
            "Pay for internet access and receive earnings when you sell bandwidth.",
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
    }
    item {
        AppCard {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = state.walletFiatEnabled,
                    onCheckedChange = { enabled ->
                        dispatch(NativeActions.updateSettings("walletFiatEnabled" to enabled))
                    },
                )
                Text("Show fiat value")
            }
            if (state.walletFiatEnabled) {
                Text("Rates from Coinbase and Kraken", color = Muted, style = MaterialTheme.typography.bodySmall)
                var currencyMenuExpanded by remember { mutableStateOf(false) }
                Box {
                    Button(onClick = { currencyMenuExpanded = true }) {
                        Text("Currency ${state.walletFiatCurrency}")
                    }
                    DropdownMenu(
                        expanded = currencyMenuExpanded,
                        onDismissRequest = { currencyMenuExpanded = false },
                    ) {
                        listOf("USD", "EUR", "GBP", "CAD", "AUD", "JPY", "CHF").forEach { currency ->
                            DropdownMenuItem(
                                text = { Text(currency) },
                                onClick = {
                                    currencyMenuExpanded = false
                                    dispatch(NativeActions.updateSettings("walletFiatCurrency" to currency))
                                },
                            )
                        }
                    }
                }
            }
        }
    }
    item { PaidRouteMarketCard(state, dispatch, PaidRouteCardMode.Wallet) }
}
