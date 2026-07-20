package org.nostrvpn.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DeviceQrParserTest {
    private val deviceId = "npub1" + "q".repeat(58)

    @Test
    fun parseScannedDeviceLinkQrAcceptsRawDeviceId() {
        assertEquals(
            ScannedDeviceLink(deviceId),
            parseScannedDeviceLinkQr("  $deviceId  "),
        )
    }

    @Test
    fun parseScannedDeviceLinkQrAcceptsNostrPrefixedDeviceId() {
        assertEquals(
            ScannedDeviceLink(deviceId),
            parseScannedDeviceLinkQr("nostr:$deviceId"),
        )
    }

    @Test
    fun parseScannedDeviceLinkQrAcceptsDeviceLinkUrl() {
        assertEquals(
            ScannedDeviceLink(deviceId, "Pixel"),
            parseScannedDeviceLinkQr("nvpn://device-link?device=$deviceId&name=Pixel"),
        )
    }

    @Test
    fun parseScannedDeviceLinkQrAcceptsJsonPayload() {
        assertEquals(
            ScannedDeviceLink(deviceId, "iPad"),
            parseScannedDeviceLinkQr("""{"deviceId":"$deviceId","nodeName":"iPad"}"""),
        )
    }

    @Test
    fun looksLikeJoinRequestQrOrLinkAcceptsSharedRequestUrls() {
        assertEquals(true, looksLikeJoinRequestQrOrLink("nvpn://join-request/abc123"))
        assertEquals(true, looksLikeJoinRequestQrOrLink("NVPN://JOIN-REQUEST/abc123"))
        assertEquals(false, looksLikeJoinRequestQrOrLink("nvpn://join-request/"))
        assertEquals(false, looksLikeJoinRequestQrOrLink("nvpn://join-request"))
        assertEquals(
            false,
            looksLikeJoinRequestQrOrLink(
                """{"networkId":"mesh","requesterNpub":"$deviceId","nodeName":"Phone"}""",
            ),
        )
    }

    @Test
    fun parseScannedDeviceLinkQrRejectsUnrelatedUrls() {
        assertNull(parseScannedDeviceLinkQr("https://example.invalid/not-a-device"))
    }
}
