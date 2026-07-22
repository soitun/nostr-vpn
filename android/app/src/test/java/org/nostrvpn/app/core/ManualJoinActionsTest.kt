package org.nostrvpn.app.core

import org.junit.Assert.assertEquals
import org.junit.Test

class ManualJoinActionsTest {
    @Test
    fun manualJoinCarriesTheAdminAndNetworkIdentifiers() {
        val action = NativeActions.manualAddNetwork(
            adminNpub = "npub1admin",
            meshNetworkId = "8d4f34f5425bc50e",
        )

        assertEquals("manual_add_network", action.getString("type"))
        assertEquals("npub1admin", action.getString("adminNpub"))
        assertEquals("8d4f34f5425bc50e", action.getString("meshNetworkId"))
    }

    @Test
    fun adminManualJoinCarriesTheJoiningDeviceAndName() {
        val action = NativeActions.addParticipant(
            networkId = "network-1",
            npub = "npub1joiner",
            alias = "Phone",
        )

        assertEquals("add_participant", action.getString("type"))
        assertEquals("network-1", action.getString("networkId"))
        assertEquals("npub1joiner", action.getString("npub"))
        assertEquals("Phone", action.getString("alias"))
    }
}
