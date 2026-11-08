# call.danendz.com

A 1-on-1 video call with text chat, tuned for calling between mainland China and Uzbekistan.

- Direct WebRTC P2P first (~200 ms RTT China ↔ Uzbekistan). The self-hosted coturn in Hong Kong provides STUN (Google's is blocked in China) and a TURN relay (UDP, TCP and TLS, ~400 ms) as fallback when P2P fails. A relayed call keeps retrying direct (15 s, 30 s, 60 s, then every 2 min). Nothing depends on Google services.
- Video is 720p capped at 2 Mbps; under congestion it drops resolution rather than turning blocky. Audio has network priority.
- A single Node service handles WebSocket signaling, the in-memory chat and the React frontend.
- Opening `/` creates a random room `/r/<id>`. Share the link; each room holds at most 2 people.
- The Stats button shows RTT, loss, bitrate and whether the call is direct or relayed. "Force TURN relay" compares the two paths.
- The call recovers from network switches: the WebSocket resumes its seat and ICE restarts.

## Develop

```sh
npm install
cp .env.example .env   # fill in the keys you have
npm run dev        # server on :8080, Vite on :5173 (open http://localhost:5173)
```

Without `TURN_HOST`, calls only work between devices that can reach each other directly, such as two tabs or the same LAN.

## Build / run

```sh
npm run build && npm start   # serves dist/web + /ws on $PORT (default 8080)
```

| Env | Meaning |
|---|---|
| `PORT` | HTTP port (Railway sets it) |
| `TURN_HOST` | e.g. `turn-hk.danendz.com` |
| `TURN_SECRET` | coturn `static-auth-secret` |
| `TURN_PORT` / `TURNS_PORT` | defaults `3478` / `5349` |
| `GROQ_API_KEY` | Whisper speech-to-chat and the free translation models |
| `LLM_BASE_URL` / `LLM_API_KEY` / `LLM_MODEL` | DeepSeek, translates when Groq's quota runs out |

Deployment (Railway via `.railway/railway.ts`, coturn on Alibaba Cloud Hong Kong, Cloudflare DNS) is covered in [infra/README.md](infra/README.md).
