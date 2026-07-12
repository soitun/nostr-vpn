package org.nostrvpn.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.NativeActions
import org.nostrvpn.app.core.NetworkState
import org.nostrvpn.app.core.ParticipantState

@Composable
private fun PlusIcon() {
    Canvas(Modifier.size(18.dp)) {
        val strokeWidth = 2.6.dp.toPx()
        val center = size.width / 2f
        drawLine(
            Color.White,
            Offset(center, 2.dp.toPx()),
            Offset(center, size.height - 2.dp.toPx()),
            strokeWidth = strokeWidth,
            cap = StrokeCap.Round,
        )
        drawLine(
            Color.White,
            Offset(2.dp.toPx(), center),
            Offset(size.width - 2.dp.toPx(), center),
            strokeWidth = strokeWidth,
            cap = StrokeCap.Round,
        )
    }
}

@Composable
private fun QrCodeIcon() {
    Canvas(Modifier.size(18.dp)) {
        val color = Color.White
        val cell = size.width / 7f
        val finderStroke = Stroke(width = 1.35.dp.toPx())

        fun finder(x: Int, y: Int) {
            drawRect(
                color,
                topLeft = Offset(x * cell, y * cell),
                size = Size(3 * cell, 3 * cell),
                style = finderStroke,
            )
            drawRect(
                color,
                topLeft = Offset((x + 1) * cell, (y + 1) * cell),
                size = Size(cell, cell),
            )
        }

        fun module(x: Int, y: Int) {
            drawRect(
                color,
                topLeft = Offset(x * cell, y * cell),
                size = Size(cell, cell),
            )
        }

        finder(0, 0)
        finder(4, 0)
        finder(0, 4)
        module(4, 4)
        module(6, 4)
        module(5, 5)
        module(4, 6)
        module(6, 6)
    }
}

internal fun androidx.compose.foundation.lazy.LazyListScope.devicesPage(
    state: AppState,
    network: NetworkState,
    dispatch: (JSONObject) -> Unit,
    onAddDevice: () -> Unit,
    onDeleteNetwork: () -> Unit,
) {
    if (!network.enabled) {
        item {
            Button(
                onClick = { dispatch(NativeActions.setNetworkEnabled(network.id, true)) },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Activate network")
            }
        }
    }
    if (network.localIsAdmin) {
        item {
            Button(
                onClick = onAddDevice,
                enabled = network.enabled,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Add device")
            }
        }
    }
    items(sortedParticipants(network.participants, state), key = { it.pubkeyHex.ifBlank { it.npub } }) { participant ->
        ParticipantRow(state, participant, network = network, dispatch = dispatch)
    }
    item {
        OutlinedButton(
            onClick = onDeleteNetwork,
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = 8.dp),
        ) {
            Text("Delete network", color = Color(0xFFB00020))
        }
    }
}

