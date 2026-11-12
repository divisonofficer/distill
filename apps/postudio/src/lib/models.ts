import type { Model } from './types';

const providerFamilies: Record<string, string[]> = {
Claude: ['fable', 'opus', 'sonnet', 'haiku'],
ChatGPT: ['astra', 'sol', 'terra', 'luna'],
'xAI': ['grok']
};

export function modelProvider(modelId: string): string {
const id = modelId.toLowerCase();
if (/^(?:claude\/|anthropic\/|claude[-_])/.test(id)) return 'Claude';
if (/^(?:chatgpt\/|openai\/|gpt[-_])/.test(id)) return 'ChatGPT';
if (/^(?:xai\/|grok[-_])/.test(id)) return 'xAI';
if (/^(?:google\/|gemini[-_])/.test(id)) return 'Google';
return '기타';
}

export function isAutoModel(model: Model): boolean {
return model.modelId.split('/').at(-1)?.toLowerCase() === 'auto' || model.name.trim().toLowerCase() === 'auto';
}

export function groupModels(models: Model[], search: string): Array<{ provider: string; models: Model[] }> {
const query = search.trim().toLocaleLowerCase();
const groups = new Map<string, Model[]>();
for (const model of models) {
if (query && !`${model.name} ${model.modelId}`.toLocaleLowerCase().includes(query)) continue;
const provider = modelProvider(model.modelId);
const entries = groups.get(provider) ?? [];
entries.push(model);
groups.set(provider, entries);
}
return [...groups].map(([provider, grouped]) => ({ provider, models: grouped }));
}

export function defaultModelGroups(models: Model[]): Array<{ provider: string; models: Model[] }> {
return groupModels(models.filter((model) => !isAutoModel(model)), '').map(({ provider, models: grouped }) => ({
provider,
models: representativeModels(grouped, provider)
}));
}

function representativeModels(models: Model[], provider: string): Model[] {
const families = providerFamilies[provider];
if (!families) return models;
const byFamily = new Map<string, Model[]>();
const other: Model[] = [];
for (const model of models) {
const family = families.find((candidate) => hasFamily(model, candidate));
if (!family) {
other.push(model);
continue;
}
const candidates = byFamily.get(family) ?? [];
candidates.push(model);
byFamily.set(family, candidates);
}
const representatives = families.flatMap((family) => {
const candidates = byFamily.get(family);
if (!candidates?.length) return [];
return [candidates.reduce((latest, model) => compareModelVersion(model, latest, family) > 0 ? model : latest)];
});
return [...representatives, ...other];
}

function hasFamily(model: Model, family: string): boolean {
const pattern = new RegExp(`(?:^|[^a-z])${family}(?=\\d|[^a-z]|$)`, 'i');
return pattern.test(`${model.modelId} ${model.name}`);
}

function compareModelVersion(left: Model, right: Model, family: string): number {
const leftVersion = modelVersion(left, family);
const rightVersion = modelVersion(right, family);
for (let index = 0; index < Math.max(leftVersion.parts.length, rightVersion.parts.length); index += 1) {
const difference = (leftVersion.parts[index] ?? 0) - (rightVersion.parts[index] ?? 0);
if (difference !== 0) return difference;
}
return (leftVersion.date ?? 0) - (rightVersion.date ?? 0);
}

function modelVersion(model: Model, family: string): { parts: number[]; date?: number } {
const pattern = new RegExp(`(?:^|[^a-z])${family}(?=\\d|[^a-z]|$)`, 'i');
for (const text of [model.modelId, model.name]) {
const match = pattern.exec(text);
if (!match) continue;
const afterFamily = text.slice(match.index + match[0].length);
const suffix = afterFamily.match(/^[\s._/-]*v?(\d+(?:[.-]\d+)*)/i)?.[1];
const beforeFamily = text.slice(0, match.index).match(/(\d+(?:[.-]\d+)*)[^0-9]*$/)?.[1];
const afterVersion = suffix ? parseVersion(suffix) : undefined;
const beforeVersion = beforeFamily ? parseVersion(beforeFamily) : undefined;
if (afterVersion?.parts.length) return afterVersion;
if (beforeVersion?.parts.length) return { ...beforeVersion, date: afterVersion?.date ?? beforeVersion.date };
if (afterVersion) return afterVersion;
if (beforeVersion) return beforeVersion;
}
return { parts: [] };
}

function parseVersion(value: string): { parts: number[]; date?: number } {
const parts = value.match(/\d+/g)?.map(Number) ?? [];
let date: number | undefined;
const last = value.match(/\d+/g)?.at(-1);
if (last?.length === 8 && parts.length > 0) {
date = parts.pop();
} else if (parts.length >= 3) {
const year = parts.at(-3) ?? 0;
const month = parts.at(-2) ?? 0;
const day = parts.at(-1) ?? 0;
if (year >= 2000 && month >= 1 && month <= 12 && day >= 1 && day <= 31) {
date = parts.splice(-3).reduce((result, part) => result * 100 + part, 0);
}
}
return { parts, date };
}

export function moveModelIndex(models: Model[], activeId: string, key: string): number | null {
if (models.length === 0) return null;
const current = models.findIndex((model) => model.modelId === activeId);
if (key === 'Home') return 0;
if (key === 'End') return models.length - 1;
if (key === 'ArrowDown') return (current + 1 + models.length) % models.length;
if (key === 'ArrowUp') return (current < 0 ? models.length - 1 : current - 1 + models.length) % models.length;
return current < 0 ? 0 : current;
}

export function moveMenuIndex(length: number, activeIndex: number, key: string): number | null {
if (length === 0) return null;
if (key === 'Home') return 0;
if (key === 'End') return length - 1;
if (key === 'ArrowDown') return (activeIndex + 1 + length) % length;
if (key === 'ArrowUp') return (activeIndex < 0 ? length - 1 : activeIndex - 1 + length) % length;
return activeIndex < 0 ? 0 : activeIndex;
}

export function modelKeyAction(key: string, open: boolean): 'move' | 'open' | 'select' | 'close' | undefined {
if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(key)) return open ? 'move' : 'open';
if (key === 'Enter') return open ? 'select' : undefined;
if (key === 'Escape' && open) return 'close';
return undefined;
}
