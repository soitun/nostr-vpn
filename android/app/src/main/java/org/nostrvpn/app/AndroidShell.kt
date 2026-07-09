package org.nostrvpn.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin
import org.json.JSONObject
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.NativeActions
import org.nostrvpn.app.core.NetworkState
import org.nostrvpn.app.core.PaidRouteMarketState
import org.nostrvpn.app.core.PaidRouteOfferState
import org.nostrvpn.app.core.PaidRouteSessionState
import org.nostrvpn.app.core.ParticipantState
import org.nostrvpn.app.core.activeNetwork
import org.nostrvpn.app.update.AndroidSelfUpdateState

internal data class SelfUpdateActions(
    val check: () -> Unit,
    val download: () -> Unit,
    val install: () -> Unit,
    val setAutoCheck: (Boolean) -> Unit,
)

private enum class Page(val title: String) {
    Devices("Devices"),
    Internet("Internet"),
    PublicExits("Buy Internet"),
    Wallet("Wallet"),
    Settings("Settings"),
}

private enum class NetworkSetupMode {
    Create,
    Join,
}

private object PaidInternetFeature {
    val enabled: Boolean
        get() {
            if (!BuildConfig.DEBUG) {
                return false
            }
            return enabledFlag(System.getProperty("nvpn.enablePaidInternet")) ||
                enabledFlag(System.getenv("NVPN_ENABLE_PAID_INTERNET"))
        }

    private fun enabledFlag(value: String?): Boolean =
        when (value?.trim()?.lowercase()) {
            "1", "true", "yes", "on" -> true
            else -> false
        }
}

private fun Page.visibleIn(state: AppState): Boolean =
    when (this) {
        Page.PublicExits, Page.Wallet -> PaidInternetFeature.enabled && state.paidRouteMarket.supported
        Page.Devices, Page.Internet, Page.Settings -> true
    }

@Composable
internal fun NostrVpnTheme(content: @Composable () -> Unit) {
    val darkTheme = isSystemInDarkTheme()
    MaterialTheme(
        colorScheme = if (darkTheme) {
            darkColorScheme(
                primary = Color(0xFFA78BFA),
                secondary = Color(0xFF67E8F9),
                background = Color(0xFF101419),
                surface = Color(0xFF171D24),
                onPrimary = Color(0xFF1E1235),
                onSecondary = Color(0xFF06161A),
                onBackground = Color(0xFFE7ECF2),
                onSurface = Color(0xFFE7ECF2),
                surfaceVariant = Color(0xFF202833),
                onSurfaceVariant = Color(0xFFAAB4C0),
                error = Color(0xFFFCA5A5),
                outline = Color(0xFF5F6B7A),
            )
        } else {
            lightColorScheme(
                primary = Color(0xFF8B5CF6),
                secondary = Color(0xFF22D3EE),
                background = Color(0xFFF6F7F8),
                surface = Color.White,
                onPrimary = Color.White,
                onSecondary = Color(0xFF111827),
                onBackground = Color(0xFF17202A),
                onSurface = Color(0xFF17202A),
                surfaceVariant = Color(0xFFF1F5F9),
                onSurfaceVariant = Color(0xFF68717C),
                error = Color(0xFFB00020),
                outline = Color(0xFF9CA3AF),
            )
        },
        content = content,
    )
}

