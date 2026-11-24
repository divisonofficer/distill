# General chat peer sync

## Native connection settings

Once both nodes are running and reachable through their trusted HTTPS origins, open **Settings → 연결 및 채팅 공유** on each node. Generate an invitation on one node, paste it into the other node, verify the displayed address and public-key fingerprint, and confirm sharing. Invitations expire after ten minutes and can be used once. Enrollment must originate from an explicitly allowed web origin on the node being configured; plain HTTP is accepted only on loopback for local testing.

Run the same Postudio version on both nodes. New title events include a signed causal revision so a later rename from either node wins over earlier edits; old snapshots without revisions remain readable. Pairing shares all existing and future general chats using the signed-event protocol. It also permits the paired node's browser origin through the API/WebSocket origin guard; the Tailscale owner identity check remains mandatory. Discovery never grants trust. Coding sessions remain on their owning host and can be opened through the paired host link; project files are not replicated.

Peer credentials are written atomically to `state-dir/managed-chat-peers.json`, with mode `0600` on Unix, and applied without restarting the node. Restart restores this configuration. Removing a peer revokes its sync credentials and dynamically granted origin access, but does not delete previously copied history or manually configured origins. If a pairing request fails after the other node accepted it, inspect both nodes and remove any one-sided connection before creating a new invitation.

Tailscale installation, HTTPS exposure, initial owner/allowed-origin configuration, and service startup remain deployment prerequisites. Pairing does not install or start a server on another device.

### General chat account routing

The chat composer combines models and connected account nicknames from the current node and directly paired nodes. It displays the credential host beside the account. `/api/chat-accounts` is owner/Origin guarded and returns only connected account IDs, providers, and labels, not emails, usage details, or credentials. Coding sessions and account administration remain host-local.

The chat's receiving node retains ownership of the message and claims its completion before sending a bounded copy of the selected branch's prompt to the credential host. The gateway uses the existing peer bearer credentials over TLS and only the configured peer identified by its node ID. The worker uses the selected Distill account with no cross-account or cross-host fallback. The receiving node persists the reply and replicates the signed chat events to peers. A lost completion response is ambiguous and is never automatically retried; this is not a distributed exactly-once protocol. Remote completions show a pending state rather than incremental text streaming. Both nodes and their Distill leaders must be upgraded together.

### WSL with Tailscale running on Windows

From WSL, check that the Windows executable can run `status --json`. Start `postudio-node` with `POSTUDIO_TAILSCALE_BIN='/mnt/c/Program Files/Tailscale/tailscale.exe'` (adjust to your installation). Discovery uses that executable. For pairing and synchronization, if normal name resolution fails for a `.ts.net` name, the node looks up the exact peer name and its Tailscale IPs in that local status output. HTTPS keeps the original name and certificate checks. This does not change browser DNS or Windows/WSL networking.

## Legacy configuration

Manual configurations remain supported with `postudio-node serve --chat-peer-config <path>`. The managed configuration becomes authoritative once it exists; the legacy file is used only when there is no managed configuration. Use a local file with mode `0600` on Unix. Never commit configs or put real URLs, bearer tokens, invitations, or private keys in this repository.

Each file has this shape; replace every placeholder locally:

```json
{
  "peers": [
    {
      "nodeId": "<peer SHA-256 public-key fingerprint, lowercase hex>",
      "publicKey": "<peer Ed25519 public key, URL-safe base64 without padding>",
      "incomingToken": "<local bearer secret, at least 32 printable ASCII characters>",
      "outgoingUrl": "https://<peer-host>/api/chat-sync",
      "outgoingToken": "<bearer secret peer accepts, at least 32 printable ASCII characters>"
    }
  ]
}
```

Obtain each node's public identity from its local `state-dir/chats/chat-store.json` `trusted_keys` entry (the node ID is the lowercase SHA-256 fingerprint of the decoded public key). To identify the local node's entry, match an event's `author` in that snapshot to `trusted_keys`; create a local chat/message first if snapshot has no events. Share only the node ID and public key. Never copy or share `chat-store.key`; it is the private signing seed. Confirm peer identity out of band before adding it. On the peer, set `incomingToken` to the same secret as this node's `outgoingToken`; set its `outgoingToken` to this node's `incomingToken`.

Peer URLs must use HTTPS. Plain HTTP is accepted only for loopback testing. Configure TLS at the peer-facing reverse proxy or use a trusted HTTPS endpoint; the node rejects redirects. Sync sends the full event snapshot each exchange, with 16 MiB maximum request and response bodies. Snapshots above that ceiling do not sync; there is no pagination or automatic upgrade path yet.

Sync validates signatures against locally configured peer keys; payloads cannot add trust. Concurrent offline edits remain separate branches. This is best-effort, single-process persistence, not distributed exactly-once delivery. Independent peers can retain branches or duplicate equivalent work.

Completion calls use a five-minute ACP timeout. A client disconnect does not cancel the node-owned operation: retry while it runs returns `completion_pending`, and a successful result is persisted for retry reuse. An ACP timeout or connection loss leaves that target blocked with `completion_outcome_unknown` for the node process lifetime, because provider completion may have happened without a received result. A definitive provider error can be retried. Unknown state is in memory only; after node restart it is lost, so retry can repeat an inference that completed remotely before shutdown. No distributed exactly-once guarantee exists.
