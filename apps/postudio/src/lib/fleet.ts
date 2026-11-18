export type FleetHost = { origin: string; priority: number; online: boolean };
const KEY = 'postudio.fleet.hosts.v1';
const safeOrigin = (value: string): string | null => {
 try {
  const url = new URL(value);
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.pathname !== '/' || url.search || url.hash) return null;
  return url.origin;
 } catch { return null; }
};
export function configuredHosts(): FleetHost[] {
 try {
  const value: unknown = JSON.parse(localStorage.getItem(KEY) || '[]');
  if (!Array.isArray(value)) return [];
  return value.flatMap((item, index) => {
   if (!item || typeof item.origin !== 'string') return [];
   const origin = safeOrigin(item.origin);
   return origin ? [{ origin, priority: Number.isFinite(item.priority) ? item.priority : index, online: false }] : [];
  }).sort((a, b) => a.priority - b.priority);
 } catch { return []; }
}
export function parseHostList(text: string): string[] {
 return text.split(/[\n,]+/).map((host) => host.trim()).filter(Boolean);
}
export function saveHosts(origins: string[]): FleetHost[] {
 const hosts = [...new Set(origins.map(safeOrigin).filter((x): x is string => !!x))].map((origin, priority) => ({ origin, priority, online: false }));
 localStorage.setItem(KEY, JSON.stringify(hosts.map(({ origin, priority }) => ({ origin, priority }))));
 return hosts;
}
export async function probeHosts(timeoutMs = 2000): Promise<FleetHost[]> {
 const hosts = configuredHosts();
 return Promise.all(hosts.map(async (host) => {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try { const response = await fetch(`${host.origin}/api/node`, { signal: controller.signal, credentials: 'include' }); return { ...host, online: response.ok }; }
  catch { return { ...host, online: false }; }
  finally { clearTimeout(timer); }
 }));
}
export function orderedOrigins(): string[] { return configuredHosts().map((host) => host.origin); }
export function isValidConfiguredOrigin(origin: string): boolean {
 if (typeof location !== 'undefined' && origin === location.origin) return true;
 return configuredHosts().some((host) => host.origin === safeOrigin(origin));
}
export function sessionOrigin(search: string): string | null {
 const params = new URLSearchParams(search);
 const host = params.get('host');
 if (host === null) return null;
 const origin = safeOrigin(host);
 if (!origin || host !== origin || !isValidConfiguredOrigin(origin)) throw new Error('Invalid or unconfigured session host');
 return origin;
}
export function sessionHref(id: string, origin: string): string {
 return origin && origin !== (typeof location === 'undefined' ? '' : location.origin) ? `/s/${encodeURIComponent(id)}?host=${encodeURIComponent(origin)}` : `/s/${encodeURIComponent(id)}`;
}
