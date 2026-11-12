# General chat peer sync

Configure sync on each node with its own private JSON file and pass it as `postudio-node serve --chat-peer-config <path>`. Use a local file with mode `0600` on Unix. Never commit configs or put real URLs, bearer tokens, or private keys in this repository.

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
