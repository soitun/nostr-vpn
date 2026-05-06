package org.nostrvpn.app

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.NativeActions
import org.nostrvpn.app.core.NetworkState
import org.nostrvpn.app.core.activeNetwork

private enum class Page(val title: String) {
    Devices("Devices"),
    Share("Share"),
    Routing("Routing"),
    Settings("Settings"),
}

@Composable
internal fun NostrVpnTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = lightColorScheme(
            primary = Color(0xFF8B5CF6),
            secondary = Color(0xFF22D3EE),
            background = Color(0xFFF6F7F8),
            surface = Color.White,
            onPrimary = Color.White,
            onSecondary = Color(0xFF111827),
            onBackground = Color(0xFF17202A),
            onSurface = Color(0xFF17202A),
        ),
        content = content,
    )
}

@Composable
internal fun NostrVpnApp(
    state: AppState,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
) {
    var page by remember { mutableStateOf(Page.Devices) }
    val network = state.activeNetwork
    Scaffold(
        containerColor = Color(0xFFF6F7F8),
        bottomBar = {
            NavigationBar(containerColor = Color.White) {
                Page.entries.forEach { item ->
                    NavigationBarItem(
                        selected = page == item,
                        onClick = { page = item },
                        icon = { Dot(selected = page == item) },
                        label = { Text(item.title) },
                    )
                }
            }
        },
    ) { padding ->
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
            contentPadding = PaddingValues(18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item { Hero(state, network, dispatch) }
            if (state.error.isNotBlank()) {
                item { Notice(state.error) }
            }
            when (page) {
                Page.Devices -> devicesPage(network, dispatch)
                Page.Share -> sharePage(state, network, qrJson, dispatch)
                Page.Routing -> routingPage(state, network, dispatch)
                Page.Settings -> settingsPage(state, network, dispatch)
            }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.devicesPage(
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    if (network == null) {
        item { EmptyCard("No network") }
        return
    }
    items(network.participants, key = { it.pubkeyHex }) { participant ->
        ParticipantRow(participant)
    }
    item { AddParticipantCard(network, dispatch) }
    items(network.inboundJoinRequests, key = { it.requesterNpub }) { request ->
        AppCard {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(request.requesterNodeName.ifBlank { "Join request" }, fontWeight = FontWeight.SemiBold)
                    Text(request.requestedAtText, color = Muted, style = MaterialTheme.typography.bodySmall)
                }
                Button(onClick = {
                    dispatch(
                        JSONObject()
                            .put("type", "accept_join_request")
                            .put("networkId", network.id)
                            .put("requesterNpub", request.requesterNpub),
                    )
                }) {
                    Text("Accept")
                }
            }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.sharePage(
    state: AppState,
    network: NetworkState?,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
) {
    item {
        var inviteInput by remember { mutableStateOf("") }
        AppCard {
            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                QrCode(invite = state.activeNetworkInvite, qrJson = qrJson)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text("Invite Devices", style = MaterialTheme.typography.titleMedium)
                    CopyLine(state.activeNetworkInvite)
                    OutlinedTextField(
                        value = inviteInput,
                        onValueChange = { inviteInput = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        label = { Text("Invite") },
                    )
                    Button(
                        enabled = inviteInput.isNotBlank(),
                        onClick = {
                            dispatch(NativeActions.importInvite(inviteInput.trim()))
                            inviteInput = ""
                        },
                    ) {
                        Text("Import")
                    }
                    if (network?.outboundJoinRequest == true) {
                        Pill("Join requested", Color(0xFFFFF7ED), Color(0xFF9A3412))
                    } else if (!network?.inviteInviterNpub.isNullOrBlank()) {
                        Button(onClick = {
                            dispatch(JSONObject().put("type", "request_network_join").put("networkId", network!!.id))
                        }) {
                            Text("Request Access")
                        }
                    }
                }
            }
        }
    }
    item { NearbyCard(state, dispatch) }
}

private fun androidx.compose.foundation.lazy.LazyListScope.routingPage(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    item {
        AppCard {
            Text("Exit Node", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(10.dp))
            Button(onClick = { dispatch(NativeActions.updateSettings("exitNode" to "")) }) {
                Text(if (state.exitNode.isBlank()) "Direct" else "Use Direct")
            }
            Spacer(Modifier.height(8.dp))
            network?.participants.orEmpty().filter { it.offersExitNode }.forEach { participant ->
                TextButton(onClick = {
                    dispatch(NativeActions.updateSettings("exitNode" to participant.npub))
                }) {
                    Text(participant.magicDnsName.ifBlank { participant.alias }, maxLines = 1)
                }
            }
        }
    }
    item {
        var routes by remember(state.advertisedRoutes) {
            mutableStateOf(state.advertisedRoutes.joinToString(", "))
        }
        AppCard {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = state.advertiseExitNode,
                    onCheckedChange = { enabled ->
                        dispatch(NativeActions.updateSettings("advertiseExitNode" to enabled))
                    },
                )
                Text("Offer exit node")
            }
            OutlinedTextField(
                value = routes,
                onValueChange = { routes = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                label = { Text("Routes") },
            )
            Button(onClick = { dispatch(NativeActions.updateSettings("advertisedRoutes" to routes)) }) {
                Text("Save")
            }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.settingsPage(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
) {
    item { DeviceSettingsCard(state, dispatch) }
    item { NetworksCard(state, network, dispatch) }
    item { RelaysCard(state.relays, dispatch) }
    item { DiagnosticsCard(state) }
}
