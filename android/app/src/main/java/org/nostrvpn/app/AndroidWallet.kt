package org.nostrvpn.app

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import org.json.JSONObject
import org.nostrvpn.app.core.AppState

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
    item { PaidRouteMarketCard(state, dispatch, PaidRouteCardMode.Wallet) }
}
