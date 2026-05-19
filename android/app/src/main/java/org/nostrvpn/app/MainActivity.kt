package org.nostrvpn.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.os.Build
import android.os.Bundle
import android.net.VpnService
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import org.json.JSONObject
import org.nostrvpn.app.core.AppCoreClient
import org.nostrvpn.app.core.AppState
import org.nostrvpn.app.core.NativeActions
import org.nostrvpn.app.update.AndroidSelfUpdateManager
import org.nostrvpn.app.update.AndroidSelfUpdateState
import org.nostrvpn.app.vpn.NostrVpnService
import java.io.File

class MainActivity : ComponentActivity() {
    private var deepLink by mutableStateOf<String?>(null)
    private var debugAction by mutableStateOf<String?>(null)
    private lateinit var selfUpdateManager: AndroidSelfUpdateManager

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        deepLink = intent?.dataString
        debugAction = intent?.getStringExtra(EXTRA_DEBUG_ACTION)
        val dataDir = filesDir.resolve("app-core")
        seedMobileConfig(dataDir, androidDeviceName())
        // Pass empty so the FFI falls back to its own CARGO_PKG_VERSION
        // (workspace-inherited). Avoids drift between BuildConfig.VERSION_NAME
        // and the bundled nvpn binary's version.
        val core = AppCoreClient(dataDir.absolutePath, "")
        selfUpdateManager =
            AndroidSelfUpdateManager(
                context = this,
                scope = lifecycleScope,
                ioDispatcher = Dispatchers.IO,
            )
        selfUpdateManager.startAutomaticChecks()

        setContent {
            var state by remember { mutableStateOf(core.state()) }
            var androidError by remember { mutableStateOf("") }
            var pendingVpnStart by remember { mutableStateOf(false) }
            var pendingLocalNetworkAction by remember { mutableStateOf<JSONObject?>(null) }
            fun showAndroidError(message: String, fallback: String = "Android action failed") {
                androidError = message.trim().ifBlank { fallback }
            }
            fun showAndroidError(error: Throwable, fallback: String) {
                showAndroidError(error.message.orEmpty(), fallback)
            }
            fun applyUserActionState(nextState: AppState) {
                state = nextState
                androidError = ""
            }
            fun startVpnTunnel() {
                startVpnService(
                    Intent(this, NostrVpnService::class.java)
                        .setAction(NostrVpnService.ACTION_CONNECT)
                        .putExtra(
                            NostrVpnService.EXTRA_CONFIG_JSON,
                            core.mobileTunnelConfigJson(),
                        ),
                )
            }
            val vpnPermissionLauncher = rememberLauncherForActivityResult(
                ActivityResultContracts.StartActivityForResult(),
            ) { result ->
                if (result.resultCode == RESULT_OK && state.vpnEnabled) {
                    startVpnTunnel()
                } else if (pendingVpnStart && state.vpnEnabled) {
                    try {
                        applyUserActionState(core.dispatch(NativeActions.disconnectVpn()))
                    } catch (error: Exception) {
                        showAndroidError(error, "Android action failed")
                    }
                }
                pendingVpnStart = false
            }
            fun requestVpnTunnel() {
                val intent = VpnService.prepare(this)
                if (intent == null) {
                    startVpnTunnel()
                } else {
                    pendingVpnStart = true
                    vpnPermissionLauncher.launch(intent)
                }
            }
            fun dispatchNow(action: JSONObject) {
                val wasEnabled = state.vpnEnabled
                try {
                    applyUserActionState(core.dispatch(action))
                } catch (error: Exception) {
                    showAndroidError(error, "Android action failed")
                }
                if (!wasEnabled && state.vpnEnabled) {
                    requestVpnTunnel()
                } else if (wasEnabled && !state.vpnEnabled) {
                    startVpnService(
                        Intent(this, NostrVpnService::class.java)
                            .setAction(NostrVpnService.ACTION_DISCONNECT),
                    )
                }
            }
            fun requiredLocalNetworkPermission(): String? =
                when {
                    Build.VERSION.SDK_INT >= ANDROID_ACCESS_LOCAL_NETWORK_API -> ACCESS_LOCAL_NETWORK_PERMISSION
                    Build.VERSION.SDK_INT >= ANDROID_LOCAL_NETWORK_OPT_IN_API -> Manifest.permission.NEARBY_WIFI_DEVICES
                    else -> null
                }

            fun requiresLocalNetworkPermission(action: JSONObject): Boolean =
                when (action.optString("type")) {
                    "connect_vpn", "start_invite_broadcast", "start_nearby_discovery" -> true
                    else -> false
                }

            fun localNetworkPermissionMessage() =
                "Local network permission is needed for nearby device discovery."

            val localNetworkPermissionLauncher = rememberLauncherForActivityResult(
                ActivityResultContracts.RequestPermission(),
            ) { granted ->
                val action = pendingLocalNetworkAction
                pendingLocalNetworkAction = null
                if (granted && action != null) {
                    dispatchNow(action)
                } else {
                    showAndroidError(localNetworkPermissionMessage())
                }
            }
            val dispatch: (JSONObject) -> Unit = { action ->
                val permission = requiredLocalNetworkPermission()
                if (
                    permission != null &&
                    requiresLocalNetworkPermission(action) &&
                    checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED
                ) {
                    pendingLocalNetworkAction = action
                    runCatching { localNetworkPermissionLauncher.launch(permission) }
                        .onFailure {
                            pendingLocalNetworkAction = null
                            showAndroidError(localNetworkPermissionMessage())
                        }
                } else {
                    dispatchNow(action)
                }
            }
            val qrScanLauncher = rememberLauncherForActivityResult(
                ActivityResultContracts.TakePicturePreview(),
            ) { bitmap ->
                if (bitmap == null) {
                    return@rememberLauncherForActivityResult
                }
                try {
                    val qrFile = cacheDir.resolve("nvpn-invite-qr.png")
                    qrFile.outputStream().use { output ->
                        bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
                    }
                    val result = core.decodeQrImage(qrFile.absolutePath)
                    val error = result.optString("error")
                    val value = result.optString("value").trim()
                    if (error.isNotBlank()) {
                        showAndroidError(error, "QR scan failed")
                    } else if (value.isNotBlank()) {
                        dispatch(NativeActions.importInvite(value))
                    }
                } catch (error: Exception) {
                    showAndroidError(error, "QR scan failed")
                }
            }
            fun launchQrScan() {
                runCatching { qrScanLauncher.launch(null) }
                    .onFailure { error ->
                        if (error is SecurityException) {
                            showAndroidError("Camera permission is needed to scan invites.")
                        } else {
                            showAndroidError("Could not open the camera.")
                        }
                    }
            }
            val cameraPermissionLauncher = rememberLauncherForActivityResult(
                ActivityResultContracts.RequestPermission(),
            ) { granted ->
                if (granted) {
                    launchQrScan()
                } else {
                    showAndroidError("Camera permission is needed to scan invites.")
                }
            }
            fun requestQrScan() {
                androidError = ""
                if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
                    launchQrScan()
                } else {
                    runCatching { cameraPermissionLauncher.launch(Manifest.permission.CAMERA) }
                        .onFailure {
                            showAndroidError("Camera permission is needed to scan invites.")
                        }
                }
            }