@Composable
private fun DeviceListHeader(
    state: AppState,
    network: NetworkState,
) {
    Column {
        Text(networkTitle(network), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        Text(deviceCountText(network), color = Muted, style = MaterialTheme.typography.bodySmall)
    }
}

private fun sortedParticipants(participants: List<ParticipantState>, state: AppState): List<ParticipantState> =
    participants.sortedWith(
        compareByDescending<ParticipantState> { it.isCurrentDevice(state) }
            .thenByDescending { it.reachable }
            .thenBy(String.CASE_INSENSITIVE_ORDER) { it.deviceName(state) },
    )

internal fun ParticipantState.isCurrentDevice(state: AppState): Boolean =
    (state.ownNpub.isNotBlank() && npub == state.ownNpub) || meshState == "local"

private fun ParticipantState.deviceName(state: AppState): String {
    if (magicDnsName.isNotBlank()) return magicDnsName
    if (isCurrentDevice(state) && state.selfMagicDnsName.isNotBlank()) return state.selfMagicDnsName
    if (alias.isNotBlank()) return alias
    if (magicDnsAlias.isNotBlank()) return magicDnsAlias
    if (npub.length <= 19) return npub.ifBlank { "Device" }
    return "${npub.take(12)}...${npub.takeLast(6)}"
}

private fun deviceCountText(network: NetworkState): String {
    if (network.expectedCount == 0L) return "This device"
    val word = if (network.expectedCount == 1L) "device" else "devices"
    return "${network.onlineCount} online - ${network.expectedCount} $word"
}

@Composable
private fun NetworkSetupCard(
    state: AppState,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
    onCreated: (() -> Unit)? = null,
    showWelcomeHeader: Boolean = false,
) {
    var setupMode by remember { mutableStateOf<NetworkSetupMode?>(null) }
    var networkName by remember { mutableStateOf("My Network") }
    var inviteInput by remember { mutableStateOf("") }
    var inviteExpanded by remember { mutableStateOf(false) }
    var manualExpanded by remember { mutableStateOf(false) }
    var manualAdminId by remember { mutableStateOf("") }
    var manualNetworkId by remember { mutableStateOf("") }
    val context = androidx.compose.ui.platform.LocalContext.current
    val clipboard = remember(context) {
        context.getSystemService(android.content.ClipboardManager::class.java)
    }
    val joinRequestQrCodeOrLink = state.joinRequestQrCodeOrLink
    fun importInviteIfPresent(value: String): Boolean {
        val trimmed = value.trim()
        if (!trimmed.startsWith("nvpn://invite/", ignoreCase = true)) {
            return false
        }
        dispatch(NativeActions.importInvite(trimmed))
        inviteInput = ""
        return true
    }

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (showWelcomeHeader && setupMode == null) {
            NostrVpnWelcomeHeader()
        }
        if (setupMode == null) {
            Column(verticalArrangement = Arrangement.spacedBy(18.dp)) {
                Button(
                    onClick = { setupMode = NetworkSetupMode.Create },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(58.dp),
                    shape = RoundedCornerShape(16.dp),
                    contentPadding = PaddingValues(horizontal = 20.dp, vertical = 14.dp),
                ) {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        PlusIcon()
                        Text(
                            "Create Network",
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.SemiBold,
                        )
                    }
                }
                Button(
                    onClick = { setupMode = NetworkSetupMode.Join },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(58.dp),
                    shape = RoundedCornerShape(16.dp),
                    contentPadding = PaddingValues(horizontal = 20.dp, vertical = 14.dp),
                ) {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        QrCodeIcon()
                        Text(
                            "Join Network",
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.SemiBold,
                        )
                    }
                }
            }
        } else {
            val mode = setupMode
            if (mode != null) {
                TextButton(onClick = { setupMode = null }) {
                    Text("Back")
                }
                when (mode) {
                    NetworkSetupMode.Create -> SetupChoiceCard("Create Network", Color(0xFF16A34A)) {
                        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            OutlinedTextField(
                                value = networkName,
                                onValueChange = { networkName = it },
                                modifier = Modifier.fillMaxWidth(),
                                singleLine = true,
                                label = { Text("Network name") },
                            )
                            Button(
                                onClick = {
                                    dispatch(NativeActions.addNetwork(networkName.trim().ifBlank { "My Network" }))
                                    networkName = "My Network"
                                    onCreated?.invoke()
                                },
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Text("Create")
                            }
                        }
                    }
                    NetworkSetupMode.Join -> {
                        SetupChoiceCard("Join Network", Color(0xFF2563EB)) {
                            if (joinRequestQrCodeOrLink.isNotBlank()) {
                                BoxWithConstraints(
                                    modifier = Modifier.fillMaxWidth(),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    val qrSide = maxWidth.coerceAtMost(220.dp)
                                    QrCode(
                                        invite = joinRequestQrCodeOrLink,
                                        qrJson = qrJson,
                                        side = qrSide,
                                    )
                                }
                                CopyButton(joinRequestQrCodeOrLink, "Copy request")
                            }

                            TextButton(onClick = { inviteExpanded = !inviteExpanded }) {
                                Text(if (inviteExpanded) "Invite link ▴" else "Invite link ▾")
                            }
                            if (inviteExpanded) {
                                OutlinedTextField(
                                    value = inviteInput,
                                    onValueChange = { newValue ->
                                        inviteInput = newValue
                                        importInviteIfPresent(newValue)
                                    },
                                    modifier = Modifier.fillMaxWidth(),
                                    singleLine = true,
                                    label = { Text("nvpn://invite/…") },
                                )
                                OutlinedButton(
                                    onClick = {
                                        val item = clipboard?.primaryClip?.getItemAt(0)?.coerceToText(context)
                                        item?.toString()?.trim()?.let { pasted ->
                                            if (!importInviteIfPresent(pasted)) {
                                                inviteInput = pasted
                                            }
                                        }
                                    },
                                    modifier = Modifier.fillMaxWidth(),
                                ) {
                                    Text("Paste")
                                }
                            }

                            // Manual join: hand off admin device id + mesh network id directly
                            // to the core's `manual_add_network` action. Both sides add each
                            // other's Device ID out-of-band; no join request is queued here.
                            TextButton(onClick = { manualExpanded = !manualExpanded }) {
                                Text(if (manualExpanded) "Add manually ▴" else "Add manually ▾")
                            }
                            if (manualExpanded) {
                                val adminTrim = manualAdminId.trim()
                                val meshTrim = normalizeNetworkIdInput(manualNetworkId)
                                val adminInvalid = adminTrim.isNotEmpty() && !isValidDeviceId(adminTrim)
                                val canSubmit = adminTrim.isNotEmpty() && meshTrim.isNotEmpty() && !adminInvalid
                                Text(
                                    "Give the admin your Device ID, then enter their Device ID and network ID.",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = Muted,
                                )
                                Text("Your Device ID", style = MaterialTheme.typography.bodySmall, color = Muted)
                                CopyButton(state.ownNpub, "Copy Device ID")
                                OutlinedTextField(
                                    value = manualAdminId,
                                    onValueChange = { manualAdminId = it },
                                    modifier = Modifier.fillMaxWidth(),
                                    singleLine = true,
                                    label = { Text("Admin Device ID") },
                                    isError = adminInvalid,
                                    supportingText = if (adminInvalid) {
                                        { Text("Not a valid device ID") }
                                    } else {
                                        null
                                    },
                                )
                                OutlinedTextField(
                                    value = manualNetworkId,
                                    onValueChange = { manualNetworkId = it },
                                    modifier = Modifier.fillMaxWidth(),
                                    singleLine = true,
                                    label = { Text("Network ID") },
                                )
                                Button(
                                    enabled = canSubmit,
                                    onClick = {
                                        dispatch(NativeActions.manualAddNetwork(adminTrim, meshTrim))
                                        manualAdminId = ""
                                        manualNetworkId = ""
                                        manualExpanded = false
                                    },
                                    modifier = Modifier.fillMaxWidth(),
                                ) {
                                    Text("Add")
                                }
                            }
                        }
                        AdvertiseJoinRequestCard(state, dispatch)
                    }
                }
            }
        }
    }
}

