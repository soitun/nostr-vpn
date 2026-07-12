package org.nostrvpn.app

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.NetworkState
import org.nostrvpn.app.update.AndroidSelfUpdateState

internal fun androidx.compose.foundation.lazy.LazyListScope.settingsPage(
    state: AppState,
    @Suppress("UNUSED_PARAMETER") network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
    selfUpdateState: AndroidSelfUpdateState,
    selfUpdateActions: SelfUpdateActions,
) {
    item { DeviceSettingsCard(state, dispatch) }
    item { GeneralSettingsCard(state, dispatch) }
    item { FipsSettingsCard(state, dispatch) }
    item { RelaySettingsCard(state, dispatch) }
    if (selfUpdateState.supported) {
        item { SelfUpdateCard(selfUpdateState, selfUpdateActions) }
    }
    item { DiagnosticsCard(state) }
}

@Composable
private fun SelfUpdateCard(
    state: AndroidSelfUpdateState,
    actions: SelfUpdateActions,
) {
    AppCard {
        Text("Updates", style = MaterialTheme.typography.titleMedium)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Switch(
                checked = state.autoCheckEnabled,
                onCheckedChange = actions.setAutoCheck,
            )
            Spacer(Modifier.width(8.dp))
            Text("Check automatically")
        }
        if (state.status.isNotBlank()) {
            Text(state.status, color = Muted, style = MaterialTheme.typography.bodySmall)
        }
        Button(
            enabled = !state.busy,
            onClick = {
                when {
                    state.downloaded -> actions.install()
                    state.available -> actions.download()
                    else -> actions.check()
                }
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(selfUpdateButtonText(state))
        }
    }
}

private fun selfUpdateButtonText(state: AndroidSelfUpdateState): String =
    when {
        state.checking -> "Checking…"
        state.downloading -> "Downloading…"
        state.downloaded -> "Install update"
        state.available -> "Download update"
        else -> "Check for updates"
    }
