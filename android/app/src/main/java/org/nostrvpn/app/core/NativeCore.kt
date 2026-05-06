package org.nostrvpn.app.core

internal object NativeCore {
    init {
        System.loadLibrary("nostr_vpn_app_core")
    }

    external fun appNew(dataDir: String, appVersion: String): Long
    external fun appFree(handle: Long)
    external fun stateJson(handle: Long): String
    external fun refreshJson(handle: Long): String
    external fun dispatchJson(handle: Long, actionJson: String): String
    external fun qrMatrixJson(text: String): String
    external fun decodeQrImageJson(path: String): String
}
