import { getJson } from './api';
import { pairingOrigin } from './fleet';
import { authenticatedModels, isAutoModel } from './models';
import type { Model, ModelCatalog } from './types';

export type ChatAccountRoute = { accountId: string; peerNodeId?: string };
export type ChatFleetAccount = ChatAccountRoute & { key: string; label: string; host: string; models: Model[] };
export type ChatFleet = { accounts: ChatFleetAccount[]; models: Model[]; preferred: string; unreachable: number };

type Host = { origin: string; nodeId?: string };
export function chatPeerHosts(value: unknown, self: string): Host[] {
 if (!value || typeof value !== 'object' || !('peers' in value) || !Array.isArray(value.peers)) return [];
 const peers = new Map<string, Host>();
 for (const peer of value.peers) {
  if (!peer || typeof peer.nodeId !== 'string' || !/^[a-f0-9]{64}$/.test(peer.nodeId) || typeof peer.origin !== 'string') continue;
  const origin = pairingOrigin(peer.origin);
  if (origin && origin === peer.origin && origin !== self && !peers.has(origin)) peers.set(origin, { origin, nodeId: peer.nodeId });
 }
 return [...peers.values()];
}

export function chatAccountsForHost(host: Host, catalog: ModelCatalog, value: unknown): ChatFleetAccount[] {
 if (!Array.isArray(value) || !Array.isArray(catalog?.availableModels)) return [];
 const accounts = new Map<string, ChatFleetAccount>();
 for (const account of value) {
  if (account?.connected !== true || typeof account.provider !== 'string' || typeof account.label !== 'string' || typeof account.id !== 'string' || !/^[a-zA-Z0-9_][a-zA-Z0-9_-]{0,63}$/.test(account.id)) continue;
  const models = authenticatedModels(catalog.availableModels, [account]).filter((model) => !isAutoModel(model));
  if (!models.length) continue;
  const key = JSON.stringify([host.nodeId ?? '', account.id]);
  accounts.set(key, { accountId: account.id, ...(host.nodeId ? { peerNodeId: host.nodeId } : {}), key, label: account.label, host: new URL(host.origin).hostname, models });
 }
 return [...accounts.values()];
}

export async function loadChatFleet(self: string): Promise<ChatFleet> {
 const peers = await getJson<unknown>('/api/connections').then((value) => chatPeerHosts(value, self), () => []);
 const hosts = [{ origin: self }, ...peers];
 const results = await Promise.allSettled(hosts.map(async (host) => {
  const destination = host.nodeId ? host.origin : undefined;
  const [catalog, accounts] = await Promise.all([getJson<ModelCatalog>('/api/models', destination), getJson<unknown>('/api/chat-accounts', destination)]);
  return { accounts: chatAccountsForHost(host, catalog, accounts), preferred: catalog.currentModelId };
 }));
 const accounts = results.flatMap((result) => result.status === 'fulfilled' ? result.value.accounts : []);
 const models = [...new Map(accounts.flatMap((account) => account.models.map((model) => [model.modelId, model] as const))).values()];
 const local = results[0];
 const localPreference = local.status === 'fulfilled' ? local.value.preferred : null;
 const preferred = localPreference && models.some((model) => model.modelId === localPreference) ? localPreference : models[0]?.modelId ?? '';
 return { accounts, models, preferred, unreachable: results.filter((result) => result.status === 'rejected').length };
}