@Composable
internal fun NostrVpnApp(
    state: AppState,
    qrJson: (String) -> JSONObject,
    scanDeviceQr: (String) -> Unit,
    dispatch: (JSONObject) -> Unit,
    selfUpdateState: AndroidSelfUpdateState,
    selfUpdateActions: SelfUpdateActions,
    importWireGuardConfigFile: () -> Unit,
) {
    var page by remember { mutableStateOf(Page.Devices) }
    var showAddDevice by remember { mutableStateOf(false) }
    var showAddNetwork by remember { mutableStateOf(false) }
    var pendingNetworkRemoval by remember { mutableStateOf<NetworkState?>(null) }
    var shownNetworkId by remember { mutableStateOf<String?>(null) }
    val activeNetwork = state.activeNetwork
    val network = state.networks.firstOrNull { it.id == shownNetworkId }
        ?: activeNetwork
        ?: state.networks.firstOrNull()
    val visiblePages = Page.entries.filter { it.visibleIn(state) }
    val effectivePage = if (page.visibleIn(state)) page else Page.Devices
    LaunchedEffect(showAddDevice, network?.enabled) {
        if (showAddDevice && network?.enabled != true) {
            showAddDevice = false
        }
    }
    LaunchedEffect(state.paidRouteMarket.supported) {
        if (!page.visibleIn(state)) {
            page = Page.Devices
        }
    }
    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            if (network != null) {
                MobileTopBar(
                    state = state,
                    network = network,
                    activeNetwork = activeNetwork,
                    dispatch = dispatch,
                    onSelectNetwork = { shownNetworkId = it },
                    onAddNetwork = { showAddNetwork = true },
                )
            }
        },
        bottomBar = {
            // Bottom nav only makes sense once a network exists. With no
            // network the only meaningful action is Add Network, which we
            // surface as the entire screen body.
            if (network != null) {
                NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
                    visiblePages.forEach { item ->
                        NavigationBarItem(
                            selected = effectivePage == item,
                            onClick = { page = item },
                            icon = {
                                NavIcon(
                                    item,
                                    selected = effectivePage == item,
                                    attention = false,
                                )
                            },
                            label = { Text(item.title) },
                        )
                    }
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
            if (state.error.isNotBlank()) {
                item { Notice(state.error) }
            }
            if (network == null) {
                addNetworkBody(state, qrJson, dispatch, showWelcomeHeader = true)
            } else {
                when (effectivePage) {
                    Page.Devices -> devicesPage(
                        state,
                        network,
                        dispatch,
                        onAddDevice = { showAddDevice = true },
                        onDeleteNetwork = { pendingNetworkRemoval = network },
                    )
                    Page.Internet -> internetPage(state, network, dispatch, importWireGuardConfigFile)
                    Page.PublicExits -> publicExitsPage(state, dispatch)
                    Page.Wallet -> walletPage(state, dispatch)
                    Page.Settings -> settingsPage(state, network, dispatch, selfUpdateState, selfUpdateActions)
                }
            }
        }
    }
    if (showAddDevice && network?.enabled == true) {
        AddDevicesDialog(
            state = state,
            network = network,
            scanDeviceQr = scanDeviceQr,
            dispatch = dispatch,
            onDismiss = { showAddDevice = false },
        )
    }
    if (showAddNetwork) {
        AddNetworkDialog(
            state = state,
            qrJson = qrJson,
            dispatch = dispatch,
            onDismiss = { showAddNetwork = false },
            onCreated = {
                // Land on the new network's Devices view: dismiss the
                // dialog and reset the nav to Devices in case the user
                // was on another page when they tapped Add
                // network.
                showAddNetwork = false
                page = Page.Devices
            },
        )
    }
    pendingNetworkRemoval?.let { target ->
        AlertDialog(
            onDismissRequest = { pendingNetworkRemoval = null },
            title = { Text("Delete ${target.name.ifBlank { "network" }}?") },
            text = { Text("Removes the network from this device.") },
            confirmButton = {
                TextButton(onClick = {
                    dispatch(NativeActions.removeNetwork(target.id))
                    pendingNetworkRemoval = null
                }) {
                    Text("Delete", color = Color(0xFFB00020))
                }
            },
            dismissButton = {
                TextButton(onClick = { pendingNetworkRemoval = null }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun MobileTopBar(
    state: AppState,
    network: NetworkState?,
    activeNetwork: NetworkState?,
    dispatch: (JSONObject) -> Unit,
    onSelectNetwork: (String) -> Unit,
    onAddNetwork: () -> Unit,
) {
    var menuExpanded by remember { mutableStateOf(false) }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface)
            .statusBarsPadding()
            .padding(horizontal = 18.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.weight(1f)) {
            // Single dropdown: switch network OR add a new one. Whole row
            // tappable so it reads as a "current network: name" affordance.
            Row(
                modifier = Modifier.clickable { menuExpanded = true },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (state.networks.size > 1) {
                    NetworkStatusDot(network)
                    Spacer(Modifier.width(8.dp))
                }
                Text(
                    networkTitle(network),
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Spacer(Modifier.width(6.dp))
                Text("▾", color = Muted)
            }
            DropdownMenu(expanded = menuExpanded, onDismissRequest = { menuExpanded = false }) {
                state.networks.forEach { saved ->
                    DropdownMenuItem(
                        text = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                if (state.networks.size > 1) {
                                    NetworkStatusDot(saved)
                                    Spacer(Modifier.width(8.dp))
                                }
                                Text(networkTitle(saved))
                            }
                        },
                        onClick = {
                            menuExpanded = false
                            onSelectNetwork(saved.id)
                        },
                    )
                }
                if (state.networks.isNotEmpty()) {
                    HorizontalDivider()
                }
                DropdownMenuItem(
                    text = { Text("Add network") },
                    onClick = {
                        menuExpanded = false
                        onAddNetwork()
                    },
                )
            }
        }
        Switch(
            checked = state.vpnEnabled,
            enabled = state.vpnControlSupported && activeNetwork != null,
            onCheckedChange = { enabled ->
                dispatch(
                    if (enabled) {
                        NativeActions.connectVpn()
                    } else {
                        NativeActions.disconnectVpn()
                    },
                )
            },
        )
    }
}

@Composable
private fun NetworkStatusDot(network: NetworkState?) {
    Canvas(modifier = Modifier.size(8.dp)) {
        drawCircle(if (network?.enabled == true) Color(0xFF16A34A) else Color(0xFF9CA3AF))
    }
}

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

@Composable
private fun NavIcon(page: Page, selected: Boolean, attention: Boolean = false) {
    val color = if (selected) Accent else MaterialTheme.colorScheme.onSurface
    val badgeRing = MaterialTheme.colorScheme.surface
    Canvas(modifier = Modifier.size(28.dp)) {
        val strokeWidth = 2.6.dp.toPx()
        val stroke = Stroke(width = strokeWidth, cap = StrokeCap.Round)
        when (page) {
            Page.Devices -> {
                val radius = 3.6.dp.toPx()
                val gap = 5.4.dp.toPx()
                val center = Offset(size.width / 2f, size.height / 2f)
                for (x in listOf(-gap, gap)) {
                    for (y in listOf(-gap, gap)) {
                        drawCircle(color, radius, Offset(center.x + x, center.y + y))
                    }
                }
            }
            Page.Internet -> {
                val center = Offset(size.width / 2f, size.height / 2f)
                drawCircle(color, 10.5.dp.toPx(), center, style = stroke)
                drawLine(
                    color,
                    Offset(center.x - 10.5.dp.toPx(), center.y),
                    Offset(center.x + 10.5.dp.toPx(), center.y),
                    strokeWidth = strokeWidth,
                    cap = StrokeCap.Round,
                )
                drawLine(
                    color,
                    Offset(center.x, center.y - 10.5.dp.toPx()),
                    Offset(center.x, center.y + 10.5.dp.toPx()),
                    strokeWidth = strokeWidth,
                    cap = StrokeCap.Round,
                )
            }
            Page.PublicExits -> {
                val baseY = 18.5.dp.toPx()
                drawLine(color, Offset(6.dp.toPx(), 8.dp.toPx()), Offset(8.5.dp.toPx(), baseY), strokeWidth = strokeWidth, cap = StrokeCap.Round)
                drawLine(color, Offset(8.5.dp.toPx(), baseY), Offset(21.dp.toPx(), baseY), strokeWidth = strokeWidth, cap = StrokeCap.Round)
                drawLine(color, Offset(9.5.dp.toPx(), 11.dp.toPx()), Offset(20.dp.toPx(), 11.dp.toPx()), strokeWidth = strokeWidth, cap = StrokeCap.Round)
                drawCircle(color, 2.4.dp.toPx(), Offset(10.dp.toPx(), 22.5.dp.toPx()))
                drawCircle(color, 2.4.dp.toPx(), Offset(20.dp.toPx(), 22.5.dp.toPx()))
            }
            Page.Wallet -> {
                val left = 5.dp.toPx()
                val top = 8.dp.toPx()
                val right = 23.dp.toPx()
                val bottom = 21.dp.toPx()
                drawRoundRect(
                    color,
                    topLeft = Offset(left, top),
                    size = androidx.compose.ui.geometry.Size(right - left, bottom - top),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(2.5.dp.toPx()),
                    style = stroke,
                )
                drawLine(color, Offset(left + 2.dp.toPx(), 13.dp.toPx()), Offset(right - 2.dp.toPx(), 13.dp.toPx()), strokeWidth = strokeWidth, cap = StrokeCap.Round)
                drawCircle(color, 1.8.dp.toPx(), Offset(19.dp.toPx(), 17.dp.toPx()))
            }
            Page.Settings -> {
                val center = Offset(size.width / 2f, size.height / 2f)
                val inner = 8.6.dp.toPx()
                val outer = 12.1.dp.toPx()
                repeat(8) { index ->
                    val angle = index * PI.toFloat() / 4f
                    val start = Offset(center.x + cos(angle) * inner, center.y + sin(angle) * inner)
                    val end = Offset(center.x + cos(angle) * outer, center.y + sin(angle) * outer)
                    drawLine(color, start, end, strokeWidth = strokeWidth, cap = StrokeCap.Round)
                }
                drawCircle(color, 6.7.dp.toPx(), center, style = stroke)
                drawCircle(color, 2.4.dp.toPx(), center)
            }
        }
        if (attention) {
            val center = Offset(size.width - 4.dp.toPx(), 4.dp.toPx())
            drawCircle(badgeRing, 5.dp.toPx(), center)
            drawCircle(Color(0xFFDC2626), 4.dp.toPx(), center)
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.devicesPage(
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
        compareByDescending<ParticipantState> { it.isSelf(state) }
            .thenByDescending { it.reachable }
            .thenBy(String.CASE_INSENSITIVE_ORDER) { it.deviceName(state) },
    )

private fun ParticipantState.isSelf(state: AppState): Boolean =
    (state.ownNpub.isNotBlank() && npub == state.ownNpub) || meshState == "local"

private fun ParticipantState.deviceName(state: AppState): String {
    if (magicDnsName.isNotBlank()) return magicDnsName
    if (isSelf(state) && state.selfMagicDnsName.isNotBlank()) return state.selfMagicDnsName
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
private fun androidx.compose.foundation.lazy.LazyListScope.addNetworkBody(
    state: AppState,
    qrJson: (String) -> JSONObject,
    dispatch: (JSONObject) -> Unit,
    showWelcomeHeader: Boolean = false,
) {
    item { NetworkSetupCard(state, qrJson, dispatch, showWelcomeHeader = showWelcomeHeader) }
}

@Composable
private fun AddNetworkDialog(
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
private fun AddDevicesDialog(
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
    fun importJoinerValue(value: String) {
        val trimmed = value.trim()
        if (trimmed.isEmpty()) return
        if (looksLikeJoinRequestQrOrLink(trimmed)) {
            stageJoinRequest(trimmed)
            return
        }
        val scanned = parseScannedDeviceLinkQr(trimmed)
        if (scanned != null) {
            dispatch(NativeActions.addParticipant(network.id, scanned.deviceId, scanned.alias))
            return
        }
        dispatch(NativeActions.importJoinRequest(trimmed))
    }
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
                    "Scan or paste the joiner's join request or Device ID.",
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
                    label = { Text("Join request or Device ID") },
                )
                Button(
                    enabled = joinRequestInput.trim().isNotEmpty(),
                    onClick = {
                        importJoinerValue(joinRequestInput)
                        joinRequestInput = ""
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text("Import request")
                }
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
}

private fun androidx.compose.foundation.lazy.LazyListScope.internetPage(
    state: AppState,
    network: NetworkState?,
    dispatch: (JSONObject) -> Unit,
    importWireGuardConfigFile: () -> Unit,
) {
    item {
        AppCard {
            Text("Internet", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(10.dp))

            // The daemon clears the *other* side automatically when
            // both would otherwise be set (see
            // `settings_patch_enforces_exit_node_mutual_exclusion`),
            // so the WG and peer rows only push the field they own.
            // Using this device's normal internet still needs to flip
            // both explicitly since neither is a conflict the daemon resolves.
            val directSelected = !state.wireguardExitEnabled && state.exitNode.isBlank()
            ExitNodeRow(
                title = "This device",
                subtitle = "Use this device's normal internet",
                selected = directSelected,
                enabled = true,
                onClick = {
                    dispatch(
                        NativeActions.updateSettings(
                            "exitNode" to "",
                            "wireguardExitEnabled" to false,
                        ),
                    )
                },
            )

            val wgSubtitle =
                if (!state.wireguardExitConfigured) {
                    "No WireGuard config saved yet"
                } else if (state.wireguardExitEndpoint.isBlank()) {
                    "Configured"
                } else {
                    state.wireguardExitEndpoint
                }
            ExitNodeRow(
                title = "WireGuard upstream",
                subtitle = wgSubtitle,
                selected = state.wireguardExitEnabled,
                enabled = state.wireguardExitConfigured,
                onClick = {
                    dispatch(NativeActions.updateSettings("wireguardExitEnabled" to true))
                },
            )

            val exitParticipants = network?.participants.orEmpty()
                .filter { it.offersExitNode && !it.isSelf(state) }
            if (exitParticipants.isEmpty()) {
                Text("No trusted devices sharing internet", color = Muted, style = MaterialTheme.typography.bodySmall)
            } else {
                exitParticipants.forEach { participant ->
                    ExitNodeRow(
                        title = participant.magicDnsName.ifBlank { participant.alias },
                        subtitle = participant.npub,
                        selected = !state.wireguardExitEnabled && state.exitNode == participant.npub,
                        enabled = true,
                        onClick = {
                            dispatch(NativeActions.updateSettings("exitNode" to participant.npub))
                        },
                    )
                }
            }
        }
    }
    item {
        AppCard {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(
                    checked = state.advertiseExitNode,
                    onCheckedChange = { enabled ->
                        dispatch(NativeActions.updateSettings("advertiseExitNode" to enabled))
                    },
                )
                val name = network?.name?.ifBlank { null } ?: "this network"
                Text("Share internet with $name")
            }
        }
    }
    if (PaidInternetFeature.enabled && state.paidExitSeller.supported) {
        item { PaidExitSellerStatusCard(state) }
    }
    item { WireGuardSettingsCard(state, dispatch, importWireGuardConfigFile) }
}

private fun androidx.compose.foundation.lazy.LazyListScope.publicExitsPage(
    state: AppState,
    dispatch: (JSONObject) -> Unit,
) {
    item { PaidRouteMarketCard(state, dispatch, PaidRouteCardMode.Market) }
}

private fun androidx.compose.foundation.lazy.LazyListScope.walletPage(
    state: AppState,
    dispatch: (JSONObject) -> Unit,
) {
    item { PaidRouteMarketCard(state, dispatch, PaidRouteCardMode.Wallet) }
}

private enum class PaidRouteCardMode {
    Market,
    Wallet,
}

@Composable
private fun PaidRouteMarketCard(
    state: AppState,
    dispatch: (JSONObject) -> Unit,
    mode: PaidRouteCardMode,
) {
    val market = state.paidRouteMarket
    var mintUrl by remember { mutableStateOf(market.wallet.defaultMint) }
    var token by remember { mutableStateOf("") }
    var topUpAmount by remember { mutableStateOf("") }
    var sendAmount by remember { mutableStateOf("") }
    var withdrawInvoice by remember { mutableStateOf("") }
    var filterCountry by remember { mutableStateOf(market.filter.countryCode) }
    var filterNetwork by remember { mutableStateOf(market.filter.networkClass) }
    var filterIpv4 by remember { mutableStateOf(market.filter.requireIpv4) }
    var filterIpv6 by remember { mutableStateOf(market.filter.requireIpv6) }
    var filterSort by remember { mutableStateOf(market.filter.sort.ifBlank { "quality" }) }
    LaunchedEffect(
        market.filter.countryCode,
        market.filter.networkClass,
        market.filter.requireIpv4,
        market.filter.requireIpv6,
        market.filter.sort,
    ) {
        filterCountry = market.filter.countryCode
        filterNetwork = market.filter.networkClass
        filterIpv4 = market.filter.requireIpv4
        filterIpv6 = market.filter.requireIpv6
        filterSort = market.filter.sort.ifBlank { "quality" }
    }
    AppCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    if (mode == PaidRouteCardMode.Wallet) "Cashu Wallet" else "Buy Internet",
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    "Wallet ${market.wallet.totalBalanceText.ifBlank { formatPaidRouteMsat(market.wallet.totalBalanceMsat) }}",
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            if (mode == PaidRouteCardMode.Market) {
                Button(
                    enabled = market.supported,
                    onClick = { dispatch(NativeActions.discoverPaidRouteOffers()) },
                ) {
                    Text("Find")
                }
            }
        }
        if (mode == PaidRouteCardMode.Market && market.statusText.isNotBlank()) {
            Text(market.statusText, color = Muted, style = MaterialTheme.typography.bodySmall)
        }
        if (!market.supported) {
            Text(
                if (mode == PaidRouteCardMode.Wallet) {
                    "Cashu wallet is not supported on this platform"
                } else {
                    "Buying internet is not supported on this platform"
                },
                color = Muted,
            )
            return@AppCard
        }

        when (mode) {
            PaidRouteCardMode.Market -> {
                PaidRouteMarketFilterControls(
                    market = market,
                    country = filterCountry,
                    onCountryChange = { filterCountry = it },
                    networkClass = filterNetwork,
                    onNetworkClassChange = { filterNetwork = it },
                    requireIpv4 = filterIpv4,
                    onRequireIpv4Change = { filterIpv4 = it },
                    requireIpv6 = filterIpv6,
                    onRequireIpv6Change = { filterIpv6 = it },
                    sort = filterSort,
                    onSortChange = { sort ->
                        filterSort = sort
                        dispatchPaidRouteMarketFilter(
                            dispatch,
                            filterCountry,
                            filterNetwork,
                            filterIpv4,
                            filterIpv6,
                            sort,
                        )
                    },
                    onApply = {
                        dispatchPaidRouteMarketFilter(
                            dispatch,
                            filterCountry,
                            filterNetwork,
                            filterIpv4,
                            filterIpv6,
                            filterSort,
                        )
                    },
                    onClear = {
                        filterCountry = ""
                        filterNetwork = ""
                        filterIpv4 = false
                        filterIpv6 = false
                        filterSort = "quality"
                        dispatchPaidRouteMarketFilter(dispatch, "", "", false, false, "quality")
                    },
                )

                PaidRoutePaymentActionResult(market.lastPaymentAction, dispatch)

                HorizontalDivider()
                Text("Offers", style = MaterialTheme.typography.titleSmall)
                val visibleOffers = if (market.hiddenOfferCount > 0 || market.visibleOffers.isNotEmpty()) {
                    market.visibleOffers
                } else {
                    market.offers
                }
                if (market.offers.isEmpty()) {
                    Text("No internet sellers found", color = Muted, style = MaterialTheme.typography.bodySmall)
                } else if (visibleOffers.isEmpty()) {
                    Text("No matching sellers", color = Muted, style = MaterialTheme.typography.bodySmall)
                } else {
                    if (market.hiddenOfferCount > 0) {
                        Text(
                            "${market.hiddenOfferCount} hidden by filters",
                            color = Muted,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    visibleOffers.take(6).forEach { offer ->
                        PaidRouteOfferRow(offer, dispatch)
                    }
                }

                HorizontalDivider()
                Text("Your Paid Internet", style = MaterialTheme.typography.titleSmall)
                if (market.sessions.isEmpty()) {
                    Text("No seller selected", color = Muted, style = MaterialTheme.typography.bodySmall)
                } else {
                    market.sessions.forEach { session ->
                        PaidRouteSessionRow(session, market.lastPaymentAction.envelopeJson, dispatch)
                    }
                }
            }
            PaidRouteCardMode.Wallet -> {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = mintUrl,
                        onValueChange = { mintUrl = it },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        label = { Text("Mint URL") },
                    )
                    Button(
                        enabled = mintUrl.trim().isNotEmpty(),
                        onClick = {
                            dispatch(NativeActions.addPaidRouteWalletMint(mintUrl.trim(), null))
                        },
                    ) {
                        Text("Add")
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = topUpAmount,
                        onValueChange = { topUpAmount = it },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        label = { Text("Top-up sats") },
                    )
                    Button(
                        enabled = parsePositivePaidRouteAmount(topUpAmount) != null,
                        onClick = {
                            val amount = parsePositivePaidRouteAmount(topUpAmount) ?: return@Button
                            dispatch(NativeActions.topUpPaidRouteWallet(optionalPaidRouteMintUrl(mintUrl), amount))
                        },
                    ) {
                        Text("Top Up")
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = sendAmount,
                        onValueChange = { sendAmount = it },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        label = { Text("Send sats") },
                    )
                    Button(
                        enabled = parsePositivePaidRouteAmount(sendAmount) != null,
                        onClick = {
                            val amount = parsePositivePaidRouteAmount(sendAmount) ?: return@Button
                            dispatch(NativeActions.sendPaidRouteWalletToken(optionalPaidRouteMintUrl(mintUrl), amount))
                        },
                    ) {
                        Text("Export")
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = token,
                        onValueChange = { token = it },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        label = { Text("Cashu token") },
                    )
                    Button(
                        enabled = token.trim().isNotEmpty(),
                        onClick = {
                            dispatch(NativeActions.receivePaidRouteWalletToken(token.trim()))
                            token = ""
                        },
                    ) {
                        Text("Import")
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedTextField(
                        value = withdrawInvoice,
                        onValueChange = { withdrawInvoice = it },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        label = { Text("Lightning invoice") },
                    )
                    Button(
                        enabled = withdrawInvoice.trim().isNotEmpty(),
                        onClick = {
                            val invoice = withdrawInvoice.trim()
                            dispatch(NativeActions.withdrawPaidRouteWalletLightning(optionalPaidRouteMintUrl(mintUrl), invoice))
                            withdrawInvoice = ""
                        },
                    ) {
                        Text("Withdraw")
                    }
                }
                OutlinedButton(onClick = { dispatch(NativeActions.refreshPaidRouteWallet()) }) {
                    Text("Refresh wallet")
                }
                HorizontalDivider()
                Text("Mints", style = MaterialTheme.typography.titleSmall)
                if (market.wallet.mints.isEmpty()) {
                    Text("No wallet mints", color = Muted, style = MaterialTheme.typography.bodySmall)
                } else {
                    market.wallet.mints.forEach { mint ->
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Column(Modifier.weight(1f)) {
                                Text(mint.label.ifBlank { mint.url }, fontWeight = FontWeight.SemiBold)
                                Text(
                                    mint.balanceText.ifBlank { formatPaidRouteMsat(mint.balanceMsat) },
                                    color = Muted,
                                    style = MaterialTheme.typography.bodySmall,
                                )
                            }
                            if (mint.isDefault || mint.url == market.wallet.defaultMint) {
                                Text("Default", color = Accent, style = MaterialTheme.typography.bodySmall)
                            } else {
                                OutlinedButton(onClick = {
                                    dispatch(NativeActions.setPaidRouteDefaultMint(mint.url))
                                }) {
                                    Text("Default")
                                }
                            }
                            Spacer(Modifier.width(6.dp))
                            OutlinedButton(onClick = {
                                dispatch(NativeActions.removePaidRouteWalletMint(mint.url))
                            }) {
                                Text("Remove")
                            }
                        }
                    }
                }
                PaidRouteWalletActionResult(market.wallet.lastAction)
            }
        }
    }
}

@Composable
private fun PaidRouteMarketFilterControls(
    market: PaidRouteMarketState,
    country: String,
    onCountryChange: (String) -> Unit,
    networkClass: String,
    onNetworkClassChange: (String) -> Unit,
    requireIpv4: Boolean,
    onRequireIpv4Change: (Boolean) -> Unit,
    requireIpv6: Boolean,
    onRequireIpv6Change: (Boolean) -> Unit,
    sort: String,
    onSortChange: (String) -> Unit,
    onApply: () -> Unit,
    onClear: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(
                value = country,
                onValueChange = onCountryChange,
                modifier = Modifier.weight(1f),
                singleLine = true,
                label = { Text("Country") },
            )
            OutlinedTextField(
                value = networkClass,
                onValueChange = onNetworkClassChange,
                modifier = Modifier.weight(1f),
                singleLine = true,
                label = { Text("Class") },
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            PaidRouteSortButton("Quality", "quality", sort, onSortChange)
            PaidRouteSortButton("Price", "price", sort, onSortChange)
            PaidRouteSortButton("Newest", "newest", sort, onSortChange)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(checked = requireIpv4, onCheckedChange = onRequireIpv4Change)
                Text("IPv4", style = MaterialTheme.typography.bodySmall)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(checked = requireIpv6, onCheckedChange = onRequireIpv6Change)
                Text("IPv6", style = MaterialTheme.typography.bodySmall)
            }
            Spacer(Modifier.weight(1f))
            OutlinedButton(onClick = onClear, enabled = market.offers.isNotEmpty()) {
                Text("Clear")
            }
            Button(onClick = onApply, enabled = market.offers.isNotEmpty()) {
                Text("Apply")
            }
        }
    }
}

@Composable
private fun PaidRouteSortButton(
    label: String,
    value: String,
    selected: String,
    onSelect: (String) -> Unit,
) {
    if (selected.ifBlank { "quality" } == value) {
        Button(onClick = { onSelect(value) }) {
            Text(label)
        }
    } else {
        OutlinedButton(onClick = { onSelect(value) }) {
            Text(label)
        }
    }
}

private fun dispatchPaidRouteMarketFilter(
    dispatch: (JSONObject) -> Unit,
    country: String,
    networkClass: String,
    requireIpv4: Boolean,
    requireIpv6: Boolean,
    sort: String,
) {
    dispatch(
        NativeActions.setPaidRouteMarketFilter(
            countryCode = country.trim(),
            networkClass = networkClass.trim(),
            requireIpv4 = requireIpv4,
            requireIpv6 = requireIpv6,
            sort = sort.ifBlank { "quality" },
        ),
    )
}

@Composable
private fun PaidRouteWalletActionResult(action: org.nostrvpn.app.core.PaidRouteWalletActionState) {
    if (action.kind.isBlank() && action.statusText.isBlank()) return
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(action.statusText.ifBlank { paidRouteWalletActionTitle(action.kind) }, color = Muted, style = MaterialTheme.typography.bodySmall)
        if (action.paymentRequest.isNotBlank()) {
            CopyLine(action.paymentRequest, "Lightning invoice ready")
        }
        if (action.token.isNotBlank()) {
            CopyLine(action.token, "Cashu token ready")
        }
        if (action.preimage.isNotBlank()) {
            CopyLine(action.preimage, "Lightning preimage ready")
        }
    }
}

