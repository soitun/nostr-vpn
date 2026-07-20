# Nostr VPN on StartOS

Open the web UI after the service starts. Create a private network, then use
each new device's signed join-request QR or link to approve phones, laptops,
or other Nostr VPN devices.

The package runs the Nostr VPN daemon and web control panel in the same StartOS
service. Data is stored in the `main` volume and included in StartOS backups.