            DisposableEffect(core) {
                onDispose { core.close() }
            }
            LaunchedEffect(core) {
                while (true) {
                    delay(2_000)
                    state = try {
                        val nextState = core.refresh()
                        if (nextState.error.isNotBlank()) {
                            androidError = ""
                        }
                        nextState
                    } catch (error: Exception) {
                        showAndroidError(error, "Android refresh failed")
                        state
                    }
                }
            }
            LaunchedEffect(deepLink, debugAction) {
                val invite = deepLink
                if (!invite.isNullOrBlank() && invite.startsWith("nvpn://", ignoreCase = true)) {
                    dispatch(NativeActions.importInvite(invite))
                    deepLink = null
                }
                when (val action = debugAction) {
                    DEBUG_ACTION_CONNECT -> {
                        if (BuildConfig.DEBUG) {
                            dispatch(NativeActions.connectVpn())
                        }
                        debugAction = null
                    }
                    DEBUG_ACTION_DISCONNECT -> {
                        if (BuildConfig.DEBUG) {
                            dispatch(NativeActions.disconnectVpn())
                        }
                        debugAction = null
                    }
                    null -> Unit
                    else -> {
                        debugAction = null
                    }
                }
            }

            val selfUpdateState by selfUpdateManager.state.collectAsState()
            val updateActions = remember {
                SelfUpdateActions(
                    check = { selfUpdateManager.check(manual = true) },
                    download = { selfUpdateManager.download() },
                    install = { selfUpdateManager.install(this@MainActivity) },
                    setAutoCheck = { enabled -> selfUpdateManager.setAutoCheckEnabled(enabled) },
                )
            }

            NostrVpnTheme {
                val displayState = if (state.error.isBlank() && androidError.isNotBlank()) {
                    state.copy(error = androidError)
                } else {
                    state
                }
                NostrVpnApp(
                    state = displayState,
                    qrJson = { invite -> core.qrMatrix(invite) },
                    scanQr = { requestQrScan() },
                    dispatch = dispatch,
                    selfUpdateState = selfUpdateState,
                    selfUpdateActions = updateActions,
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        deepLink = intent.dataString
        debugAction = intent.getStringExtra(EXTRA_DEBUG_ACTION)
    }

    private fun startVpnService(intent: Intent) {
        if (intent.action == NostrVpnService.ACTION_CONNECT && Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun seedMobileConfig(dataDir: File, deviceName: String) {
        val name = deviceName.trim()
        if (name.isEmpty()) return
        val config = dataDir.resolve("config.toml")
        if (config.exists()) return

        runCatching {
            dataDir.mkdirs()
            config.writeText("node_name = \"${tomlString(name)}\"\n")
        }
    }

    private fun androidDeviceName(): String {
        val manufacturer = Build.MANUFACTURER.orEmpty().trim()
        val model = Build.MODEL.orEmpty().trim()
        val prefix = titlecaseAscii(manufacturer)
        return when {
            model.isEmpty() -> prefix
            prefix.isEmpty() -> model
            model.startsWith(manufacturer, ignoreCase = true) -> model
            else -> "$prefix $model"
        }.ifBlank { "Android device" }
    }

    private fun titlecaseAscii(value: String): String =
        when {
            value.isEmpty() -> ""
            else -> value.take(1).uppercase() + value.drop(1)
        }

    private fun tomlString(value: String): String =
        value.replace("\\", "\\\\").replace("\"", "\\\"")

    companion object {
        const val EXTRA_DEBUG_ACTION = "org.nostrvpn.app.DEBUG_ACTION"
        const val DEBUG_ACTION_CONNECT = "connect"
        const val DEBUG_ACTION_DISCONNECT = "disconnect"
        private const val ANDROID_LOCAL_NETWORK_OPT_IN_API = 36
        private const val ANDROID_ACCESS_LOCAL_NETWORK_API = 37
        private const val ACCESS_LOCAL_NETWORK_PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"
    }
}
