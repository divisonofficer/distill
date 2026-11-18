import { getJson } from './api';
import { configuredHosts } from './fleet';
import type { NodeInfo } from './types';

export interface ArtifactEntry { name: string; path: string; kind: 'directory' | 'file'; mime?: string }
export interface PortEntry { port: number; process?: string }

export async function artifactEntries(id: string, path: string, host: string): Promise<ArtifactEntry[]> {
 const query = new URLSearchParams();
 if (path) query.set('path', path);
 const suffix = query.size ? `?${query}` : '';
 const result = await getJson<{ entries: ArtifactEntry[] }>(`/api/sessions/${encodeURIComponent(id)}/artifacts${suffix}`, host);
 return result.entries;
}

export function configuredUntrustedOrigin(info: NodeInfo, trustedOrigin?: string): string | null {
 if (!info.untrustedOrigin) return null;
 try {
  const url = new URL(info.untrustedOrigin);
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.pathname !== '/' || url.search || url.hash) return null;
  const trustedOrigins = new Set([trustedOrigin, typeof location !== 'undefined' ? location.origin : undefined, ...configuredHosts().map((host) => host.origin)].filter((origin): origin is string => !!origin));
  if (url.origin !== info.untrustedOrigin.replace(/\/$/, '') || trustedOrigins.has(url.origin)) return null;
  return url.origin;
 } catch { return null; }
}

export function artifactUrl(origin: string, id: string, path: string): string {
 return `${origin}/f/${encodeURIComponent(id)}/${path.split('/').map(encodeURIComponent).join('/')}`;
}
