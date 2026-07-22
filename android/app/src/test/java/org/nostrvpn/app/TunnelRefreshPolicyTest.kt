package org.nostrvpn.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TunnelRefreshPolicyTest {
    @Test
    fun qrAndManualApprovalActionsRestartTheRunningTunnel() {
        assertTrue(TunnelRefreshPolicy.requiresTunnelRefresh("import_join_request"))
        assertTrue(TunnelRefreshPolicy.requiresTunnelRefresh("accept_join_request"))
        assertTrue(TunnelRefreshPolicy.requiresTunnelRefresh("manual_add_network"))
        assertTrue(TunnelRefreshPolicy.requiresTunnelRefresh("add_participant"))
    }

    @Test
    fun rosterAndTunnelSettingsRestartButUiOnlyActionsDoNot() {
        assertTrue(TunnelRefreshPolicy.requiresTunnelRefresh("set_participant_alias"))
        assertTrue(
            TunnelRefreshPolicy.requiresTunnelRefresh(
                "update_settings",
                setOf("exitDnsMode"),
            ),
        )
        assertFalse(TunnelRefreshPolicy.requiresTunnelRefresh("tick"))
        assertFalse(
            TunnelRefreshPolicy.requiresTunnelRefresh(
                "update_settings",
                setOf("fiatCurrency"),
            ),
        )
    }
    @Test
    fun explicitDisconnectAlwaysStopsTheAndroidVpnService() {
        assertEquals(
            TunnelServiceCommand.DISCONNECT,
            TunnelServiceCommandPolicy.commandAfterAction(
                actionType = "disconnect_vpn",
                wasEnabled = false,
                isEnabled = false,
                requiresRefresh = false,
            ),
        )
        assertEquals(
            TunnelServiceCommand.DISCONNECT,
            TunnelServiceCommandPolicy.commandAfterAction(
                actionType = "disconnect_vpn",
                wasEnabled = true,
                isEnabled = true,
                requiresRefresh = false,
            ),
        )
    }

    @Test
    fun tunnelTransitionsAndRefreshesSelectTheExpectedServiceCommand() {
        assertEquals(
            TunnelServiceCommand.CONNECT,
            TunnelServiceCommandPolicy.commandAfterAction("connect_vpn", false, true, false),
        )
        assertEquals(
            TunnelServiceCommand.CONNECT,
            TunnelServiceCommandPolicy.commandAfterAction("update_settings", true, true, true),
        )
        assertEquals(
            TunnelServiceCommand.NONE,
            TunnelServiceCommandPolicy.commandAfterAction("tick", true, true, false),
        )
    }
}