@Composable
private fun PaidRoutePaymentActionResult(
    action: org.nostrvpn.app.core.PaidRoutePaymentActionState,
    dispatch: (JSONObject) -> Unit,
) {
    if (action.kind.isBlank() && action.statusText.isBlank() && action.envelopeJson.isBlank()) return
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            action.statusText.ifBlank { paidRoutePaymentActionTitle(action.kind) },
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
        if (action.envelopeJson.isNotBlank()) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("Payment ready", modifier = Modifier.weight(1f), color = Muted)
                OutlinedButton(onClick = {
                    dispatch(NativeActions.sendPaidRoutePaymentEnvelope(action.envelopeJson))
                }) {
                    Text("Send payment")
                }
            }
        }
    }
}

@Composable
private fun PaidRouteOfferRow(
    offer: PaidRouteOfferState,
    dispatch: (JSONObject) -> Unit,
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(paidRouteOfferTitle(offer), fontWeight = FontWeight.SemiBold)
            Text(
                offer.statusText.ifBlank { offer.sellerNpub },
                color = Muted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.bodySmall,
            )
            val metricText = paidRouteMetricText(
                offer.qualityText.ifBlank {
                    paidRouteQualityText(offer.latencyMs, offer.jitterMs, offer.packetLossPpm)
                },
                offer.bandwidthText,
            )
            if (metricText.isNotBlank()) {
                Text(
                    metricText,
                    color = Muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
        Button(
            enabled = offer.key.isNotBlank(),
            onClick = { dispatch(NativeActions.buyPaidRouteOffer(offer.key)) },
        ) {
            Text("Connect")
        }
    }
}

@Composable
private fun PaidRouteSessionRow(
    session: PaidRouteSessionState,
    envelopeJson: String,
    dispatch: (JSONObject) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(paidRouteBuyerSessionTitle(session), fontWeight = FontWeight.SemiBold)
                Text(
                    paidRouteSessionDetail(session),
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
                if (session.locationText.isNotBlank()) {
                    Text(
                        session.locationText,
                        color = Muted,
                        style = MaterialTheme.typography.bodySmall,
                    )
                } else if (session.realizedExitIp.isNotBlank()) {
                    Text(
                        "${session.realizedExitIp} · ${paidRouteCountryClaimText(session)}",
                        color = Muted,
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                val metricText = paidRouteMetricText(
                    session.qualityText.ifBlank {
                        paidRouteQualityText(session.latencyMs, session.jitterMs, session.packetLossPpm)
                    },
                    session.bandwidthText,
                )
                if (metricText.isNotBlank()) {
                    Text(
                        metricText,
                        color = Muted,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                if (session.settlementText.isNotBlank()) {
                    Text(
                        session.settlementText,
                        color = Muted,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(
                    session.paidText.ifBlank { "${formatPaidRouteMsat(session.paidMsat)} paid" },
                    style = MaterialTheme.typography.bodySmall,
                )
                if (session.unpaidMsat > 0) {
                    Text(
                        session.unpaidText.ifBlank { "${formatPaidRouteMsat(session.unpaidMsat)} behind" },
                        color = Color(0xFF9A3412),
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = {
                    dispatch(NativeActions.selectPaidRouteSession(session.sessionId, connect = true))
                }) {
                    Text("Connect")
                }
                OutlinedButton(onClick = {
                    dispatch(NativeActions.probePaidRouteSession(session.sessionId, timeoutSecs = 5))
                }) {
                    Text("Probe")
                }
            }
            if (paidRouteSessionCanOpenChannel(session) ||
                paidRouteSessionCanSignPayment(session) ||
                paidRouteSessionCanCloseChannel(session) ||
                envelopeJson.isNotBlank()
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (paidRouteSessionCanOpenChannel(session)) {
                        OutlinedButton(onClick = {
                            dispatch(NativeActions.openPaidRouteChannelFromWallet(session.sessionId))
                        }) {
                            Text("Fund")
                        }
                    }
                    if (paidRouteSessionCanSignPayment(session)) {
                        OutlinedButton(onClick = {
                            dispatch(NativeActions.signPaidRoutePaymentEnvelopeFromWallet(session.sessionId))
                        }) {
                            Text("Pay")
                        }
                    }
                    if (paidRouteSessionCanCloseChannel(session)) {
                        OutlinedButton(onClick = {
                            dispatch(NativeActions.closePaidRouteChannelFromWallet(session.sessionId))
                        }) {
                            Text("Settle")
                        }
                    }
                    if (envelopeJson.isNotBlank()) {
                        OutlinedButton(onClick = {
                            dispatch(NativeActions.sendPaidRoutePaymentEnvelope(envelopeJson))
                        }) {
                            Text("Send")
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun PaidExitSellerStatusCard(state: AppState) {
    val seller = state.paidExitSeller
    AppCard {
        Text("Share My Internet", style = MaterialTheme.typography.titleMedium)
        Text(
            paidExitSellerStatusText(seller),
            color = Muted,
            style = MaterialTheme.typography.bodySmall,
        )
        if (seller.supported) {
            Text(
                paidExitSellerInternetText(seller),
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
            )
            if (seller.publicIpText.isNotBlank()) {
                Text(
                    "Public IP ${seller.publicIpText}",
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            Text(
                "Spendable wallet ${state.paidRouteMarket.wallet.totalBalanceText.ifBlank { formatPaidRouteMsat(state.paidRouteMarket.wallet.totalBalanceMsat) }}",
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
            )
            Text(
                "${seller.channelCreditTitleText.ifBlank { "Pending buyer credit" }} ${seller.channelCreditText.ifBlank { formatPaidRouteMsat(seller.channelCreditMsat) }}",
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
            )
            val creditHelp = seller.channelCreditHelpText.ifBlank {
                if (seller.channelCreditMsat > 0) "Collect to move it into wallet" else ""
            }
            if (creditHelp.isNotBlank()) {
                Text(
                    creditHelp,
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            if (state.paidRouteMarket.lastPaymentAction.kind.isNotBlank() || state.paidRouteMarket.lastPaymentAction.statusText.isNotBlank()) {
                Text(
                    "Payments ${state.paidRouteMarket.lastPaymentAction.statusText.ifBlank { paidRoutePaymentActionTitle(state.paidRouteMarket.lastPaymentAction.kind) }}",
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            Text(
                "${seller.countryCode.ifBlank { "Country unset" }} · ${paidRouteNetworkClassTitle(seller.networkClass)} · ${seller.priceText.ifBlank { paidRoutePriceText(seller.priceMsat, seller.perUnits, seller.meter, seller.perUnitsText) }}",
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
            )
            Text(
                "Free test ${seller.freeProbeText.ifBlank { paidRouteTrafficUnitText(seller.freeProbeUnits, seller.meter) }} · Grace ${seller.graceText.ifBlank { paidRouteTrafficUnitText(seller.graceUnits, seller.meter) }}",
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
            )
            if (seller.settlementText.isNotBlank()) {
                Text(
                    seller.settlementText,
                    color = Muted,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            val sellerSummary = listOf(
                "${seller.currentConnectionCount} connected",
                "${seller.pastConnectionCount} past",
                seller.totalTrafficText.ifBlank { "${formatBytes(seller.totalBillableBytes)} routed" },
                "${seller.totalPaidText.ifBlank { formatPaidRouteMsat(seller.totalPaidMsat) }} paid",
                "${seller.totalDueText.ifBlank { formatPaidRouteMsat(seller.totalDueMsat) }} due",
            ).joinToString(" · ")
            Text(
                sellerSummary,
                color = Muted,
                style = MaterialTheme.typography.bodySmall,
            )
            if (seller.totalUnpaidMsat > 0) {
                Text(
                    "${seller.totalUnpaidText.ifBlank { formatPaidRouteMsat(seller.totalUnpaidMsat) }} behind",
                    color = Color(0xFF9A3412),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            if (seller.sessions.isNotEmpty()) {
                Text("${seller.sessions.size} active customer${if (seller.sessions.size == 1) "" else "s"}", color = Muted)
            }
        }
    }
}

private fun paidRouteOfferTitle(offer: PaidRouteOfferState): String {
    val location = offer.countryCode.ifBlank { "Unknown country" }.uppercase()
    val network = paidRouteNetworkClassTitle(offer.networkClass)
    val price = offer.priceText.ifBlank {
        paidRoutePriceText(offer.priceMsat, offer.perUnits, offer.meter, offer.perUnitsText)
    }
    return "$location · $network · $price"
}

private fun paidRouteSessionDetail(session: PaidRouteSessionState): String {
    if (session.detailText.isNotBlank()) return session.detailText
    val access = paidRouteAccessTitle(session.accessState, session.lifecycleStatus.ifBlank { "session" })
    val units = when {
        session.bytes > 0 -> "${formatBytes(session.bytes)} used"
        session.packets > 0 -> "${session.packets} packets"
        else -> "${session.deliveredUnits} units"
    }
    return "$access, $units, ${formatPaidRouteMsat(session.amountDueMsat)} due"
}

private fun paidRouteBuyerSessionTitle(session: PaidRouteSessionState): String =
    when {
        session.titleText.isNotBlank() -> session.titleText
        session.allowRouting -> "Ready"
        session.unpaidMsat > 0 -> "Payment needed"
        !session.paymentChannelReady -> "Needs funds"
        else -> paidRoutePlainStatus(
            session.statusText.ifBlank { session.lifecycleStatus },
            "Session",
        )
    }

private fun paidRouteAccessTitle(value: String, fallback: String): String =
    when (value) {
        "paid" -> "Paid"
        "free_probe" -> "Free test"
        "grace" -> "Grace"
        "suspended" -> "Paused"
        else -> paidRoutePlainStatus(value, fallback)
    }

private fun paidRoutePlainStatus(value: String, fallback: String): String {
    val raw = value.ifBlank { fallback }
    return when (raw) {
        "opening" -> "Opening"
        "probing" -> "Checking quality"
        "active" -> "Active"
        "paused" -> "Paused"
        "closed" -> "Closed"
        "session" -> "Session"
        else -> raw.replace('_', ' ').replaceFirstChar { it.uppercase() }
    }
}

private fun paidRoutePaymentActionTitle(kind: String): String =
    when (kind) {
        "send" -> "Payment sent"
        "receive" -> "Payment received"
        "apply" -> "Payment applied"
        "create" -> "Payment ready"
        "open_channel" -> "Exit funded"
        "sign" -> "Payment ready"
        "close" -> "Channel settled"
        "stream" -> "Payments sent"
        "probe" -> "Quality checked"
        else -> kind.ifBlank { "Payment" }.replace('_', ' ').replaceFirstChar { it.uppercase() }
    }

private fun paidRouteWalletActionTitle(kind: String): String =
    when (kind) {
        "topup" -> "Invoice ready"
        "receive" -> "Token imported"
        "send" -> "Token ready"
        "withdraw" -> "Invoice paid"
        "refresh" -> "Wallet refreshed"
        "open_channel" -> "Exit funded"
        else -> kind.ifBlank { "Wallet updated" }.replace('_', ' ').replaceFirstChar { it.uppercase() }
    }

private fun paidRouteNetworkClassTitle(value: String): String =
    when (value) {
        "datacenter" -> "Datacenter"
        "residential" -> "Residential"
        "mobile" -> "Mobile"
        "satellite" -> "Satellite"
        "community_mesh" -> "Community mesh"
        "unknown", "" -> "Unknown"
        else -> value.replace('_', ' ').replaceFirstChar { it.uppercase() }
    }

private fun paidRouteCountryClaimText(session: PaidRouteSessionState): String =
    when (session.countryClaimStatus) {
        "match" -> "${session.observedCountryCode.ifBlank { session.claimedCountryCode }} matches claim"
        "mismatch" -> "${session.observedCountryCode.ifBlank { "Observed country" }} differs from ${session.claimedCountryCode}"
        else -> session.observedCountryCode.ifBlank { session.claimedCountryCode.ifBlank { "country unknown" } }
    }

private fun paidRouteQualityText(latencyMs: Int, jitterMs: Int, packetLossPpm: Int): String {
    if (latencyMs <= 0 && jitterMs <= 0 && packetLossPpm <= 0) return "Quality unmeasured"
    val loss = packetLossPpm.toDouble() / 10_000.0
    return "${latencyMs} ms · ${jitterMs} ms jitter · %.2f%% loss".format(loss)
}

private fun paidRouteMetricText(qualityText: String, bandwidthText: String): String =
    listOf(qualityText, bandwidthText)
        .map { it.trim() }
        .filter { it.isNotEmpty() && it != "Quality unmeasured" }
        .joinToString(" · ")

private fun paidExitSellerStatusText(seller: org.nostrvpn.app.core.PaidExitSellerState): String {
    val fallback =
        if (seller.supported) {
            "People can pay to use my internet"
        } else {
            "This platform cannot sell public internet access"
        }
    return seller.statusText.ifBlank { fallback }
        .replace("Paid exit selling", "Selling internet")
        .replace("paid exit selling", "selling internet")
}

private fun paidExitSellerInternetText(seller: org.nostrvpn.app.core.PaidExitSellerState): String =
    seller.internetText.ifBlank {
        when (seller.upstream) {
            "wireguard_exit", "wireguard", "wg", "upstream_vpn", "vpn" -> "My internet through WireGuard"
            else -> "My internet"
        }
    }

private fun paidRouteSessionCanOpenChannel(session: PaidRouteSessionState): Boolean =
    session.sessionId.isNotBlank() && !session.paymentChannelReady

private fun paidRouteSessionCanSignPayment(session: PaidRouteSessionState): Boolean =
    session.sessionId.isNotBlank() && session.paymentChannelReady && session.unpaidMsat > 0

private fun paidRouteSessionCanCloseChannel(session: PaidRouteSessionState): Boolean =
    session.sessionId.isNotBlank() &&
        session.paymentChannelReady &&
        session.lifecycleStatus !in setOf("closed", "expired")

private fun parsePositivePaidRouteAmount(value: String): Long? =
    value.trim().toLongOrNull()?.takeIf { it > 0 }

private fun optionalPaidRouteMintUrl(value: String): String? {
    val trimmed = value.trim()
    return if (trimmed.isEmpty()) null else trimmed
}

private fun formatPaidRouteMsat(msat: Long): String {
    if (msat <= 0) return "0 sat"
    val whole = msat / 1000
    val rem = msat % 1000
    return if (rem == 0L) {
        "$whole sat"
    } else {
        "%d.%03d sat".format(whole, rem)
    }
}

private fun formatBytes(bytes: Long): String {
    val units = listOf("B", "KB", "MB", "GB", "TB")
    var value = bytes.toDouble()
    var index = 0
    while (value >= 1024.0 && index < units.lastIndex) {
        value /= 1024.0
        index += 1
    }
    return when {
        index == 0 -> "$bytes B"
        kotlin.math.abs(value - kotlin.math.round(value)) < 0.05 -> "%.0f %s".format(value, units[index])
        else -> "%.1f %s".format(value, units[index])
    }
}

private fun paidRoutePriceText(
    priceMsat: Long,
    perUnits: Long,
    meter: String,
    perUnitsText: String = "",
): String =
    "${formatPaidRouteMsat(priceMsat)} / ${perUnitsText.ifBlank { paidRouteMeterUnitText(perUnits, meter) }}"

private fun paidRouteMeterUnitText(perUnits: Long, meter: String): String =
    when (meter) {
        "bytes" -> formatDecimalBytes(perUnits)
        "milliseconds", "millisecond", "ms" -> "${perUnits} ms"
        "packets", "packet" -> if (perUnits == 1L) "1 packet" else "${perUnits} packets"
        "" -> "${perUnits} units"
        else -> "${perUnits} $meter"
    }

private fun paidRouteTrafficUnitText(units: Long, meter: String): String =
    if (meter == "bytes") formatBytes(units) else paidRouteMeterUnitText(units, meter)

private fun formatDecimalBytes(bytes: Long): String {
    val units = listOf("B", "KB", "MB", "GB", "TB")
    var value = bytes.toDouble()
    var index = 0
    while (value >= 1000.0 && index < units.lastIndex) {
        value /= 1000.0
        index += 1
    }
    return when {
        index == 0 -> "$bytes B"
        kotlin.math.abs(value - kotlin.math.round(value)) < 0.05 -> "%.0f %s".format(value, units[index])
        else -> "%.1f %s".format(value, units[index])
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.settingsPage(
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
