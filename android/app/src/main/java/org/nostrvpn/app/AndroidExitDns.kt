package org.nostrvpn.app

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.NativeActions

@Composable
internal fun ExitDnsSettingsCard(
    state: AppState,
    dispatch: (JSONObject) -> Unit,
) {
    var mode by remember(state.exitDnsMode) { mutableStateOf(state.exitDnsMode) }
    var provider by remember(state.exitDnsDohProvider) { mutableStateOf(state.exitDnsDohProvider) }
    var customUrl by remember(state.exitDnsCustomDohUrl) { mutableStateOf(state.exitDnsCustomDohUrl) }
    var bootstrapIps by remember(state.exitDnsCustomDohBootstrapIps) {
        mutableStateOf(state.exitDnsCustomDohBootstrapIps)
    }
    var throughExitServers by remember(state.exitDnsThroughExitServers) {
        mutableStateOf(state.exitDnsThroughExitServers)
    }

    AppCard {
        Text("Exit DNS", style = MaterialTheme.typography.titleMedium)
        Text(
            "MagicDNS stays local. Public DNS follows this policy while an internet exit is active.",
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
        ChoiceButtons(
            choices = listOf(
                "automatic" to "Automatic",
                "encrypted" to "Encrypted",
                "through_exit" to "Through exit",
            ),
            selected = mode,
            onSelect = { mode = it },
        )
        when (mode) {
            "encrypted" -> {
                ChoiceButtons(
                    choices = listOf(
                        "cloudflare" to "Cloudflare",
                        "quad9" to "Quad9",
                        "custom" to "Custom",
                    ),
                    selected = provider,
                    onSelect = { provider = it },
                )
                if (provider == "custom") {
                    OutlinedTextField(
                        customUrl,
                        { customUrl = it },
                        Modifier.fillMaxWidth(),
                        label = { Text("HTTPS DoH URL") },
                        singleLine = true,
                    )
                    OutlinedTextField(
                        bootstrapIps,
                        { bootstrapIps = it },
                        Modifier.fillMaxWidth(),
                        label = { Text("Bootstrap IPs") },
                        singleLine = true,
                    )
                }
            }
            "through_exit" -> {
                OutlinedTextField(
                    throughExitServers,
                    { throughExitServers = it },
                    Modifier.fillMaxWidth(),
                    label = { Text("DNS server IPs") },
                    singleLine = true,
                )
                Text(
                    "These DNS packets are sent only through the selected exit.",
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            else -> Text(
                "Uses profile DNS when supplied; otherwise built-in encrypted DNS.",
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
            )
        }
        Button(onClick = {
            dispatch(
                NativeActions.updateSettings(
                    "exitDnsMode" to mode,
                    "exitDnsDohProvider" to provider,
                    "exitDnsCustomDohUrl" to customUrl,
                    "exitDnsCustomDohBootstrapIps" to bootstrapIps,
                    "exitDnsThroughExitServers" to throughExitServers,
                ),
            )
        }) {
            Text("Save Exit DNS")
        }
    }
}

@Composable
private fun ChoiceButtons(
    choices: List<Pair<String, String>>,
    selected: String,
    onSelect: (String) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        choices.forEach { (value, label) ->
            if (selected == value) {
                Button(
                    onClick = { onSelect(value) },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(label) }
            } else {
                OutlinedButton(
                    onClick = { onSelect(value) },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(label) }
            }
        }
    }
}
