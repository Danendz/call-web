# Deploying

Two pieces:

| Piece | Where | Domain |
|---|---|---|
| App (static frontend + WebSocket signaling/chat) | Railway, **Southeast Asia (Singapore)** region | `call.danendz.com` |
| STUN + fallback TURN relay (coturn) | Alibaba Cloud Simple Application Server, **Hong Kong** | `turn-hk.danendz.com` |

The relay is in Hong Kong because Alibaba's China-optimized BGP plan has clean routes to all three mainland carriers, and from outside China the traffic reaches Hong Kong over China Mobile International (~110 ms, no loss). The old Vultr Singapore VPS (`turn.danendz.com`) lost 10-30% of packets on the Singtel link into Vultr.

Pick the **General-purpose** plan (ISP `BGP`), not **International** (`BGP_NCO`, not optimized for mainland China). Use the Ubuntu OS image.

## 1. Cloudflare DNS

Set **every record to "DNS only" (grey cloud)**. Cloudflare's proxy is unreliable from mainland China and can't carry TURN traffic at all.

- `call` → CNAME to the target Railway shows when you add the custom domain
- `turn-hk` → A record → the server's public IPv4

## 2. coturn on the Hong Kong server

The default user is `admin` with passwordless sudo; add your SSH key through the console's Remote Connection.

```sh
sudo apt update && sudo apt install -y coturn certbot
sudo mkdir -p /etc/coturn/certs /var/log/turnserver && sudo chown turnserver: /var/log/turnserver
```

**Certificate.** certbot's HTTP-01 challenge needs port 80:

```sh
sudo certbot certonly --standalone -d turn-hk.danendz.com
```

coturn runs as `turnserver` and can't read `/etc/letsencrypt`, so copy the cert on every renewal:

```sh
sudo tee /etc/letsencrypt/renewal-hooks/deploy/coturn.sh >/dev/null <<'EOF'
#!/bin/sh
install -o turnserver -g turnserver -m 600 /etc/letsencrypt/live/turn-hk.danendz.com/fullchain.pem /etc/coturn/certs/fullchain.pem
install -o turnserver -g turnserver -m 600 /etc/letsencrypt/live/turn-hk.danendz.com/privkey.pem /etc/coturn/certs/privkey.pem
systemctl restart coturn
EOF
sudo chmod +x /etc/letsencrypt/renewal-hooks/deploy/coturn.sh
sudo /etc/letsencrypt/renewal-hooks/deploy/coturn.sh
```

**Config.** Copy `infra/coturn/turnserver.conf` to `/etc/turnserver.conf` and fill in the CHANGE_ME values. The server is behind NAT: the private IP is the one `ip -4 addr` shows on `eth0`, the public IP is the one in the console. `static-auth-secret` is `openssl rand -hex 32`, or the current `TURN_SECRET` when replacing a server. Then:

```sh
sudo sed -i 's/^#\?TURNSERVER_ENABLED=.*/TURNSERVER_ENABLED=1/' /etc/default/coturn
sudo systemctl enable --now coturn
sudo systemctl restart coturn && sudo tail -f /var/log/turnserver/turnserver.log
```

**Firewall.** Open these on ufw **and** in the instance's Firewall tab in the Alibaba console (source `0.0.0.0/0`; clients change networks all the time):

```sh
sudo ufw allow 22/tcp
sudo ufw allow 80/tcp                        # certbot
sudo ufw allow 3478/udp && sudo ufw allow 3478/tcp
sudo ufw allow 5349/tcp
sudo ufw allow 49160:49200/udp
sudo ufw enable
```

## 3. Railway

The project is defined in `.railway/railway.ts`: service `web`, Singapore region, build/start commands, health check and `TURN_HOST`.

```sh
railway config plan && railway config apply                        # sync the service definition
railway variable set --service web TURN_SECRET=<static-auth-secret>  # kept out of source
railway domain call.danendz.com --service web                       # IaC can't register custom domains
```

Changing `.railway/railway.ts` doesn't redeploy on its own (it isn't in `watchPatterns`): after the merge, run `railway config plan && railway config apply`.

The `releases` bucket holds native app releases, uploaded by CI and served at `/updates/*` by the web service, which gets the bucket's credentials as `RELEASES_*` variables from the IaC. CI needs the same values as GitHub secrets (`RELEASES_ENDPOINT`, `RELEASES_BUCKET`, `RELEASES_KEY_ID`, `RELEASES_SECRET`; `railway bucket credentials` shows them), plus `MACOS_CERT_P12` (base64 of the self-signed "Call Danya" code-signing certificate's .p12) and `MACOS_CERT_PASSWORD`. See `native/AGENTS.md` (Distribution).

The service is connected to GitHub `Danendz/call-web` (branch `main`): every push to `main` deploys automatically. `railway up --service web --ci` still works for deploying an uncommitted working copy.

`railway domain` prints the CNAME target and a `_railway-verify.call` TXT record. Add both in Cloudflare as DNS only.

## 4. Verify

1. **TURN works.** Open a room with `?relay=1` on two devices, e.g. `https://call.danendz.com/r/test123?relay=1`. That forces relay-only ICE and opens the stats overlay. The call must connect and show `TURN relay`. If it hangs on "Connecting…", check the coturn log and both firewalls.
2. **Reachable from China.** Before anything else, have someone in mainland China open `https://call.danendz.com` without a VPN. If the page or the WebSocket won't load, Railway's edge is the problem, not WebRTC.
3. **Real call.** Open Stats during the call:
   - `Direct P2P` means the call is peer-to-peer. `TURN relay (udp|tcp|tls)` means it goes through Hong Kong.
   - RTT under 300 ms is fine for talking. Loss over ~3% means video will stutter.
   - Tick **Force TURN relay** and compare RTT and loss. Keep whichever path is better.
4. **Network switch.** Toggle wifi ↔ mobile data mid-call. It should recover within a few seconds via ICE restart, without rejoining.

Calls normally go direct. coturn is still required: it's the only STUN server reachable from China (without it direct fails too), and it carries the call when direct P2P fails.

## If calls start failing from China

Check loss first: `ping -c 100 turn-hk.danendz.com` from both sides, during the Chinese evening peak (20:00-23:00 Beijing time). Keep this server for TURN only: running a VPN on it from China breaks Alibaba's terms, and if the GFW flags the IP, TURN dies with it. To move the relay, set up a new server as above, point a new DNS name at it, and change `TURN_HOST` in `.railway/railway.ts`.