@Composable
private fun NostrVpnWelcomeHeader() {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 26.dp, bottom = 10.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Image(
            painter = painterResource(R.drawable.ic_launcher_foreground),
            contentDescription = null,
            modifier = Modifier.size(82.dp),
        )
        Text(
            "Nostr VPN",
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun SetupChoiceCard(
    title: String,
    accent: Color,
    content: @Composable ColumnScope.() -> Unit,
) {
    AppCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Canvas(Modifier.size(10.dp)) {
                drawCircle(accent)
            }
            Spacer(Modifier.width(8.dp))
            Text(title, style = MaterialTheme.typography.titleMedium, color = accent)
        }
        content()
    }
}

// LazyListScope wrapper for the Add Network body, used as the entire
// screen content when there is no active network. Mirrors the in-dialog
// content we show when the user picks "Add network" from the header
// switcher with an existing network already in place.
internal fun androidx.compose.foundation.lazy.LazyListScope.addNetworkBody(
    state: AppState,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
    showWelcomeHeader: Boolean = false,
) {
    item { NetworkSetupCard(state, qrJson, dispatch, showWelcomeHeader = showWelcomeHeader) }
}

@Composable
internal fun AddNetworkDialog(
    state: AppState,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
    onDismiss: () -> Unit,
    onCreated: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Network") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (state.error.isNotBlank()) {
                    Notice(state.error)
                }
                NetworkSetupCard(state, qrJson, dispatch, onCreated = onCreated)
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
}

