# Deploying

Two pieces:

| Piece | Where | Domain |
|---|---|---|
| App (static frontend + WebSocket signaling/chat) | Railway, **Southeast Asia (Singapore)** region | `call.danendz.com` |
| STUN + fallback TURN relay (coturn) | Existing Vultr Singapore VPS | `turn.danendz.com` |

Both live in Singapore because it has good routes to mainland China and sits roughly on the way to Uzbekistan.

## 1. Cloudflare DNS

Set **both records to "DNS only" (grey cloud)**. Cloudflare's proxy is unreliable from mainland China and can't carry TURN traffic at all.

- `call` → CNAME to the target Railway shows when you add the custom domain
- `turn` → A record → Vultr public IPv4

## 2. coturn on the Vultr VPS

```sh
sudo apt update && sudo apt install -y coturn certbot
sudo mkdir -p /etc/coturn/certs /var/log/turnserver && sudo chown turnserver: /var/log/turnserver
```

**Certificate.** VLESS probably holds 443, but certbot's HTTP-01 challenge only needs port 80:

```sh
sudo certbot certonly --standalone -d turn.danendz.com
```

If port 80 is also taken, use the DNS challenge instead (`python3-certbot-dns-cloudflare` with a Cloudflare API token).

coturn runs as `turnserver` and can't read `/etc/letsencrypt`, so copy the cert on every renewal:

```sh
sudo tee /etc/letsencrypt/renewal-hooks/deploy/coturn.sh >/dev/null <<'EOF'
#!/bin/sh
install -o turnserver -g turnserver -m 600 /etc/letsencrypt/live/turn.danendz.com/fullchain.pem /etc/coturn/certs/fullchain.pem
install -o turnserver -g turnserver -m 600 /etc/letsencrypt/live/turn.danendz.com/privkey.pem /etc/coturn/certs/privkey.pem
systemctl restart coturn
EOF
sudo chmod +x /etc/letsencrypt/renewal-hooks/deploy/coturn.sh
sudo /etc/letsencrypt/renewal-hooks/deploy/coturn.sh
```

**Config.** Copy `infra/coturn/turnserver.conf` to `/etc/turnserver.conf` and fill in `external-ip` and `static-auth-secret` (`openssl rand -hex 32`). Then:

```sh
sudo sed -i 's/^#\?TURNSERVER_ENABLED=.*/TURNSERVER_ENABLED=1/' /etc/default/coturn
sudo systemctl enable --now coturn
sudo systemctl restart coturn && sudo tail -f /var/log/turnserver/turnserver.log
```

**Firewall.** Open these on ufw **and** in the Vultr firewall group if you use one:

```sh
sudo ufw allow 3478/udp && sudo ufw allow 3478/tcp
sudo ufw allow 5349/tcp
sudo ufw allow 49160:49200/udp
```

## 3. Railway

The project is defined in `.railway/railway.ts`: service `web`, Singapore region, build/start commands, health check and `TURN_HOST`.

```sh
railway config plan && railway config apply                        # sync the service definition
railway variable set --service web TURN_SECRET=<static-auth-secret>  # kept out of source
railway domain call.danendz.com --service web                       # IaC can't register custom domains
```

The service is connected to GitHub `Danendz/call-web` (branch `main`): every push to `main` deploys automatically. `railway up --service web --ci` still works for deploying an uncommitted working copy.

`railway domain` prints the CNAME target and a `_railway-verify.call` TXT record. Add both in Cloudflare as DNS only.

## 4. Verify

1. **TURN works.** Open a room with `?relay=1` on two devices, e.g. `https://call.danendz.com/r/test123?relay=1`. That forces relay-only ICE and opens the stats overlay. The call must connect and show `TURN relay`. If it hangs on "Connecting…", check the coturn log and the firewall.
2. **Reachable from China.** Before anything else, have her open `https://call.danendz.com` without a VPN. If the page or the WebSocket won't load, Railway's edge is the problem, not WebRTC.
3. **Real call.** Open Stats during the call:
   - `Direct P2P` means the call is peer-to-peer. `TURN relay (udp|tcp|tls)` means it goes through Singapore.
   - RTT under 300 ms is fine for talking. Loss over ~3% means video will stutter.
   - Tick **Force TURN relay** and compare RTT and loss. Keep whichever path is better.
4. **Network switch.** Toggle wifi ↔ mobile data mid-call. It should recover within a few seconds via ICE restart, without rejoining.

Calls normally go direct. coturn is still required: it's the only STUN server reachable from China (without it direct fails too), and it carries the call when direct P2P fails.

## If calls start failing from China

The VPS also runs VLESS. If the GFW flags that IP, TURN dies with it. The fix is a separate small VPS (Tokyo or Singapore) used only for TURN. Point `turn.danendz.com` at it; no app changes needed.
