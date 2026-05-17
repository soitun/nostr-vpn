# Umbrel packaging

This directory contains:

- `docker-compose.yml`: template for the real Umbrel app, using host networking
  and `/dev/net/tun`
- `docker-compose.local.yml`: local-only Compose file that builds the same image
  in an ordinary bridged Docker network for safer UI and API testing
- `umbrel-app.yml`: app metadata template

## Local validation

From the repo root:

```sh
docker compose -f umbrel/docker-compose.local.yml up --build
```

Then open [http://localhost:38080](http://localhost:38080) and verify:

```sh
curl http://localhost:38080/api/health
curl -X POST http://localhost:38080/api/tick
```

The image builds the responsive Svelte control panel from
`web/control-panel` and serves it from `/usr/share/nostr-vpn/web`.

## Release bundle

Umbrel app submissions need a pinned remote image reference, not a local build
tag. Generate a submission-ready app directory with:

```sh
node scripts/umbrel-release.mjs \
  --image-ref ghcr.io/example/nostr-vpn-umbrel:v0.3.4@sha256:... \
  --output-dir dist/umbrel-v0.3.4
```

Or build and push the multi-arch image first:

```sh
node scripts/umbrel-release.mjs \
  --push \
  --image-repo ghcr.io/example/nostr-vpn-umbrel
```

That writes a ready-to-submit app folder with a pinned `docker-compose.yml`.

## Current limitation

The Umbrel container can manage the mesh tunnel and routes, but host split-DNS
integration is not wired up yet. MagicDNS aliases are therefore not installed on
the Umbrel host itself in this package.