/// Admin-only sheet for adding a device to YOUR network. The admin scans or
/// pastes the joiner's request/Device ID; joining someone else's network and
/// finding nearby networks belong to Add Network, not here.
@Composable
internal fun AddDevicesDialog(
    state: AppState,
    network: NetworkState,
    scanDeviceQr: (String) -> Unit,
    dispatch: (JSONObject) -> Unit,
    onDismiss: () -> Unit,
) {
    var joinRequestInput by remember(network.id) { mutableStateOf("") }
    var pendingJoinRequest by remember(network.id) { mutableStateOf<String?>(null) }
    fun stageJoinRequest(value: String) {
        val trimmed = value.trim()
        if (looksLikeJoinRequestQrOrLink(trimmed)) {
            pendingJoinRequest = trimmed
        }
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Device") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Add join request", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Scan or paste the joiner's join request. Valid links open confirmation automatically.",
                    style = MaterialTheme.typography.bodySmall,
                    color = Muted,
                )
                OutlinedTextField(
                    value = joinRequestInput,
                    onValueChange = {
                        joinRequestInput = it
                        stageJoinRequest(it)
                    },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Join request") },
                )
                Button(
                    onClick = { scanDeviceQr(network.id) },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Scan QR")
                }
                NearbyCard(state, dispatch)
                Spacer(modifier = Modifier.height(8.dp))
                Text("For manual join", style = MaterialTheme.typography.titleMedium)
                Text(
                    "If join-request linking isn't available, share these two values. They'll enter them under Join Network -> Add manually. You still need to add their Device ID below.",
                    style = MaterialTheme.typography.bodySmall,
                    color = Muted,
                )
                Text("Your Device ID", style = MaterialTheme.typography.bodySmall, color = Muted)
                CopyLine(state.ownNpub)
                Text("Network ID", style = MaterialTheme.typography.bodySmall, color = Muted)
                CopyLine(network.networkId, displayNetworkId(network.networkId))

                Spacer(modifier = Modifier.height(8.dp))
                Text("Add by Device ID", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Manual pairing: enter the joiner's Device ID.",
                    style = MaterialTheme.typography.bodySmall,
                    color = Muted,
                )
                AddParticipantForm(network, dispatch)
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text("Done")
            }
        },
    )
    pendingJoinRequest?.let { request ->
        AlertDialog(
            onDismissRequest = { pendingJoinRequest = null },
            title = { Text("Add device?") },
            text = { Text("Add the device from this join request to ${network.name.ifBlank { "this network" }}?") },
            confirmButton = {
                Button(
                    onClick = {
                        dispatch(NativeActions.importJoinRequest(request))
                        joinRequestInput = ""
                        pendingJoinRequest = null
                        onDismiss()
                    },
                ) {
                    Text("Add")
                }
            },
            dismissButton = {
                TextButton(onClick = { pendingJoinRequest = null }) {
                    Text("Cancel")
                }
            },
        )
    }
}

private fun displayNetworkId(value: String): String {
    val trimmed = value.trim()
    if (trimmed.length <= 4 || !trimmed.all { it.isHexDigit() }) {
        return trimmed
    }
    return trimmed.chunked(4).joinToString("-")
}

private fun normalizeNetworkIdInput(value: String): String {
    val trimmed = value.trim()
    val compact = trimmed.filter { !it.isWhitespace() && it != '-' }
    if (compact.isEmpty() && trimmed.all { it.isWhitespace() || it == '-' }) {
        return ""
    }
    return if (compact.isNotEmpty() && compact.all { it.isHexDigit() }) compact.lowercase() else trimmed
}

private fun Char.isHexDigit(): Boolean =
    this in '0'..'9' || lowercaseChar() in 'a'..'f'
